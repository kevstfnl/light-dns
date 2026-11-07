mod policy;

use crate::support::{TestResult, dns::*};
use hickory_proto::{
    op::{Edns, Message, ResponseCode},
    rr::{DNSClass, RData, RecordType},
};
use light_dns::{
    config::CacheConfig,
    dns::server::{DnsServer, bind},
};
use std::{net::SocketAddr, sync::atomic::Ordering, time::Duration};
use tokio::{
    net::TcpStream,
    time::{sleep, timeout},
};

struct CachedServer {
    server: DnsServer,
    address: SocketAddr,
    upstream: Upstream,
}

impl CachedServer {
    async fn new(response: fn(&Message) -> Message, cache: CacheConfig) -> TestResult<Self> {
        let upstream = Upstream::start(response, response).await?;
        let mut config = test_config(vec![upstream.address])?;
        config.cache = cache;
        let (server, address) = bind(config).await?;
        Ok(Self {
            server,
            address,
            upstream,
        })
    }
    async fn ask(&self, request: &Message) -> TestResult<Message> {
        Ok(udp(self.address, request).await?.0)
    }
    fn hits(&self) -> usize {
        self.upstream.hits.load(Ordering::SeqCst)
    }
    async fn stop(mut self) -> TestResult {
        self.server.shutdown_gracefully().await?;
        Ok(())
    }
}

#[tokio::test]
async fn repeated_queries_share_cache_across_case_ids_and_transport() -> TestResult {
    let cached = CachedServer::new(answer, CacheConfig::default()).await?;
    let first = query("Mixed.TEST.")?;
    assert_eq!(cached.ask(&first).await?.answers.len(), 1);
    let mut next = query("mixed.test.")?;
    next.metadata.id = first.id.wrapping_add(1);
    let response = cached.ask(&next).await?;
    assert_eq!(response.id, next.id);
    assert_eq!(response.queries, next.queries);
    assert_eq!(response.answers[0].data, answer(&first).answers[0].data);
    assert!(response.answers[0].ttl <= 123);
    let mut tcp = TcpStream::connect(cached.address).await?;
    write_tcp(&mut tcp, &next).await?;
    assert_eq!(
        timeout(Duration::from_secs(3), read_tcp(&mut tcp))
            .await??
            .answers
            .len(),
        1
    );
    assert_eq!(cached.hits(), 1);
    assert_eq!(cached.server.cache_stats().await.entries, 1);
    cached.stop().await
}

#[tokio::test]
async fn cache_hit_is_available_without_recursion_but_a_miss_is_refused() -> TestResult {
    let cached = CachedServer::new(answer, CacheConfig::default()).await?;
    let mut request = query("cached.test.")?;
    cached.ask(&request).await?;
    request.metadata.recursion_desired = false;
    let response = cached.ask(&request).await?;
    assert_eq!(response.response_code, ResponseCode::NoError);
    assert!(!response.recursion_desired);
    let mut miss = query("miss.test.")?;
    miss.metadata.recursion_desired = false;
    assert_eq!(
        cached.ask(&miss).await?.response_code,
        ResponseCode::Refused
    );
    assert_eq!(cached.hits(), 1);
    cached.stop().await
}

#[tokio::test]
async fn ttl_decreases_without_refreshing_and_expired_data_is_forwarded_again() -> TestResult {
    fn short(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers[0].ttl = 2;
        response
    }
    let cached = CachedServer::new(short, CacheConfig::default()).await?;
    let request = query("short.test.")?;
    assert_eq!(cached.ask(&request).await?.answers[0].ttl, 2);
    sleep(Duration::from_millis(1100)).await;
    assert!(cached.ask(&request).await?.answers[0].ttl < 2);
    assert_eq!(cached.hits(), 1);
    sleep(Duration::from_millis(1000)).await;
    assert_eq!(cached.ask(&request).await?.answers[0].ttl, 2);
    assert_eq!(cached.hits(), 2);
    cached.stop().await
}

#[tokio::test]
async fn configured_ttl_cap_and_disabled_mode_are_observed() -> TestResult {
    let cached = CachedServer::new(
        answer,
        CacheConfig {
            max_ttl: Duration::from_secs(1),
            ..CacheConfig::default()
        },
    )
    .await?;
    let request = query("cap.test.")?;
    cached.ask(&request).await?;
    sleep(Duration::from_millis(1100)).await;
    cached.ask(&request).await?;
    assert_eq!(cached.hits(), 2);
    cached.stop().await?;
    let disabled = CachedServer::new(answer, CacheConfig::disabled()).await?;
    disabled.ask(&request).await?;
    disabled.ask(&request).await?;
    assert_eq!(disabled.hits(), 2);
    assert_eq!(disabled.server.cache_stats().await.entries, 0);
    disabled.stop().await
}

#[tokio::test]
async fn qtype_class_and_dnssec_contexts_do_not_mix() -> TestResult {
    let cached = CachedServer::new(answer, CacheConfig::default()).await?;
    let base = query("contexts.test.")?;
    cached.ask(&base).await?;
    cached.ask(&base).await?;
    let mut edns = base.clone();
    edns.edns = Some(Edns::new());
    cached.ask(&edns).await?;
    cached.ask(&edns).await?;
    let mut dnssec = edns.clone();
    dnssec.edns.as_mut().unwrap().set_dnssec_ok(true);
    cached.ask(&dnssec).await?;
    let mut cd = base.clone();
    cd.metadata.checking_disabled = true;
    cached.ask(&cd).await?;
    let mut ad = base.clone();
    ad.metadata.authentic_data = true;
    cached.ask(&ad).await?;
    assert_eq!(cached.hits(), 5);
    let mut aaaa = base.clone();
    aaaa.queries[0].query_type = RecordType::AAAA;
    cached.ask(&aaaa).await?;
    let mut chaos = base.clone();
    chaos.queries[0].query_class = DNSClass::CH;
    cached.ask(&chaos).await?;
    cached.ask(&chaos).await?;
    assert_eq!(cached.hits(), 8);
    cached.stop().await
}

#[tokio::test]
async fn concurrent_cache_hits_keep_each_clients_id() -> TestResult {
    let cached = CachedServer::new(answer, CacheConfig::default()).await?;
    let base = query("concurrent.test.")?;
    cached.ask(&base).await?;
    let mut tasks = tokio::task::JoinSet::new();
    for id in 0..32 {
        let mut request = base.clone();
        request.metadata.id = id;
        let address = cached.address;
        tasks.spawn(async move {
            let response = udp(address, &request).await?.0;
            assert_eq!(response.id, id);
            assert_eq!(response.answers.len(), 1);
            TestResult::Ok(())
        });
    }
    while let Some(result) = tasks.join_next().await {
        result??;
    }
    assert_eq!(cached.hits(), 1);
    cached.stop().await
}

#[tokio::test]
async fn entry_and_weight_limits_evict_under_pressure() -> TestResult {
    let cached = CachedServer::new(
        answer,
        CacheConfig {
            max_entries: 4,
            max_memory_bytes: 65536,
            ..CacheConfig::default()
        },
    )
    .await?;
    for i in 0..64 {
        cached.ask(&query(&format!("entry{i}.test."))?).await?;
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let stats = cached.server.cache_stats().await;
        if stats.entries <= 4 && stats.estimated_bytes <= 65536 {
            assert!(stats.entries > 0);
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        sleep(Duration::from_millis(10)).await;
    }
    cached.stop().await
}

#[tokio::test]
async fn rewrites_and_changed_filters_precede_a_warm_cache() -> TestResult {
    use crate::support::process::TestDirectory;
    use hickory_proto::rr::Name;
    use light_dns::config::FilteringConfig;
    let directory = TestDirectory::new()?;
    let upstream = Upstream::start(answer, answer).await?;
    let mut config = test_config(vec![upstream.address])?;
    config.cache = CacheConfig::default();
    let block = directory.0.join("block.txt");
    let allow = directory.0.join("allow.txt");
    tokio::fs::write(&block, "blocked.test\n").await?;
    tokio::fs::write(&allow, "").await?;
    config.filtering = FilteringConfig {
        sources: vec![],
        blocklists: vec![block.clone()],
        allowlist_files: vec![allow.clone()],
        cache_dir: directory.0.join("filters"),
        update_interval: Duration::from_secs(1),
        ..FilteringConfig::default()
    };
    config
        .rewrites
        .insert(Name::from_ascii("local.test.")?, "192.168.1.10".parse()?);
    let (mut server, address) = bind(config).await?;
    assert_eq!(
        udp(address, &query("local.test.")?).await?.0.answers[0].ttl,
        60
    );
    assert_eq!(
        udp(address, &query("blocked.test.")?)
            .await?
            .0
            .response_code,
        ResponseCode::NXDomain
    );
    assert_eq!(server.cache_stats().await.entries, 0);
    let request = query("public.test.")?;
    udp(address, &request).await?;
    assert_eq!(upstream.hits.load(Ordering::SeqCst), 1);
    tokio::fs::write(&block, "public.test\n").await?;
    for expected in [ResponseCode::NXDomain, ResponseCode::NoError] {
        if expected == ResponseCode::NoError {
            tokio::fs::write(&allow, "public.test\n").await?;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if udp(address, &request).await?.0.response_code == expected {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline);
            sleep(Duration::from_millis(10)).await;
        }
    }
    assert_eq!(upstream.hits.load(Ordering::SeqCst), 1);
    server.shutdown_gracefully().await?;
    Ok(())
}
