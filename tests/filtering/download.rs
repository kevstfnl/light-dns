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
