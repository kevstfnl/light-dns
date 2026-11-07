use crate::support::{TestResult, dns::*};
use hickory_proto::{
    op::{Edns, Message},
    rr::{
        Name, RData, Record, RecordType,
        rdata::{AAAA, CNAME},
    },
};
use std::{net::Ipv6Addr, time::Duration};
use tokio::{net::TcpStream, time::timeout};

const UDP_PAYLOAD: u16 = 1232;

fn large_answer(request: &Message) -> Message {
    let mut response = answer(request);
    response.answers = (0..100).map(|_| response.answers[0].clone()).collect();
    response
}

#[tokio::test]
async fn udp_size_limits_and_tcp_full_response() -> TestResult {
    let upstream = Upstream::start(large_answer, large_answer).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let mut request = query("large.test.")?;
    for payload in [None, Some(800), Some(4096)] {
        request.edns = payload.map(|payload| {
            let mut edns = Edns::new();
            edns.set_max_payload(payload);
            edns
        });
        let (response, length) = udp(address, &request).await?;
        assert!(response.truncation);
        assert!(length <= payload.unwrap_or(512).min(UDP_PAYLOAD) as usize);
        assert_eq!(response.edns.is_some(), payload.is_some());
    }
    let mut stream = TcpStream::connect(address).await?;
    write_tcp(&mut stream, &request).await?;
    let response = timeout(Duration::from_secs(3), read_tcp(&mut stream)).await??;
    assert!(!response.truncation);
    assert_eq!(response.answers.len(), 100);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn forwards_aaaa_cname_additionals_and_dnssec_flags() -> TestResult {
    fn ipv6_answer(request: &Message) -> Message {
        assert_eq!(request.queries[0].query_type(), RecordType::AAAA);
        assert!(request.checking_disabled);
        assert!(request.edns.as_ref().unwrap().flags().dnssec_ok);
        let mut response = answer(request);
        response.metadata.authentic_data = true;
        response.answers = vec![
            Record::from_rdata(
                request.queries[0].name().clone(),
                40,
                RData::CNAME(CNAME(Name::from_ascii("target.test.").unwrap())),
            ),
            Record::from_rdata(
                Name::from_ascii("target.test.").unwrap(),
                60,
                RData::AAAA(AAAA(Ipv6Addr::LOCALHOST)),
            ),
        ];
        response.additionals = vec![response.answers[1].clone()];
        response
    }
    let upstream = Upstream::start(ipv6_answer, ipv6_answer).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let mut request = query("alias.test.")?;
    request.queries[0].query_type = RecordType::AAAA;
    request.metadata.checking_disabled = true;
    let mut edns = Edns::new();
    edns.set_dnssec_ok(true);
    request.edns = Some(edns);
    let (response, _) = udp(address, &request).await?;
    assert_eq!(response.answers, ipv6_answer(&request).answers);
    assert_eq!(response.additionals, ipv6_answer(&request).additionals);
    assert!(response.checking_disabled && response.authentic_data);
    assert!(response.edns.as_ref().unwrap().flags().dnssec_ok);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn client_subnet_is_not_forwarded_over_udp_or_tcp() -> TestResult {
    use hickory_proto::rr::rdata::opt::{ClientSubnet, EdnsCode, EdnsOption};

    fn private_answer(request: &Message) -> Message {
        let edns = request.edns.as_ref().unwrap();
        assert!(edns.option(EdnsCode::Subnet).is_none());
        assert!(edns.flags().dnssec_ok);
        answer(request)
    }

    let upstream = Upstream::start(private_answer, private_answer).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let mut request = query("private.test.")?;
    let mut edns = Edns::new();
    edns.set_dnssec_ok(true);
    edns.options_mut()
        .insert(EdnsOption::Subnet(ClientSubnet::new(
            "192.0.2.0".parse()?,
            24,
            0,
        )));
    request.edns = Some(edns);
    // Valider la fixture évite un faux succès si l'option n'était pas encodée.
    assert!(
        Message::from_vec(&request.to_vec()?)?
            .edns
            .as_ref()
            .unwrap()
            .option(EdnsCode::Subnet)
            .is_some()
    );
    let udp_response = udp(address, &request).await?.0;
    let mut stream = TcpStream::connect(address).await?;
    write_tcp(&mut stream, &request).await?;
    let tcp_response = timeout(Duration::from_secs(3), read_tcp(&mut stream)).await??;
    for response in [udp_response, tcp_response] {
        assert_eq!(response.answers, answer(&request).answers);
        let edns = response.edns.as_ref().unwrap();
        assert!(edns.option(EdnsCode::Subnet).is_none());
        assert!(edns.flags().dnssec_ok);
    }
    server.shutdown_gracefully().await?;
    Ok(())
}
