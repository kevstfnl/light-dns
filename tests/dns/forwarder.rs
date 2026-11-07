use crate::support::{TestResult, dns::*};
use hickory_proto::{
    op::{Message, MessageType, OpCode, Query, ResponseCode},
    rr::{Name, RData, Record, RecordType, rdata::SOA},
};
use light_dns::{
    config::{Config, DnsConfig},
    dns::server::bind,
};
use std::{collections::HashMap, time::Duration};
use tokio::{
    net::{TcpStream, UdpSocket},
    time::timeout,
};

#[tokio::test]
async fn udp_forwarding_preserves_question_id_records_and_ttl() -> TestResult {
    let upstream = Upstream::start(answer, answer).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let request = query("Example.TEST.")?;
    let (response, _) = udp(address, &request).await?;
    assert_eq!(response.id, request.id);
    assert_eq!(response.message_type, MessageType::Response);
    assert_eq!(response.queries, request.queries);
    assert_eq!(response.answers, answer(&request).answers);
    assert!(response.recursion_desired && response.recursion_available);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn tcp_supports_multiple_requests_per_connection() -> TestResult {
    fn udp_forbidden(_: &Message) -> Message {
        panic!("un client TCP doit utiliser l'upstream TCP")
    }
    let upstream = Upstream::start(udp_forbidden, answer).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let mut stream = TcpStream::connect(address).await?;
    for name in ["first.test.", "second.test."] {
        let request = query(name)?;
        write_tcp(&mut stream, &request).await?;
        let response = timeout(Duration::from_secs(3), read_tcp(&mut stream)).await??;
        assert_eq!(response.id, request.id);
        assert_eq!(response.answers, answer(&request).answers);
    }
    drop(stream);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn truncated_upstream_udp_retries_over_tcp() -> TestResult {
    fn truncated(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers.clear();
        response.metadata.truncation = true;
        response
    }
    let upstream = Upstream::start(truncated, answer).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let request = query("fallback.test.")?;
    let (response, _) = udp(address, &request).await?;
    assert!(!response.truncation);
    assert_eq!(response.answers, answer(&request).answers);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn timeout_and_servfail_try_the_next_upstream() -> TestResult {
    let silent = UdpSocket::bind("127.0.0.1:0").await?;
    fn failed(request: &Message) -> Message {
        let mut response = answer(request);
        response.metadata.response_code = ResponseCode::ServFail;
        response
    }
    let failed = Upstream::start(failed, failed).await?;
    let healthy = Upstream::start(answer, answer).await?;
    let (mut server, address) =
        start(vec![silent.local_addr()?, failed.address, healthy.address]).await?;
    let (response, _) = udp(address, &query("failover.test.")?).await?;
    assert_eq!(response.response_code, ResponseCode::NoError);
    assert_eq!(response.answers.len(), 1);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn unavailable_upstreams_return_servfail_without_stopping_service() -> TestResult {
    let silent = UdpSocket::bind("127.0.0.1:0").await?;
    let (mut server, address) = start(vec![silent.local_addr()?]).await?;
    for _ in 0..2 {
        let request = query("timeout.test.")?;
        let (response, _) = udp(address, &request).await?;
        assert_eq!(response.id, request.id);
        assert_eq!(response.response_code, ResponseCode::ServFail);
        assert!(response.answers.is_empty());
    }
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn nxdomain_preserves_authority_soa() -> TestResult {
    fn negative(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers.clear();
        response.metadata.response_code = ResponseCode::NXDomain;
        response.authorities.push(Record::from_rdata(
            Name::from_ascii("test.").unwrap(),
            300,
            RData::SOA(SOA::new(
                Name::from_ascii("ns.test.").unwrap(),
                Name::from_ascii("hostmaster.test.").unwrap(),
                1,
                60,
                60,
                3600,
                30,
            )),
        ));
        response
    }
    let upstream = Upstream::start(negative, negative).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let request = query("absent.test.")?;
    let (response, _) = udp(address, &request).await?;
    assert_eq!(response.response_code, ResponseCode::NXDomain);
    assert_eq!(response.authorities, negative(&request).authorities);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn rejects_mismatched_or_invalid_upstream_responses() -> TestResult {
    fn wrong_id(request: &Message) -> Message {
        let mut response = answer(request);
        response.metadata.id = request.id.wrapping_add(1);
        response.answers[0].ttl = 999;
        response
    }
    fn wrong_question(request: &Message) -> Message {
        let mut response = answer(request);
        response.queries[0] =
            Query::query(Name::from_ascii("forged.test.").unwrap(), RecordType::A);
        response.answers[0].ttl = 999;
        response
    }
    fn wrong_type(request: &Message) -> Message {
        let mut response = answer(request);
        response.metadata.message_type = MessageType::Query;
        response.answers[0].ttl = 999;
        response
    }
    fn wrong_opcode(request: &Message) -> Message {
        let mut response = answer(request);
        response.metadata.op_code = OpCode::Status;
        response.answers[0].ttl = 999;
        response
    }
    let healthy = Upstream::start(answer, answer).await?;
    for bad in [wrong_id, wrong_question, wrong_type, wrong_opcode] {
        let bad = Upstream::start(bad, bad).await?;
        let (mut server, address) = start(vec![bad.address, healthy.address]).await?;
        let request = query("secure.test.")?;
        let (response, _) = udp(address, &request).await?;
        assert_eq!(response.message_type, MessageType::Response);
        assert_eq!(response.op_code, OpCode::Query);
        assert_eq!(response.answers, answer(&request).answers);
        server.shutdown_gracefully().await?;
    }
    Ok(())
}

#[tokio::test]
async fn ipv6_udp_and_tcp_forwarding() -> TestResult {
    let upstream = Upstream::start_on("[::1]:0", answer, answer).await?;
    let (mut server, address) = bind(Config {
        dns: DnsConfig {
            listen: "[::1]:0".parse()?,
            upstreams: vec![upstream.address],
            upstream_timeout: Duration::from_secs(1),
        },
        rewrites: HashMap::new(),
    })
    .await?;
    let request = query("ipv6.test.")?;
    assert_eq!(
        udp(address, &request).await?.0.answers,
        answer(&request).answers
    );
    let mut stream = TcpStream::connect(address).await?;
    write_tcp(&mut stream, &request).await?;
    assert_eq!(
        timeout(Duration::from_secs(3), read_tcp(&mut stream))
            .await??
            .answers,
        answer(&request).answers
    );
    server.shutdown_gracefully().await?;
    Ok(())
}
