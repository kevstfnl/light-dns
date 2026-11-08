mod filtering;
mod forwarding;
mod measurement;

use crate::support::{TestResult, dns::*, process::TestDirectory};
use hickory_proto::{
    op::{Message, ResponseCode},
    rr::Name,
};
use light_dns::{
    config::{CacheConfig, FilteringConfig},
    dns::server::bind,
};
use measurement::{Usage, ticks_per_second};
use std::{fmt::Write, fs, sync::atomic::Ordering, time::Instant};
use tokio::net::UdpSocket;

const SAMPLES: usize = 5000;

async fn measure(
    name: &str,
    cached: bool,
    local: bool,
    blocked: bool,
    unique: bool,
    ticks: f64,
    csv: &mut String,
) -> TestResult {
    let directory = TestDirectory::new()?;
    let upstream = Upstream::start(answer, answer).await?;
    let mut config = test_config(vec![upstream.address])?;
    config.cache = CacheConfig {
        enabled: cached,
        ..CacheConfig::default()
    };
    if local {
        config
            .rewrites
            .insert(Name::from_ascii("bench.test.")?, "192.168.1.10".parse()?);
    }
    if blocked {
        let path = directory.0.join("block.txt");
        fs::write(&path, "bench.test\n")?;
        config.filtering = FilteringConfig {
            sources: vec![],
            blocklists: vec![path],
            ..FilteringConfig::default()
        };
    }
    let (mut server, address) = bind(config).await?;
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    socket.connect(address).await?;
    let mut buffer = [0; 4096];
    let request = query("bench.test.")?;
    // Échauffement de la pile réseau et du cache, exclu des mesures.
    for _ in 0..100 {
        exchange(&socket, &request, &mut buffer).await?;
    }
    let hits_before = upstream.hits.load(Ordering::SeqCst);
    let before = Usage::read()?;
    let start = Instant::now();
    let mut latencies = Vec::with_capacity(SAMPLES);
    for index in 0..SAMPLES {
        let mut request = if unique {
            query(&format!("entry{index}.test."))?
        } else {
            request.clone()
        };
        request.metadata.id = index as u16;
        let begin = Instant::now();
        let response = exchange(&socket, &request, &mut buffer).await?;
        latencies.push(begin.elapsed().as_secs_f64() * 1e6);
        assert_eq!(response.id, request.id);
        assert_eq!(
            response.response_code,
            if blocked {
                ResponseCode::NXDomain
            } else {
                ResponseCode::NoError
            }
        );
    }
    let seconds = start.elapsed().as_secs_f64();
    let after = Usage::read()?;
    let upstream_queries = upstream.hits.load(Ordering::SeqCst) - hits_before;
    let stats = server.cache_stats().await;
    assert_eq!(
        upstream_queries,
        if !local && !blocked && (!cached || unique) {
            SAMPLES
        } else {
            0
        }
    );
    assert!(stats.entries <= CacheConfig::default().max_entries);
    assert!(stats.estimated_bytes <= CacheConfig::default().max_memory_bytes);
    if cached && !local && !blocked {
        assert!(stats.entries > 0);
    }
    latencies.sort_by(f64::total_cmp);
    let cpu_ms = after.cpu_ticks.saturating_sub(before.cpu_ticks) as f64 * 1000.0 / ticks;
    writeln!(
        csv,
        "{name},{SAMPLES},{:.2},{:.2},{:.2},{:.2},{:.2},{cpu_ms:.1},{},{},{},{},{}",
        seconds * 1000.0,
        SAMPLES as f64 / seconds,
        latencies[SAMPLES / 2],
        latencies[SAMPLES * 95 / 100],
        latencies[SAMPLES * 99 / 100],
        before.rss_kib,
        after.rss_kib,
        stats.entries,
        stats.estimated_bytes,
        upstream_queries
    )?;
    server.shutdown_gracefully().await?;
    Ok(())
}

async fn exchange(socket: &UdpSocket, request: &Message, buffer: &mut [u8]) -> TestResult<Message> {
    socket.send(&request.to_vec()?).await?;
    let length =
        tokio::time::timeout(std::time::Duration::from_secs(3), socket.recv(buffer)).await??;
    Ok(Message::from_vec(&buffer[..length])?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "benchmark Linux local ; exécuter en --release avec --nocapture"]
async fn dns_cache() -> TestResult {
    if cfg!(debug_assertions) {
        return Err(std::io::Error::other("exécuter ce benchmark avec --release").into());
    }
    let ticks = ticks_per_second()?;
    let mut csv = String::from(
        "scenario,samples,elapsed_ms,qps,p50_us,p95_us,p99_us,cpu_ms,rss_before_kib,rss_after_kib,cache_entries,cache_weight_bytes,upstream_queries\n",
    );
    for (name, enabled, local, blocked, unique) in [
        ("forward_no_cache", false, false, false, false),
        ("warm_cache", true, false, false, false),
        ("local_rewrite", true, true, false, false),
        ("blocked", true, false, true, false),
        ("cache_pressure", true, false, false, true),
    ] {
        measure(name, enabled, local, blocked, unique, ticks, &mut csv).await?;
    }
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/benchmarks");
    fs::create_dir_all(&directory)?;
    fs::write(directory.join("cache.csv"), &csv)?;
    println!(
        "Linux {}, Tokio 2 workers, boucle locale, même processus pour client/serveur/upstream\n{csv}",
        std::env::consts::ARCH
    );
    Ok(())
}
