mod advanced;
mod download;
mod parser;

use crate::support::{TestResult, dns::*, process::TestDirectory};
use hickory_proto::{
    op::{Message, ResponseCode},
    rr::Name,
};
use light_dns::{
    config::{Config, FilteringConfig},
    dns::server::{DnsServer, bind},
};
use std::{net::SocketAddr, time::Duration};
use tokio::time::{Instant, sleep};

struct TestServer {
    server: DnsServer,
    address: SocketAddr,
    _upstream: Upstream,
    directory: TestDirectory,
}

impl TestServer {
    async fn config(directory: &TestDirectory) -> TestResult<(Config, Upstream)> {
        let upstream = Upstream::start(answer, answer).await?;
        let mut config = Config::from_toml(&format!(
            "[dns]\nlisten = '127.0.0.1:1053'\nupstreams = ['{}']\n[rewrites]\n[filtering]\nsources = []\n",
            upstream.address
        ))?;
        config.dns.listen.set_port(0);
        config.filtering.cache_dir = directory.0.join("cache");
        Ok((config, upstream))
    }

    async fn local(block: &str, allow: &str, exceptions: &[&str]) -> TestResult<Self> {
        let directory = TestDirectory::new()?;
        let (mut config, upstream) = Self::config(&directory).await?;
        let block_path = directory.0.join("block.txt");
        let allow_path = directory.0.join("allow.txt");
        tokio::fs::write(&block_path, block).await?;
        tokio::fs::write(&allow_path, allow).await?;
        config.filtering.blocklists = vec![block_path];
        config.filtering.update_interval = Duration::from_secs(1);
        config.filtering.allowlist_files = vec![allow_path];
        config.filtering.allowlist = exceptions
            .iter()
            .map(Name::from_ascii)
            .collect::<Result<_, _>>()?;
        let (server, address) = bind(config).await?;
        Ok(Self {
            server,
            address,
            _upstream: upstream,
            directory,
        })
    }

    async fn ask(&self, name: &str) -> TestResult<Message> {
        Ok(udp(self.address, &query(name)?).await?.0)
    }
    async fn stop(mut self) -> TestResult {
        self.server.shutdown_gracefully().await?;
        Ok(())
    }
}

async fn wait_for_code(address: SocketAddr, name: &str, expected: ResponseCode) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let response = udp(address, &query(name)?).await?.0;
        if response.response_code == expected {
            return Ok(());
        }
        assert!(
            Instant::now() < deadline,
            "code attendu : {expected}, reçu : {}",
            response.response_code
        );
        sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn disabled_filtering_does_not_read_files_or_download_sources() -> TestResult {
    let directory = TestDirectory::new()?;
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    config.filtering = FilteringConfig::disabled();
    config.filtering.blocklists = vec![directory.0.join("missing.txt")];
    let (mut server, address) = bind(config).await?;
    assert_eq!(udp(address, &query("ads.test.")?).await?.0.answers.len(), 1);
    assert!(!directory.0.join("cache").exists());
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn missing_or_oversized_local_files_fail_startup() -> TestResult {
    let directory = TestDirectory::new()?;
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    config.filtering.blocklists = vec![directory.0.join("missing.txt")];
    assert!(bind(config).await.is_err());
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    let path = directory.0.join("big.txt");
    tokio::fs::write(&path, "a".repeat(1025)).await?;
    config.filtering.blocklists = vec![path];
    config.filtering.max_list_bytes = 1024;
    assert!(bind(config).await.is_err());
    Ok(())
}
