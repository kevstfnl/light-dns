use super::*;
use crate::support::http::HttpServer;
use light_dns::config::FilterSource;
use std::sync::atomic::Ordering;

async fn start_remote(
    directory: &TestDirectory,
    url: &str,
    interval: u64,
    max: usize,
) -> TestResult<(DnsServer, SocketAddr, Upstream)> {
    let (mut config, upstream) = TestServer::config(directory).await?;
    config.filtering.sources = vec![FilterSource {
        name: "test".into(),
        url: url.into(),
        allow_dnsrewrite: false,
    }];
    config.filtering.update_interval = Duration::from_secs(interval);
    config.filtering.download_timeout = Duration::from_secs(1);
    config.filtering.max_list_bytes = max;
    let (server, address) = bind(config).await?;
    Ok((server, address, upstream))
}

async fn wait_for_hit(http: &HttpServer, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while http.hits.load(Ordering::SeqCst) < count {
        assert!(
            Instant::now() < deadline,
            "la liste n'a pas été téléchargée"
        );
        sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn first_start_serves_local_rules_then_activates_the_remote_list() -> TestResult {
    let directory = TestDirectory::new()?;
    let http = HttpServer::gated("remote.test\n").await?;
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    let local = directory.0.join("block.txt");
    tokio::fs::write(&local, "local.test\n").await?;
    config.filtering.blocklists = vec![local];
    config.filtering.sources = vec![FilterSource {
        name: "test".into(),
        url: http.url.clone(),
        allow_dnsrewrite: false,
    }];
    let (mut server, address) = bind(config).await?;
    wait_for_hit(&http, 1).await;
    assert_eq!(
        udp(address, &query("local.test.")?).await?.0.response_code,
        ResponseCode::NXDomain
    );
    assert_eq!(
        udp(address, &query("remote.test.")?).await?.0.response_code,
        ResponseCode::NoError
    );
    http.release();
    wait_for_code(address, "remote.test.", ResponseCode::NXDomain).await?;
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn unavailable_first_download_preserves_local_filtering_and_dns_service() -> TestResult {
    let directory = TestDirectory::new()?;
    let http = HttpServer::new("unavailable").await?;
    http.set(503, "unavailable", true, 0);
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    let local = directory.0.join("block.txt");
    tokio::fs::write(&local, "local.test\n").await?;
    config.filtering.blocklists = vec![local];
    config.filtering.sources = vec![FilterSource {
        name: "test".into(),
        url: http.url.clone(),
        allow_dnsrewrite: false,
    }];
    config.filtering.update_interval = Duration::from_secs(1);
    let (mut server, address) = bind(config).await?;
    wait_for_hit(&http, 2).await;
    assert_eq!(
        udp(address, &query("local.test.")?).await?.0.response_code,
        ResponseCode::NXDomain
    );
    assert_eq!(
        udp(address, &query("allowed.test.")?)
            .await?
            .0
            .answers
            .len(),
        1
    );
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn invalid_disk_cache_is_ignored_until_a_valid_download_replaces_it() -> TestResult {
    let directory = TestDirectory::new()?;
    let http = HttpServer::gated("valid.test\n").await?;
    let cache = directory.0.join("cache");
    tokio::fs::create_dir(&cache).await?;
    let path = cache.join("test.txt");
    let invalid = format!(
        "! light-dns source {}\n<html>invalid.test</html>\n",
        http.url
    );
    tokio::fs::write(&path, &invalid).await?;
    let (mut server, address, _upstream) = start_remote(&directory, &http.url, 86400, 8192).await?;
    wait_for_hit(&http, 1).await;
    assert_eq!(
        udp(address, &query("valid.test.")?).await?.0.response_code,
        ResponseCode::NoError
    );
    assert_eq!(tokio::fs::read_to_string(&path).await?, invalid);
    http.release();
    wait_for_code(address, "valid.test.", ResponseCode::NXDomain).await?;
    assert!(
        tokio::fs::read_to_string(&path)
            .await?
            .ends_with("valid.test\n")
    );
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn failed_cache_write_retries_within_five_minutes_without_redownloading() -> TestResult {
    let directory = TestDirectory::new()?;
    let http = HttpServer::gated("new.test\n").await?;
    let cache = directory.0.join("cache");
    tokio::fs::create_dir(&cache).await?;
    let path = cache.join("test.txt");
    let previous = format!("! light-dns source {}\nold.test\n", http.url);
    tokio::fs::write(&path, &previous).await?;
    std::fs::File::open(&path)?.set_modified(std::time::UNIX_EPOCH)?;
    let (mut server, address, upstream) = start_remote(&directory, &http.url, 86400, 8192).await?;
    wait_for_hit(&http, 1).await;
    assert_eq!(
        udp(address, &query("old.test.")?).await?.0.response_code,
        ResponseCode::NXDomain
    );
    // A regular file at cache_dir makes saving fail even when tests run as root.
    let backup = directory.0.join("backup");
    tokio::fs::rename(&cache, &backup).await?;
    tokio::fs::write(&cache, "unavailable").await?;
    http.release();
    wait_for_code(address, "new.test.", ResponseCode::NXDomain).await?;
    assert_eq!(
        tokio::fs::read_to_string(backup.join("test.txt")).await?,
        previous
    );
    tokio::fs::remove_file(&cache).await?;
    tokio::fs::rename(&backup, &cache).await?;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    tokio::time::resume();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !tokio::fs::read_to_string(&path)
        .await?
        .ends_with("new.test\n")
    {
        assert!(
            Instant::now() < deadline,
            "cache non sauvegardé après 301 secondes"
        );
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(http.hits.load(Ordering::SeqCst), 1);
    server.shutdown_gracefully().await?;
    drop(upstream);
    let url = http.url.clone();
    drop(http);
    let (mut server, address, _upstream) = start_remote(&directory, &url, 86400, 8192).await?;
    assert_eq!(
        udp(address, &query("new.test.")?).await?.0.response_code,
        ResponseCode::NXDomain
    );
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn downloads_updates_and_uses_disk_cache_offline() -> TestResult {
    let directory = TestDirectory::new()?;
    let http = HttpServer::new("||old.test^\n").await?;
    let (mut server, address, _upstream) = start_remote(&directory, &http.url, 1, 8192).await?;
    wait_for_code(address, "old.test.", ResponseCode::NXDomain).await?;
    http.set(200, "||new.test^\n", true, 0);
    wait_for_code(address, "new.test.", ResponseCode::NXDomain).await?;
    wait_for_code(address, "old.test.", ResponseCode::NoError).await?;
    server.shutdown_gracefully().await?;
    let url = http.url.clone();
    drop(http);
    let (mut server, address, _upstream) = start_remote(&directory, &url, 86400, 8192).await?;
    assert_eq!(
        udp(address, &query("new.test.")?).await?.0.response_code,
        ResponseCode::NXDomain
    );
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn failed_empty_and_html_downloads_preserve_the_last_valid_cache() -> TestResult {
    let directory = TestDirectory::new()?;
    let http = HttpServer::new("keep.test\n").await?;
    let (mut server, address, _upstream) = start_remote(&directory, &http.url, 86400, 8192).await?;
    wait_for_code(address, "keep.test.", ResponseCode::NXDomain).await?;
    server.shutdown_gracefully().await?;
    let cache = directory.0.join("cache/test.txt");
    let valid = tokio::fs::read(&cache).await?;
    for (status, body) in [
        (500, "error"),
        (200, ""),
        (200, "<!doctype html>\n<html>\naccidental.test\n</html>"),
    ] {
        http.set(status, body, true, 0);
        std::fs::File::open(&cache)?.set_modified(std::time::UNIX_EPOCH)?;
        let hits = http.hits.load(Ordering::SeqCst);
        let (mut server, address, _upstream) = start_remote(&directory, &http.url, 1, 8192).await?;
        wait_for_hit(&http, hits + 2).await;
        // Une réponse invalide ne peut remplacer le cache, même après la fin de sa lecture.
        wait_for_code(address, "keep.test.", ResponseCode::NXDomain).await?;
        assert_eq!(tokio::fs::read(&cache).await?, valid);
        server.shutdown_gracefully().await?;
    }
    Ok(())
}

#[tokio::test]
async fn bounded_downloads_reject_content_length_and_streamed_overflow() -> TestResult {
    for length in [true, false] {
        let directory = TestDirectory::new()?;
        let http = HttpServer::new("").await?;
        http.set(200, &format!("bad.test\n{}", "x".repeat(2048)), length, 0);
        let (mut server, address, _upstream) = start_remote(&directory, &http.url, 1, 1024).await?;
        wait_for_hit(&http, 2).await;
        assert_eq!(
            udp(address, &query("bad.test.")?).await?.0.response_code,
            ResponseCode::NoError
        );
        assert!(!directory.0.join("cache/test.txt").exists());
        server.shutdown_gracefully().await?;
    }
    Ok(())
}

#[tokio::test]
async fn download_timeout_does_not_delay_dns_service() -> TestResult {
    let directory = TestDirectory::new()?;
    let http = HttpServer::new("delayed.test\n").await?;
    http.set(200, "delayed.test\n", true, 2000);
    let (mut server, address, _upstream) = start_remote(&directory, &http.url, 1, 8192).await?;
    wait_for_hit(&http, 2).await;
    assert_eq!(
        udp(address, &query("delayed.test.")?)
            .await?
            .0
            .answers
            .len(),
        1
    );
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn changed_url_does_not_reuse_the_previous_source_cache() -> TestResult {
    let directory = TestDirectory::new()?;
    let old = HttpServer::new("old.test\n").await?;
    let (mut server, address, _upstream) = start_remote(&directory, &old.url, 86400, 8192).await?;
    wait_for_code(address, "old.test.", ResponseCode::NXDomain).await?;
    server.shutdown_gracefully().await?;
    let new = HttpServer::new("new.test\n").await?;
    let (mut server, address, _upstream) = start_remote(&directory, &new.url, 86400, 8192).await?;
    wait_for_code(address, "new.test.", ResponseCode::NXDomain).await?;
    assert_eq!(udp(address, &query("old.test.")?).await?.0.answers.len(), 1);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn remote_badfilter_only_list_can_disable_a_local_rule() -> TestResult {
    let directory = TestDirectory::new()?;
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    let path = directory.0.join("block.txt");
    tokio::fs::write(&path, "||ads.test^\n").await?;
    let http = HttpServer::new("||ads.test^$badfilter\n").await?;
    config.filtering.blocklists = vec![path];
    config.filtering.sources = vec![FilterSource {
        name: "test".into(),
        url: http.url.clone(),
        allow_dnsrewrite: false,
    }];
    let (mut server, address) = bind(config).await?;
    wait_for_code(address, "ads.test.", ResponseCode::NoError).await?;
    assert!(directory.0.join("cache/test.txt").exists());
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn rejected_combined_update_keeps_the_previous_disk_cache_and_recovers() -> TestResult {
    let directory = TestDirectory::new()?;
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    let local = directory.0.join("block.txt");
    tokio::fs::write(&local, "local.test\n").await?;
    let http = HttpServer::new("old.test\n").await?;
    config.filtering.blocklists = vec![local.clone()];
    config.filtering.sources = vec![FilterSource {
        name: "test".into(),
        url: http.url.clone(),
        allow_dnsrewrite: false,
    }];
    config.filtering.update_interval = Duration::from_secs(1);
    let (mut server, address) = bind(config).await?;
    wait_for_code(address, "old.test.", ResponseCode::NXDomain).await?;
    let cache = directory.0.join("cache/test.txt");
    let previous = tokio::fs::read(&cache).await?;
    tokio::fs::write(&local, "<html>invalid local file</html>").await?;
    http.set(200, "new.test\n", true, 0);
    let hits = http.hits.load(Ordering::SeqCst);
    wait_for_hit(&http, hits + 2).await;
    assert_eq!(tokio::fs::read(&cache).await?, previous);
    assert_eq!(
        udp(address, &query("old.test.")?).await?.0.response_code,
        ResponseCode::NXDomain
    );
    assert_eq!(
        udp(address, &query("new.test.")?).await?.0.response_code,
        ResponseCode::NoError
    );
    tokio::fs::write(&local, "local2.test\n").await?;
    wait_for_code(address, "new.test.", ResponseCode::NXDomain).await?;
    wait_for_code(address, "old.test.", ResponseCode::NoError).await?;
    assert!(
        tokio::fs::read_to_string(&cache)
            .await?
            .contains("new.test")
    );
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn remote_rewrite_updates_stay_inert_and_keep_ordinary_rules_offline() -> TestResult {
    let directory = TestDirectory::new()?;
    let body = "||redirect.test^$dnsrewrite=192.0.2.99\nblocked.test\n@@||safe.blocked.test^\n";
    let http = HttpServer::new(body).await?;
    let (mut server, address, _upstream) = start_remote(&directory, &http.url, 1, 8192).await?;
    wait_for_code(address, "blocked.test.", ResponseCode::NXDomain).await?;
    let check = |response: Message| {
        assert_eq!(response.response_code, ResponseCode::NoError);
        assert_eq!(response.answers[0].data.to_string(), "192.0.2.10");
    };
    check(udp(address, &query("redirect.test.")?).await?.0);
    check(udp(address, &query("safe.blocked.test.")?).await?.0);
    http.set(
        200,
        "||redirect.test^$important,dnsrewrite=REFUSED\nupdated.test\n",
        true,
        0,
    );
    wait_for_code(address, "updated.test.", ResponseCode::NXDomain).await?;
    check(udp(address, &query("redirect.test.")?).await?.0);
    let cache = directory.0.join("cache/test.txt");
    let saved = tokio::fs::read_to_string(&cache).await?;
    assert!(saved.contains("dnsrewrite=REFUSED"));
    server.shutdown_gracefully().await?;
    let url = http.url.clone();
    drop(http);
    let (mut server, address, _upstream) = start_remote(&directory, &url, 86400, 8192).await?;
    assert_eq!(
        udp(address, &query("updated.test.")?)
            .await?
            .0
            .response_code,
        ResponseCode::NXDomain
    );
    check(udp(address, &query("redirect.test.")?).await?.0);
    assert_eq!(tokio::fs::read_to_string(cache).await?, saved);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn reload_revokes_cached_remote_rewrites_but_keeps_toml_priority() -> TestResult {
    let directory = TestDirectory::new()?;
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    let url = "https://lists.invalid/list";
    tokio::fs::create_dir_all(&config.filtering.cache_dir).await?;
    tokio::fs::write(config.filtering.cache_dir.join("trusted.txt"), format!("! light-dns source {url}\n||redirect.test^$dnsrewrite=192.0.2.99\n||local.test^$dnsrewrite=192.0.2.99\n")).await?;
    config.filtering.sources = vec![FilterSource {
        name: "trusted".into(),
        url: url.into(),
        allow_dnsrewrite: true,
    }];
    config
        .rewrites
        .insert(Name::from_ascii("local.test.")?, "192.168.1.1".parse()?);
    let mut replacement = Config {
        dns: light_dns::config::DnsConfig {
            listen: config.dns.listen,
            upstreams: config.dns.upstreams.clone(),
            upstream_timeout: config.dns.upstream_timeout,
        },
        rewrites: config.rewrites.clone(),
        filtering: config.filtering.clone(),
        cache: config.cache.clone(),
    };
    let (mut server, address) = bind(config).await?;
    for (name, expected) in [
        ("redirect.test.", "192.0.2.99"),
        ("local.test.", "192.168.1.1"),
    ] {
        assert_eq!(
            udp(address, &query(name)?).await?.0.answers[0]
                .data
                .to_string(),
            expected
        );
    }
    replacement.filtering.sources[0].allow_dnsrewrite = false;
    server.control().reload(replacement).await?;
    assert_eq!(
        udp(address, &query("redirect.test.")?).await?.0.answers[0]
            .data
            .to_string(),
        "192.0.2.10"
    );
    assert_eq!(
        udp(address, &query("local.test.")?).await?.0.answers[0]
            .data
            .to_string(),
        "192.168.1.1"
    );
    server.shutdown_gracefully().await?;
    Ok(())
}
