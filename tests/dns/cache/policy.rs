use super::*;
use hickory_proto::rr::{
    Name, Record,
    rdata::{CNAME, NULL, SOA},
};

fn negative(request: &Message) -> Message {
    let mut response = answer(request);
    response.answers.clear();
    response.metadata.response_code = ResponseCode::NXDomain;
    response.authorities.push(Record::from_rdata(
        Name::from_ascii("test.").unwrap(),
        100,
        RData::SOA(SOA::new(
            Name::from_ascii("ns.test.").unwrap(),
            Name::from_ascii("hostmaster.test.").unwrap(),
            1,
            60,
            60,
            3600,
            2,
        )),
    ));
    response
}

#[tokio::test]
async fn negative_cache_uses_soa_minimum_and_ages_the_returned_soa() -> TestResult {
    for rcode in [ResponseCode::NXDomain, ResponseCode::NoError] {
        fn nodata(request: &Message) -> Message {
            let mut response = negative(request);
            response.metadata.response_code = ResponseCode::NoError;
            response
        }
        let cached = CachedServer::new(
            if rcode == ResponseCode::NXDomain {
                negative
            } else {
                nodata
            },
            CacheConfig::default(),
        )
        .await?;
        let request = query("negative.test.")?;
        cached.ask(&request).await?;
        let response = cached.ask(&request).await?;
        assert_eq!(response.response_code, rcode);
        assert!(response.answers.is_empty());
        assert!(response.authorities[0].ttl <= 2);
        assert_eq!(cached.hits(), 1);
        sleep(Duration::from_millis(2100)).await;
        cached.ask(&request).await?;
        assert_eq!(cached.hits(), 2);
        cached.stop().await?;
    }
    Ok(())
}

#[tokio::test]
async fn errors_zero_ttls_partial_answers_and_missing_or_unrelated_soa_are_not_cached() -> TestResult
{
    fn zero(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers[0].ttl = 0;
        response
    }
    fn huge_ttl(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers[0].ttl = u32::MAX;
        response
    }
    fn error(request: &Message) -> Message {
        let mut response = answer(request);
        response.metadata.response_code = ResponseCode::ServFail;
        response
    }
    fn missing_soa(request: &Message) -> Message {
        let mut response = negative(request);
        response.authorities.clear();
        response
    }
    fn unrelated(request: &Message) -> Message {
        let mut response = negative(request);
        response.authorities[0].name = Name::from_ascii("other.").unwrap();
        response
    }
    fn partial(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers[0].data = RData::CNAME(CNAME(Name::from_ascii("target.test.").unwrap()));
        response
    }
    fn signed(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers.push(Record::from_rdata(
            request.queries[0].name.clone(),
            60,
            RData::Unknown {
                code: RecordType::RRSIG,
                rdata: NULL::with(vec![0; 20]),
            },
        ));
        response
    }
    for create in [
        zero,
        huge_ttl,
        error,
        missing_soa,
        unrelated,
        partial,
        signed,
    ] {
        let cached = CachedServer::new(create, CacheConfig::default()).await?;
        let request = query("uncacheable.test.")?;
        cached.ask(&request).await?;
        cached.ask(&request).await?;
        assert_eq!(cached.hits(), 2);
        assert_eq!(cached.server.cache_stats().await.entries, 0);
        cached.stop().await?;
    }
    Ok(())
}

#[tokio::test]
async fn all_sections_age_and_the_shortest_ttl_expires_the_complete_message() -> TestResult {
    fn mixed(request: &Message) -> Message {
        let mut response = answer(request);
        response.metadata.authoritative = true;
        let mut extra = response.answers[0].clone();
        extra.ttl = 1;
        response.additionals.push(extra);
        response
    }
    let cached = CachedServer::new(mixed, CacheConfig::default()).await?;
    let request = query("mixed.test.")?;
    assert!(cached.ask(&request).await?.authoritative);
    let response = cached.ask(&request).await?;
    assert!(!response.authoritative);
    assert!(response.answers[0].ttl < 123 && response.additionals[0].ttl < 1);
    sleep(Duration::from_millis(1100)).await;
    cached.ask(&request).await?;
    assert_eq!(cached.hits(), 2);
    cached.stop().await
}

#[tokio::test]
async fn oversized_entries_are_served_but_not_retained() -> TestResult {
    fn large(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers[0].data = RData::NULL(NULL::with(vec![0; 65000]));
        response
    }
    let cached = CachedServer::new(
        large,
        CacheConfig {
            max_memory_bytes: 65536,
            ..CacheConfig::default()
        },
    )
    .await?;
    let mut request = query("large.test.")?;
    request.queries[0].query_type = RecordType::NULL;
    let mut tcp = TcpStream::connect(cached.address).await?;
    for _ in 0..2 {
        write_tcp(&mut tcp, &request).await?;
        assert_eq!(
            timeout(Duration::from_secs(3), read_tcp(&mut tcp))
                .await??
                .answers
                .len(),
            1
        );
    }
    assert_eq!(cached.hits(), 2);
    assert_eq!(cached.server.cache_stats().await.entries, 0);
    cached.stop().await
}

#[tokio::test]
async fn complete_cache_survives_udp_truncation_and_keeps_per_client_edns_limits() -> TestResult {
    fn truncated(request: &Message) -> Message {
        let mut response = answer(request);
        response.metadata.truncation = true;
        response.answers.clear();
        response
    }
    fn complete(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers = vec![response.answers[0].clone(); 100];
        response
    }
    let upstream = Upstream::start(truncated, complete).await?;
    let mut config = test_config(vec![upstream.address])?;
    config.cache = CacheConfig::default();
    let (mut server, address) = bind(config).await?;
    let mut stream = TcpStream::connect(address).await?;
    for payload in [None, Some(800), Some(4096), Some(512)] {
        let mut request = query("large.test.")?;
        request.edns = payload.map(|payload| {
            let mut edns = Edns::new();
            edns.set_max_payload(payload);
            edns
        });
        for _ in 0..2 {
            let (response, length) = udp(address, &request).await?;
            assert!(response.truncation);
            assert!(length <= payload.unwrap_or(512).min(1232) as usize);
            assert_eq!(response.edns.is_some(), payload.is_some());
        }
        write_tcp(&mut stream, &request).await?;
        let response = timeout(Duration::from_secs(3), read_tcp(&mut stream)).await??;
        assert!(!response.truncation);
        assert_eq!(response.answers.len(), 100);
        // Le premier échange de chaque contexte EDNS fait UDP puis repli TCP.
        assert_eq!(
            upstream.hits.load(Ordering::SeqCst),
            if payload.is_some() { 4 } else { 2 }
        );
    }
    assert_eq!(server.cache_stats().await.entries, 2);
    server.shutdown_gracefully().await?;
    Ok(())
}
