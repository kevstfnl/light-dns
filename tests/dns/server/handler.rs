use crate::support::{TestResult, dns::*};
use hickory_proto::{
    op::{Edns, OpCode, ResponseCode},
    rr::{Name, RecordType},
};
use light_dns::{
    config::{Config, DnsConfig},
    dns::server::bind,
};
use std::{collections::HashMap, io, time::Duration};
use tokio::{net::UdpSocket, time::timeout};

#[tokio::test]
async fn rejects_unsupported_and_invalid_queries() -> TestResult {
    let silent = UdpSocket::bind("127.0.0.1:0").await?;
    let (mut server, address) = start(vec![silent.local_addr()?]).await?;
    let mut request = query("example.test.")?;
    request.metadata.op_code = OpCode::Update;
    assert_eq!(
        udp(address, &request).await?.0.response_code,
        ResponseCode::NotImp
    );
    request.metadata.op_code = OpCode::Query;
    request.queries.clear();
    assert_eq!(
        udp(address, &request).await?.0.response_code,
        ResponseCode::FormErr
    );
    request = query("example.test.")?;
    request.queries.push(request.queries[0].clone());
    assert_eq!(
        udp(address, &request).await?.0.response_code,
        ResponseCode::FormErr
    );
    request = query("example.test.")?;
    request.queries[0].query_type = RecordType::AXFR;
    assert_eq!(
        udp(address, &request).await?.0.response_code,
        ResponseCode::Refused
    );
    request = query("example.test.")?;
    request.metadata.recursion_desired = false;
    let response = udp(address, &request).await?.0;
    assert_eq!(response.response_code, ResponseCode::Refused);
    assert!(!response.recursion_desired);
    request = query("example.test.")?;
    let mut edns = Edns::new();
    edns.set_version(1);
    request.edns = Some(edns);
    let response = udp(address, &request).await?.0;
    assert_eq!(
        u16::from(response.response_code),
        u16::from(ResponseCode::BADVERS)
    );
    assert_eq!(response.edns.unwrap().version(), 0);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn local_names_and_zones_never_reach_upstreams() -> TestResult {
    let upstream = UdpSocket::bind("127.0.0.1:0").await?;
    let config = Config {
        cache: light_dns::config::CacheConfig::disabled(),
        filtering: light_dns::config::FilteringConfig::disabled(),
        dns: DnsConfig {
            listen: "127.0.0.1:0".parse()?,
            upstreams: vec![upstream.local_addr()?],
            upstream_timeout: Duration::from_millis(200),
        },
        rewrites: HashMap::from([
            (Name::from_ascii("*.lan.")?, "192.168.1.1".parse()?),
            (Name::from_ascii("private.test.")?, "192.168.1.2".parse()?),
        ]),
    };
    let (mut server, address) = bind(config).await?;
    for name in ["lan.", "unknown.LAN.", "deep.unknown.lan.", "private.test."] {
        assert_eq!(
            udp(address, &query(name)?).await?.0.response_code,
            ResponseCode::NoError
        );
    }
    let mut bytes = [0; 512];
    assert!(
        matches!(upstream.try_recv_from(&mut bytes), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
    );
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn handles_concurrent_clients_without_mixing_ids() -> TestResult {
    let upstream = Upstream::start(answer, answer).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let mut requests = tokio::task::JoinSet::new();
    for index in 0..32 {
        requests.spawn(async move {
            let mut request = query(&format!("client{index}.test."))?;
            request.metadata.id = index;
            let (response, _) = udp(address, &request).await?;
            assert_eq!(response.id, request.id);
            assert_eq!(response.queries, request.queries);
            assert_eq!(response.answers, answer(&request).answers);
            TestResult::Ok(())
        });
    }
    while let Some(result) = requests.join_next().await {
        result??;
    }
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn saturation_does_not_start_more_upstream_requests() -> TestResult {
    let silent = UdpSocket::bind("127.0.0.1:0").await?;
    let (mut server, address) = bind(Config {
        cache: light_dns::config::CacheConfig::disabled(),
        filtering: light_dns::config::FilteringConfig::disabled(),
        dns: DnsConfig {
            listen: "127.0.0.1:0".parse()?,
            upstreams: vec![silent.local_addr()?],
            upstream_timeout: Duration::from_secs(30),
        },
        rewrites: HashMap::from([(Name::from_ascii("local.lan.")?, "192.168.1.10".parse()?)]),
    })
    .await?;

    // Remplir la limite documentée par le réseau, sans accès au sémaphore interne.
    let mut clients = Vec::new();
    let mut bytes = [0; 512];
    timeout(Duration::from_secs(5), async {
        for index in 0..256 {
            let client = UdpSocket::bind("127.0.0.1:0").await?;
            let request = query(&format!("busy{index}.test."))?;
            client.send_to(&request.to_vec()?, address).await?;
            // Synchroniser chaque envoi évite de remplir le buffer UDP du faux upstream.
            silent.recv_from(&mut bytes).await?;
            clients.push(client);
        }
        TestResult::Ok(())
    })
    .await??;

    // Le reload ne doit pas doubler le quota pendant que l'ancien état répond encore.
    let mut replacement = test_config(vec![silent.local_addr()?])?;
    replacement.dns.upstream_timeout = Duration::from_secs(30);
    replacement
        .rewrites
        .insert(Name::from_ascii("local.lan.")?, "192.168.1.10".parse()?);
    server.control().reload(replacement).await?;
    let (response, _) = udp(address, &query("overflow.test.")?).await?;
    assert_eq!(response.response_code, ResponseCode::ServFail);
    let (local, _) = udp(address, &query("local.lan.")?).await?;
    assert_eq!(local.response_code, ResponseCode::NoError);
    assert_eq!(local.answers.len(), 1);
    assert!(
        matches!(silent.try_recv_from(&mut bytes), Err(error) if error.kind() == io::ErrorKind::WouldBlock)
    );
    timeout(Duration::from_secs(1), server.shutdown_gracefully()).await??;
    Ok(())
}
