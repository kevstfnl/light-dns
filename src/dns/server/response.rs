use hickory_proto::{
    op::{Edns, Header, HeaderCounts, Message, Metadata, ResponseCode},
    serialize::binary::BinEncoder,
};
use hickory_server::{
    net::xfer::Protocol,
    server::{Request, ResponseHandler, ResponseInfo},
    zone_handler::MessageResponseBuilder,
};
use std::time::Duration;
use tokio::time::timeout;

const UDP_PAYLOAD: u16 = 1232;

pub(super) async fn send_response<R: ResponseHandler>(
    request: &Request,
    mut response: Message,
    mut response_handle: R,
) -> ResponseInfo {
    response.metadata.id = request.metadata.id;
    response.metadata.recursion_desired = request.metadata.recursion_desired;
    response.metadata.checking_disabled = request.metadata.checking_disabled;
    response.metadata.authentic_data &= request.metadata.authentic_data
        || request
            .edns
            .as_ref()
            .is_some_and(|edns| edns.flags().dnssec_ok);
    let edns = response_edns(request);
    if request.protocol() == Protocol::Udp
        && let Some(edns) = &edns
    {
        preserve_opt_on_truncation(request, &mut response, edns);
    }
    let response_message = MessageResponseBuilder::new(&request.queries, edns.as_ref()).build(
        response.metadata,
        &response.answers,
        &response.authorities,
        [],
        &response.additionals,
    );
    match timeout(
        Duration::from_secs(1),
        response_handle.send_response(response_message),
    )
    .await
    {
        Ok(Ok(info)) => info,
        _ => {
            tracing::debug!(client = %request.src(), "échec de l'envoi de la réponse DNS");
            ResponseInfo::from(Header {
                metadata: response.metadata,
                counts: HeaderCounts::default(),
            })
        }
    }
}

pub(super) fn response_edns(request: &Request) -> Option<Edns> {
    request.edns.as_ref().map(|client| {
        // OPT est propre à chaque saut : seules la taille et l'indication DNSSEC sont reprises.
        let mut edns = Edns::new();
        edns.set_max_payload(UDP_PAYLOAD.min(client.max_payload()))
            .set_dnssec_ok(client.flags().dnssec_ok);
        edns
    })
}

fn preserve_opt_on_truncation(request: &Request, response: &mut Message, edns: &Edns) {
    // Hickory 0.26 écrit OPT après les RR, sans lui réserver de place lors d'une
    // troncature. Une pré-sérialisation bornée permet de conserver l'OPT obligatoire
    // en renvoyant seulement la question et TC si la réponse ne tient pas.
    let mut bytes = Vec::with_capacity(edns.max_payload() as usize);
    let mut encoder = BinEncoder::new(&mut bytes);
    encoder.set_max_size(edns.max_payload());
    let result = MessageResponseBuilder::new(&request.queries, Some(edns))
        .build(
            response.metadata,
            &response.answers,
            &response.authorities,
            [],
            &response.additionals,
        )
        .destructive_emit(&mut encoder);
    if !matches!(result, Ok(info) if !info.truncation) {
        response.answers.clear();
        response.authorities.clear();
        response.additionals.clear();
        response.metadata.truncation = true;
        response.metadata.authentic_data = false;
    }
}

pub(super) fn error_response(request: &Request, code: ResponseCode) -> Message {
    let mut response = Message::response(request.metadata.id, request.metadata.op_code);
    response.metadata = Metadata::response_from_request(&request.metadata);
    response.metadata.recursion_available = true;
    response.metadata.response_code = code;
    response
}

#[cfg(all(test, target_os = "linux"))]
#[path = "../../../tests/performance/response_offline.rs"]
mod offline;
