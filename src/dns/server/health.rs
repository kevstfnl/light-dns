use super::{runtime::Runtime, stats::Counters};
use crate::dns::forwarder::Forwarder;
use hickory_proto::{
    op::{Message, Query, ResponseCode},
    rr::{Name, RecordType},
};
use hickory_server::net::xfer::Protocol;
use std::{
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

pub(super) async fn check(
    mut address: SocketAddr,
    runtime: &Runtime,
    upstream: bool,
) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        if address.ip().is_unspecified() {
            address.set_ip(if address.is_ipv6() {
                IpAddr::V6(Ipv6Addr::LOCALHOST)
            } else {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            });
        }
        let local = Forwarder::new(
            vec![address],
            Duration::from_secs(1),
            Arc::new(Counters::default()),
            Arc::new(tokio::sync::Semaphore::new(256)),
        );
        let mut request = Message::query();
        request
            .queries
            .push(Query::query(Name::root(), RecordType::NS));
        // RD=0 garantit qu'une panne Internet ne rend pas le conteneur indisponible.
        for protocol in [Protocol::Udp, Protocol::Tcp] {
            let response = local
                .check_local(&request, protocol)
                .await
                .map_err(io::Error::other)?;
            if !matches!(
                response.response_code,
                ResponseCode::NoError | ResponseCode::NXDomain | ResponseCode::Refused
            ) {
                return Err(io::Error::other("réponse de santé DNS locale invalide"));
            }
        }
        if upstream {
            request.metadata.recursion_desired = true;
            let resolver = runtime.current.load_full();
            let response = resolver
                .forwarder
                .forward(request, Protocol::Udp)
                .await
                .map_err(io::Error::other)?;
            if response.response_code != ResponseCode::NoError
                || !response
                    .answers
                    .iter()
                    .any(|rr| rr.name.is_root() && rr.data.record_type() == RecordType::NS)
            {
                return Err(io::Error::other(
                    "résolution upstream de la racine impossible",
                ));
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "contrôle de santé expiré"))?
}
