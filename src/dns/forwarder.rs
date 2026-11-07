use std::{net::SocketAddr, time::Duration};

use hickory_proto::op::{DnsRequest, DnsResponse, Message, MessageType, ResponseCode};
use hickory_server::net::{
    NetError,
    runtime::TokioRuntimeProvider,
    tcp::TcpClientStream,
    udp::UdpClientStream,
    xfer::{DnsHandle, DnsRequestSender, FirstAnswer, Protocol},
};
use tokio::time::timeout;

pub(super) struct Forwarder {
    upstreams: Vec<SocketAddr>,
    timeout: Duration,
}

impl Forwarder {
    pub(super) fn new(upstreams: Vec<SocketAddr>, timeout: Duration) -> Self {
        Self { upstreams, timeout }
    }

    pub(super) async fn forward(
        &self,
        message: Message,
        protocol: Protocol,
    ) -> Result<Message, NetError> {
        for &upstream in &self.upstreams {
            let result = timeout(self.timeout, self.exchange(upstream, &message, protocol)).await;
            match result {
                Ok(Ok(response))
                    if !matches!(
                        response.response_code,
                        ResponseCode::ServFail | ResponseCode::Refused
                    ) =>
                {
                    return Ok(response);
                }
                Ok(Ok(response)) => {
                    tracing::debug!(%upstream, code = %response.response_code, "réponse upstream inutilisable")
                }
                Ok(Err(error)) => tracing::debug!(%upstream, %error, "upstream indisponible"),
                Err(error) => tracing::debug!(%upstream, %error, "timeout upstream"),
            }
        }
        Err(NetError::from(
            "aucun upstream n'a fourni de réponse utilisable",
        ))
    }

    async fn exchange(
        &self,
        upstream: SocketAddr,
        message: &Message,
        protocol: Protocol,
    ) -> Result<Message, NetError> {
        let response = if protocol == Protocol::Tcp {
            self.tcp(upstream, message).await?
        } else {
            let mut stream = UdpClientStream::builder(upstream, TokioRuntimeProvider::new())
                .with_timeout(Some(self.timeout))
                .with_max_retries(1)
                .build();
            let response = stream
                .send_message(DnsRequest::from(message.clone()))
                .first_answer()
                .await?;
            self.validate(&response, message)?;
            if response.truncation {
                self.tcp(upstream, message).await?
            } else {
                response
            }
        };
        self.validate(&response, message)?;
        if response.truncation {
            return Err(NetError::from("réponse TCP tronquée"));
        }
        Ok(response.into_message())
    }

    async fn tcp(&self, upstream: SocketAddr, message: &Message) -> Result<DnsResponse, NetError> {
        // Le provider possède les tâches Hickory et doit vivre jusqu'à la réponse.
        let provider = TokioRuntimeProvider::new();
        let exchange =
            TcpClientStream::exchange(upstream, None, self.timeout, Some(1), provider.clone())
                .await?;
        exchange
            .send(DnsRequest::from(message.clone()))
            .first_answer()
            .await
    }

    fn validate(&self, response: &Message, request: &Message) -> Result<(), NetError> {
        if response.message_type != MessageType::Response
            || response.op_code != request.op_code
            || response.queries != request.queries
            || response.signature.is_some()
        {
            return Err(NetError::from(
                "réponse upstream incompatible avec la requête",
            ));
        }
        Ok(())
    }
}
