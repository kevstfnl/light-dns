use super::response::{error_response, response_edns, send_response};
use super::{runtime::Runtime, stats::Counters};
use crate::dns::cache::CacheKey;
use hickory_proto::{
    op::{Message, OpCode, ResponseCode},
    rr::RecordType,
};
use hickory_server::{
    net::runtime::Time,
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
};
use std::sync::Arc;
use tokio::sync::Semaphore;

const MAX_IN_FLIGHT: usize = 256;

pub(super) struct Handler {
    runtime: Arc<Runtime>,
    permits: Semaphore,
}

impl Handler {
    pub(super) fn new(runtime: Arc<Runtime>) -> Self {
        Self {
            runtime,
            permits: Semaphore::new(MAX_IN_FLIGHT),
        }
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
        if resolver
            .filters
            .load()
            .is_blocked(info.query.original().name())
        {
            Counters::increment(&self.runtime.counters.blocked);
            return error_response(request, ResponseCode::NXDomain);
        }
        let key = CacheKey::new(info.query.original(), request);
        let cache = resolver.cache.load_full();
        if let Some(response) = cache.get(&key).await {
            Counters::increment(&self.runtime.counters.cache_hits);
            return response;
        }
        if !request.metadata.recursion_desired {
            return error_response(request, ResponseCode::Refused);
        }
        let Ok(_permit) = self.permits.try_acquire() else {
            return error_response(request, ResponseCode::ServFail);
        };
        let mut message = Message::query();
        message.queries.push(info.query.original().clone());
        message.metadata.recursion_desired = true;
        message.metadata.checking_disabled = request.metadata.checking_disabled;
        message.metadata.authentic_data = request.metadata.authentic_data;
        message.edns = response_edns(request);
        Counters::increment(&self.runtime.counters.forwarded);
        match resolver
            .forwarder
            .forward(message, request.protocol())
            .await
        {
            Ok(response) => {
                cache.insert(key, &response).await;
                response
            }
            Err(error) => {
                tracing::debug!(%error, "échec du forwarding DNS");
                error_response(request, ResponseCode::ServFail)
            }
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
