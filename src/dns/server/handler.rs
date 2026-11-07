use super::response::{error_response, response_edns, send_response};
use crate::{
    config::Config,
    dns::{forwarder::Forwarder, rewrites::Rewrites},
};
use hickory_proto::{
    op::{Message, OpCode, ResponseCode},
    rr::RecordType,
};
use hickory_server::{
    net::runtime::Time,
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
};
use tokio::sync::Semaphore;

const MAX_IN_FLIGHT: usize = 256;

pub(super) struct Handler {
    forwarder: Forwarder,
    rewrites: Rewrites,
    permits: Semaphore,
}

impl Handler {
    pub(super) fn new(config: Config) -> Self {
        Self {
            forwarder: Forwarder::new(config.dns.upstreams, config.dns.upstream_timeout),
            rewrites: Rewrites::new(config.rewrites),
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
        if let Some(response) = self
            .rewrites
            .resolve(info.query.original(), &request.metadata)
        {
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
        match self.forwarder.forward(message, request.protocol()).await {
            Ok(response) => response,
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
        send_response(request, self.resolve(request).await, response_handle).await
    }
}
