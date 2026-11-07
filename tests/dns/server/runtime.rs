use crate::support::{TestResult, dns::*};
use hickory_proto::{
    op::Message,
    rr::{Name, RData, rdata::A},
};
use light_dns::{config::CacheConfig, dns::server::bind};
use std::{net::Ipv4Addr, time::Duration};
use tokio::{net::UdpSocket, time::timeout};

#[tokio::test]
async fn in_flight_queries_cannot_repopulate_a_cleared_or_replaced_cache() -> TestResult {
    fn next(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers[0].data = RData::A(A(Ipv4Addr::new(192, 0, 2, 20)));
        response
    }
    for reload in [false, true] {
        let upstream = UdpSocket::bind("127.0.0.1:0").await?;
        let next_upstream = Upstream::start(next, next).await?;
        let mut config = test_config(vec![upstream.local_addr()?])?;
        config.cache = CacheConfig::default();
        config.dns.upstream_timeout = Duration::from_secs(3);
        let (mut server, address) = bind(config).await?;
        let request = query("slow.test.")?;
        let waiting = request.clone();
        let query_task = tokio::spawn(async move { udp(address, &waiting).await });
        let mut bytes = [0; 4096];
        let (length, peer) =
            timeout(Duration::from_secs(3), upstream.recv_from(&mut bytes)).await??;
        let received = Message::from_vec(&bytes[..length])?;
        if reload {
            let mut config = test_config(vec![next_upstream.address])?;
            config.cache = CacheConfig::default();
            server.control().reload(config).await?;
        } else {
            server.control().clear_cache().await;
        }
        upstream.send_to(&answer(&received).to_vec()?, peer).await?;
        assert_eq!(
            query_task.await??.0.answers[0].data,
            answer(&received).answers[0].data
        );
        assert_eq!(server.cache_stats().await.entries, 0);
        if reload {
            assert_eq!(
                udp(address, &request).await?.0.answers[0].data,
                next(&request).answers[0].data
            );
        } else {
            let mut config = test_config(vec![next_upstream.address])?;
            config.cache = CacheConfig::default();
            config
                .rewrites
                .insert(Name::from_ascii("slow.test.")?, "192.168.1.3".parse()?);
            server.control().reload(config).await?;
            assert_eq!(
                udp(address, &request).await?.0.answers[0].data.to_string(),
                "192.168.1.3"
            );
        }
        server.shutdown_gracefully().await?;
    }
    Ok(())
}
