use super::response::{error_response, response_edns, send_response};
use super::{runtime::Runtime, stats::Counters};
use crate::{dns::cache::CacheKey, filtering::Decision};
use hickory_proto::{
    op::{Message, OpCode, ResponseCode},
    rr::{DNSClass, RData, Record, RecordType},
};
use hickory_server::{
    net::runtime::Time,
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
};
use std::sync::Arc;

pub(super) struct Handler {
    runtime: Arc<Runtime>,
}

impl Handler {
    pub(super) fn new(runtime: Arc<Runtime>) -> Self {
        Self { runtime }
    }

    fn blocked(&self, request: &Request, cname: bool) -> Message {
        Counters::increment(&self.runtime.counters.blocked);
        if cname {
            Counters::increment(&self.runtime.counters.cname_blocked);
        }
        error_response(request, ResponseCode::NXDomain)
    }

    async fn resolve(&self, request: &Request) -> Message {
        if request.metadata.op_code != OpCode::Query {
            return error_response(request, ResponseCode::NotImp);
        }
        let Ok(info) = request.request_info() else {
            return error_response(request, ResponseCode::FormErr);
        };
        if request.version() != 0 {
            return error_response(request, ResponseCode::BADVERS);
        }
        if !request.answers.is_empty() || !request.authorities.is_empty() {
            return error_response(request, ResponseCode::FormErr);
        }
        if request.signature.is_some()
            || matches!(info.query.query_type(), RecordType::AXFR | RecordType::IXFR)
        {
            return error_response(request, ResponseCode::Refused);
        }
        let resolver = self.runtime.current.load_full();
        if let Some(response) = resolver
            .rewrites
            .resolve(info.query.original(), &request.metadata)
        {
            Counters::increment(&self.runtime.counters.local_answers);
            return response;
        }
        let mut deadline = None;
        let mut query = std::borrow::Cow::Borrowed(info.query.original());
        let mut aliases = Vec::new();
        let mut visited = Vec::new();
        loop {
            if !aliases.is_empty()
                && let Some(mut response) = resolver.rewrites.resolve(&query, &request.metadata)
            {
                Counters::increment(&self.runtime.counters.local_answers);
                response.answers.splice(0..0, aliases);
                response.metadata.authentic_data = false;
                return response;
            }
            let filters = resolver.filters.load_full();
            let mut allowed = match filters.decide(query.name(), query.query_type()) {
                Decision::Block => return self.blocked(request, !aliases.is_empty()),
                Decision::Rewrite { code, data } => {
                    if query.query_class() != DNSClass::IN {
                        return error_response(request, ResponseCode::Refused);
                    }
                    Counters::increment(&self.runtime.counters.rewritten);
                    let mut response = error_response(request, code);
                    if let Some(RData::CNAME(target)) = data.first() {
                        let record = Record::from_rdata(query.name().clone(), 60, data[0].clone());
                        if query.query_type() == RecordType::CNAME {
                            aliases.push(record);
                            response.answers = aliases;
                            return response;
                        }
                        if visited.len() == 16
                            || target.0 == *query.name()
                            || visited.contains(&target.0)
                        {
                            return error_response(request, ResponseCode::ServFail);
                        }
                        visited.push(query.name().clone());
                        aliases.push(record);
                        deadline.get_or_insert_with(|| {
                            tokio::time::Instant::now() + resolver.forwarder.budget()
                        });
                        query.to_mut().set_name(target.0.clone());
                        continue;
                    }
                    response.answers = aliases;
                    response.answers.extend(
                        data.into_iter()
                            .filter(|data| {
                                query.query_type() == RecordType::ANY
                                    || data.record_type() == query.query_type()
                            })
                            .map(|data| Record::from_rdata(query.name().clone(), 60, data)),
                    );
                    return response;
                }
                Decision::Pass => false,
                Decision::Allow => true,
            };
            let key = CacheKey::new(&query, request);
            let cache = resolver.cache.load_full();
            let cached = cache.get(&key).await;
            let from_cache = cached.is_some();
            let mut response = if let Some(response) = cached {
                Counters::increment(&self.runtime.counters.cache_hits);
                response
            } else {
                if !request.metadata.recursion_desired {
                    if aliases.is_empty() {
                        return error_response(request, ResponseCode::Refused);
                    }
                    let mut response = error_response(request, ResponseCode::NoError);
                    response.answers = aliases;
                    return response;
                }
                let mut message = Message::query();
                message.queries.push(query.as_ref().clone());
                message.metadata.recursion_desired = true;
                message.metadata.checking_disabled = request.metadata.checking_disabled;
                message.metadata.authentic_data = request.metadata.authentic_data;
                message.edns = response_edns(request);
                let result = if aliases.is_empty() {
                    resolver
                        .forwarder
                        .forward(message, request.protocol())
                        .await
                } else {
                    match tokio::time::timeout_at(
                        deadline.unwrap_or_else(tokio::time::Instant::now),
                        resolver.forwarder.forward(message, request.protocol()),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => return error_response(request, ResponseCode::ServFail),
                    }
                };
                match result {
                    Ok(response) => response,
                    Err(_) => return error_response(request, ResponseCode::ServFail),
                }
            };
            // Reprendre l'index actif après l'attente réseau ou la lecture du cache.
            let current_filters = resolver.filters.load();
            if !Arc::ptr_eq(&filters, &current_filters) {
                allowed = match current_filters.decide(query.name(), query.query_type()) {
                    Decision::Block => return self.blocked(request, !aliases.is_empty()),
                    Decision::Rewrite { .. } => continue,
                    Decision::Allow => true,
                    Decision::Pass => false,
                };
            }
            let filters = &current_filters;
            {
                match filters.blocks_cname(query.name(), &response, allowed) {
                    Ok(true) => return self.blocked(request, true),
                    Err(()) => return error_response(request, ResponseCode::ServFail),
                    Ok(false) => {}
                }
            }
            if aliases.is_empty() {
                if !from_cache {
                    cache.insert(key, &response).await;
                }
            } else {
                let raw = (!from_cache).then(|| response.clone());
                response.answers.splice(0..0, aliases);
                response.metadata.authentic_data = false;
                response.metadata.authoritative = false;
                match filters.decide(
                    info.query.original().name(),
                    info.query.original().query_type(),
                ) {
                    Decision::Block => return self.blocked(request, true),
                    Decision::Allow => {
                        if filters
                            .blocks_cname(info.query.original().name(), &response, true)
                            .is_err()
                        {
                            return error_response(request, ResponseCode::ServFail);
                        }
                    }
                    _ => match filters.blocks_cname(info.query.original().name(), &response, false)
                    {
                        Ok(true) => return self.blocked(request, true),
                        Err(()) => return error_response(request, ResponseCode::ServFail),
                        Ok(false) => {}
                    },
                }
                if let Some(raw) = raw {
                    cache.insert(key, &raw).await;
                }
            }
            return response;
        }
    }
}

#[async_trait::async_trait]
impl RequestHandler for Handler {
    async fn handle_request<R: ResponseHandler, T: Time>(
        &self,
        request: &Request,
        response_handle: R,
    ) -> ResponseInfo {
        Counters::increment(&self.runtime.counters.requests);
        let response = self.resolve(request).await;
        if !matches!(
            response.response_code,
            ResponseCode::NoError | ResponseCode::NXDomain
        ) {
            Counters::increment(&self.runtime.counters.errors);
        }
        send_response(request, response, response_handle).await
    }
}
