mod socket;

use crate::support::{TestResult, dns::*, process::TestDirectory};
use hickory_proto::{
    op::Message,
    rr::{RData, rdata::A},
};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    process::{Child, Command, Output, Stdio},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    time::{sleep, timeout},
};

struct Daemon {
    directory: TestDirectory,
    child: Child,
    address: SocketAddr,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn cli(directory: &Path, args: &[&str]) -> TestResult<Output> {
    let directory = directory.to_owned();
    let args: Vec<_> = args.iter().map(|arg| arg.to_string()).collect();
    Ok(tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(directory)
            .args(args)
            .output()
    })
    .await??)
}

impl Daemon {
    async fn new(upstream: SocketAddr) -> TestResult<Self> {
        let directory = TestDirectory::new()?;
        let udp = UdpSocket::bind("127.0.0.1:0").await?;
        let address = udp.local_addr()?;
        let tcp = TcpListener::bind(address).await?;
        tokio::fs::write(
            directory.0.join("settings.toml"),
            configuration(address, upstream, "192.168.1.1"),
        )
        .await?;
        drop((udp, tcp));
        let child = Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(&directory.0)
            .args(["--config", "settings.toml", "--socket", "admin.sock"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut daemon = Self {
            directory,
            child,
            address,
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let result = daemon.command(&["stats"]).await?;
            if result.status.success() {
                break;
            }
            assert!(daemon.child.try_wait()?.is_none(), "arrêt prématuré");
            assert!(
                tokio::time::Instant::now() < deadline,
                "démarrage trop lent"
            );
            sleep(Duration::from_millis(10)).await;
        }
        Ok(daemon)
    }

    async fn command(&self, args: &[&str]) -> TestResult<Output> {
        let mut options = vec!["--socket", "admin.sock"];
        options.extend_from_slice(args);
        cli(&self.directory.0, &options).await
    }

    async fn stats(&self) -> TestResult<toml::Table> {
        let output = self.command(&["stats"]).await?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(toml::from_str(&String::from_utf8(output.stdout)?)?)
    }

    async fn cache_status(&self) -> TestResult<toml::Table> {
        let output = self.command(&["cache", "status"]).await?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(toml::from_str(&String::from_utf8(output.stdout)?)?)
    }

    async fn replace_config(&self, content: &str) -> TestResult {
        let path = self.directory.0.join("settings.toml");
        tokio::fs::write(path.with_extension("new"), content).await?;
        tokio::fs::rename(path.with_extension("new"), path).await?;
        Ok(())
    }
}

fn configuration(address: SocketAddr, upstream: SocketAddr, ip: &str) -> String {
    format!(
        "[dns]\nlisten = '{address}'\nupstreams = ['{upstream}']\n[rewrites]\n'local.test' = '{ip}'\n[filtering]\nenabled = false\n"
    )
}

fn address(response: &Message) -> String {
    response.answers[0].data.to_string()
}

#[tokio::test]
async fn cli_clears_cache_reports_stats_and_reloads_without_closing_tcp() -> TestResult {
    fn next(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers[0].data = RData::A(A(Ipv4Addr::new(192, 0, 2, 20)));
        response
    }
    let upstream = Upstream::start(answer, answer).await?;
    let next_upstream = Upstream::start(next, next).await?;
    let daemon = Daemon::new(upstream.address).await?;
    let initial = daemon.cache_status().await?;
    assert_eq!(initial["enabled"].as_bool(), Some(true));
    assert_eq!(initial["entries"].as_integer(), Some(0));
    assert_eq!(initial["hits"].as_integer(), Some(0));
    assert_eq!(initial["max_entries"].as_integer(), Some(4096));
    assert_eq!(initial["max_memory_bytes"].as_integer(), Some(8388608));
    assert_eq!(initial["max_ttl_secs"].as_integer(), Some(86400));
    let request = query("public.test.")?;
    for _ in 0..2 {
        udp(daemon.address, &request).await?;
    }
    let stats = daemon.stats().await?;
    for (key, value) in [
        ("requests", 2),
        ("forwarded", 1),
        ("cache_hits", 1),
        ("cache_entries", 1),
    ] {
        assert_eq!(stats[key].as_integer(), Some(value));
    }
    let cache = daemon.cache_status().await?;
    assert_eq!(cache["entries"].as_integer(), Some(1));
    assert_eq!(cache["hits"].as_integer(), Some(1));
    assert!(cache["estimated_bytes"].as_integer().is_some_and(|n| n > 0));
    // Reporting leaves the cached response intact and does not issue DNS queries.
    assert_eq!(
        daemon.cache_status().await?["entries"].as_integer(),
        Some(1)
    );
    assert_eq!(daemon.stats().await?["requests"].as_integer(), Some(2));
    assert!(daemon.command(&["cache", "clear"]).await?.status.success());
    let cleared = daemon.cache_status().await?;
    assert_eq!(cleared["entries"].as_integer(), Some(0));
    assert_eq!(cleared["estimated_bytes"].as_integer(), Some(0));
    assert_eq!(cleared["clears"].as_integer(), Some(1));
    assert_eq!(cleared["hits"].as_integer(), Some(1));
    assert_eq!(daemon.stats().await?["cache_entries"].as_integer(), Some(0));
    udp(daemon.address, &request).await?;
    assert_eq!(daemon.stats().await?["forwarded"].as_integer(), Some(2));
    let mut tcp = TcpStream::connect(daemon.address).await?;
    let local = query("local.test.")?;
    write_tcp(&mut tcp, &local).await?;
    assert_eq!(address(&read_tcp(&mut tcp).await?), "192.168.1.1");
    let content = configuration(daemon.address, next_upstream.address, "192.168.1.2");
    daemon.replace_config(&content).await?;
    assert!(daemon.command(&["reload"]).await?.status.success());
    assert_eq!(daemon.stats().await?["cache_entries"].as_integer(), Some(0));
    write_tcp(&mut tcp, &local).await?;
    assert_eq!(
        address(&timeout(Duration::from_secs(3), read_tcp(&mut tcp)).await??),
        "192.168.1.2"
    );
    assert_eq!(
        address(&udp(daemon.address, &request).await?.0),
        "192.0.2.20"
    );
    let stats = daemon.stats().await?;
    assert_eq!(stats["reloads"].as_integer(), Some(1));
    assert_eq!(stats["cache_clears"].as_integer(), Some(1));
    for invalid in [
        "[dns".to_string(),
        content.replace(&daemon.address.to_string(), "127.0.0.1:9999"),
        content.replace(
            "enabled = false",
            "sources = []\nblocklists = ['missing.txt']",
        ),
        format!("{content}\n[cache]\nmax_entries = 0\n"),
    ] {
        daemon.replace_config(&invalid).await?;
        let output = daemon.command(&["reload"]).await?;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            address(&udp(daemon.address, &local).await?.0),
            "192.168.1.2"
        );
        assert_eq!(daemon.stats().await?["reloads"].as_integer(), Some(1));
        assert_eq!(daemon.stats().await?["cache_entries"].as_integer(), Some(1));
    }
    Ok(())
}

#[tokio::test]
async fn reload_replaces_filtering_and_cache_settings() -> TestResult {
    use hickory_proto::op::ResponseCode;
    let upstream = Upstream::start(answer, answer).await?;
    let daemon = Daemon::new(upstream.address).await?;
    let content = configuration(daemon.address, upstream.address, "192.168.1.1").replace(
        "enabled = false",
        "sources = []\nblocklists = ['block.txt']",
    );
    tokio::fs::write(daemon.directory.0.join("block.txt"), "ads.test\n").await?;
    daemon.replace_config(&content).await?;
    assert!(daemon.command(&["reload"]).await?.status.success());
    assert_eq!(
        udp(daemon.address, &query("ads.test.")?)
            .await?
            .0
            .response_code,
        ResponseCode::NXDomain
    );
    let stats = daemon.stats().await?;
    assert_eq!(stats["blocked"].as_integer(), Some(1));
    assert!(
        stats["filter_rules"]
            .as_integer()
            .is_some_and(|count| count > 0)
    );
    daemon
        .replace_config(&format!(
            "{}\n[cache]\nenabled = false\n",
            content.replace(
                "blocklists = ['block.txt']",
                "blocklists = ['block.txt']\nallowlist = ['ads.test']"
            )
        ))
        .await?;
    assert!(daemon.command(&["reload"]).await?.status.success());
    for _ in 0..2 {
        assert_eq!(
            udp(daemon.address, &query("ads.test.")?)
                .await?
                .0
                .response_code,
            ResponseCode::NoError
        );
    }
    let stats = daemon.stats().await?;
    assert_eq!(stats["cache_entries"].as_integer(), Some(0));
    assert_eq!(stats["forwarded"].as_integer(), Some(2));
    let disabled = daemon.cache_status().await?;
    assert_eq!(disabled["enabled"].as_bool(), Some(false));
    assert_eq!(disabled["entries"].as_integer(), Some(0));
    assert_eq!(disabled["estimated_bytes"].as_integer(), Some(0));
    Ok(())
}

#[tokio::test]
async fn health_distinguishes_local_dns_and_upstream_resolution() -> TestResult {
    use hickory_proto::rr::{Name, Record, rdata::NS};
    fn root(request: &Message) -> Message {
        let mut response = answer(request);
        response.answers = vec![Record::from_rdata(
            Name::root(),
            60,
            RData::NS(NS(Name::from_ascii("ns.test.").unwrap())),
        )];
        response
    }
    let upstream = Upstream::start(root, root).await?;
    let daemon = Daemon::new(upstream.address).await?;
    assert!(daemon.command(&["health"]).await?.status.success());
    assert!(
        daemon
            .command(&["health", "--upstream"])
            .await?
            .status
            .success()
    );
    drop(upstream);
    assert!(daemon.command(&["health"]).await?.status.success());
    assert!(
        !daemon
            .command(&["health", "--upstream"])
            .await?
            .status
            .success()
    );
    // Ne relit pas le TOML pour contrôler l'état réellement actif.
    daemon.replace_config("[broken").await?;
    assert!(daemon.command(&["health"]).await?.status.success());
    Ok(())
}
