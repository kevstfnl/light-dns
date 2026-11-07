use crate::support::{TestResult, dns::*, process::TestDirectory};
use hickory_proto::{op::ResponseCode, rr::RData};
use std::{
    io,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    process::{Command, Output},
    time::Duration,
};
use tokio::{
    net::TcpStream,
    time::{sleep, timeout},
};

struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "--force", "--volumes", &self.0])
            .output();
    }
}

async fn docker(args: Vec<String>) -> TestResult<Output> {
    Ok(tokio::task::spawn_blocking(move || Command::new("docker").args(args).output()).await??)
}

async fn checked(args: &[&str]) -> TestResult<String> {
    let output = docker(args.iter().map(|arg| arg.to_string()).collect()).await?;
    if !output.status.success() {
        return Err(io::Error::other(String::from_utf8_lossy(&output.stderr).into_owned()).into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

impl Container {
    async fn port(&self, protocol: &str) -> TestResult<SocketAddr> {
        Ok(checked(&["port", &self.0, &format!("1053/{protocol}")])
            .await?
            .parse()?)
    }

    async fn cli(&self, command: &[&str]) -> TestResult<Output> {
        let mut args = vec!["exec".into(), self.0.clone(), "light-dns".into()];
        args.extend(command.iter().map(|arg| arg.to_string()));
        docker(args).await
    }

    async fn wait_healthy(&self) -> TestResult {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let health =
                checked(&["inspect", "--format", "{{.State.Health.Status}}", &self.0]).await?;
            if health == "healthy" {
                return Ok(());
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "healthcheck : {health}"
            );
            sleep(Duration::from_millis(100)).await;
        }
    }
}

#[tokio::test]
#[ignore = "nécessite Docker et l'image light-dns:validation construite localement"]
async fn nonroot_image_serves_udp_tcp_reloads_and_stops_cleanly() -> TestResult {
    let directory = TestDirectory::new()?;
    let image =
        std::env::var("LIGHT_DNS_TEST_IMAGE").unwrap_or_else(|_| "light-dns:validation".into());
    assert_eq!(
        checked(&[
            "image",
            "inspect",
            "--format",
            "{{.Os}}/{{.Architecture}}",
            &image
        ])
        .await?,
        "linux/amd64",
        "validation réservée à AMD64"
    );
    std::fs::set_permissions(&directory.0, std::fs::Permissions::from_mode(0o755))?;
    let path = directory.0.join("config.toml");
    let config = "[dns]\nlisten = '0.0.0.0:1053'\nupstreams = ['127.0.0.1:9']\n[rewrites]\n'*.home.arpa' = '192.168.1.10'\n[filtering]\ncache_dir = '/var/lib/light-dns/filters'\n[[filtering.sources]]\nname = 'test'\nurl = 'http://127.0.0.1:9/list.txt'\n";
    tokio::fs::write(&path, config).await?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
    let mount = format!(
        "type=bind,src={},dst=/etc/light-dns,readonly",
        directory.0.display()
    );
    let id = checked(&[
        "run",
        "--platform",
        "linux/amd64",
        "--detach",
        "--read-only",
        "--user",
        "65532:65532",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges:true",
        "--tmpfs",
        "/run/light-dns:uid=65532,gid=65532,mode=0700,size=1m",
        "--mount",
        &mount,
        "--volume",
        "/var/lib/light-dns",
        "--publish",
        "127.0.0.1::1053/udp",
        "--publish",
        "127.0.0.1::1053/tcp",
        "--health-cmd",
        "/usr/local/bin/light-dns stats",
        "--health-interval",
        "1s",
        "--health-timeout",
        "5s",
        "--health-retries",
        "3",
        &image,
    ])
    .await?;
    let container = Container(id);
    container.wait_healthy().await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !container.cli(&["stats"]).await?.status.success() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "conteneur indisponible"
        );
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        checked(&[
            "inspect",
            "--format",
            "{{.Config.User}} {{.HostConfig.ReadonlyRootfs}}",
            &container.0
        ])
        .await?,
        "65532:65532 true"
    );
    checked(&[
        "exec",
        &container.0,
        "sh",
        "-c",
        "test ! -w /etc/light-dns/config.toml && test -w /var/lib/light-dns/filters",
    ])
    .await?;
    let udp_address = container.port("udp").await?;
    let tcp_address = container.port("tcp").await?;
    let request = query("nas.home.arpa.")?;
    let mut stream = TcpStream::connect(tcp_address).await?;
    assert_eq!(
        udp(udp_address, &request).await?.0.response_code,
        ResponseCode::NoError
    );
    write_tcp(&mut stream, &request).await?;
    let original = timeout(Duration::from_secs(3), read_tcp(&mut stream)).await??;
    assert!(matches!(&original.answers[0].data, RData::A(_)));
    assert_eq!(original.answers[0].data.to_string(), "192.168.1.10");
    tokio::fs::write(&path, "[dns").await?;
    assert!(!container.cli(&["reload"]).await?.status.success());
    assert_eq!(
        udp(udp_address, &request).await?.0.answers[0]
            .data
            .to_string(),
        "192.168.1.10"
    );
    let replacement = path.with_extension("new");
    tokio::fs::write(&replacement, config.replace("192.168.1.10", "192.168.1.20")).await?;
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o644))?;
    tokio::fs::rename(replacement, &path).await?;
    assert!(container.cli(&["reload"]).await?.status.success());
    write_tcp(&mut stream, &request).await?;
    assert_eq!(
        timeout(Duration::from_secs(3), read_tcp(&mut stream))
            .await??
            .answers[0]
            .data
            .to_string(),
        "192.168.1.20"
    );
    assert_eq!(
        udp(udp_address, &request).await?.0.answers[0]
            .data
            .to_string(),
        "192.168.1.20"
    );
    assert!(container.cli(&["cache", "clear"]).await?.status.success());
    let stats = container.cli(&["stats"]).await?;
    assert!(stats.status.success());
    let stats: toml::Table = toml::from_str(&String::from_utf8(stats.stdout)?)?;
    assert_eq!(stats["reloads"].as_integer(), Some(1));
    assert_eq!(stats["cache_clears"].as_integer(), Some(1));
    // Seed an authentic cache using the container UID and its persistent volume.
    // Download/save behavior itself is covered by the local HTTP integration tests.
    checked(&["exec", &container.0, "sh", "-c", "printf '%s\n' '! light-dns source http://127.0.0.1:9/list.txt' 'blocked.test' > /var/lib/light-dns/filters/test.txt"]).await?;
    checked(&["restart", "--timeout", "8", &container.0]).await?;
    container.wait_healthy().await?;
    let udp_address = container.port("udp").await?;
    assert_eq!(
        udp(udp_address, &query("blocked.test.")?)
            .await?
            .0
            .response_code,
        ResponseCode::NXDomain
    );
    let stats = container.cli(&["stats"]).await?;
    assert!(stats.status.success());
    let stats: toml::Table = toml::from_str(&String::from_utf8(stats.stdout)?)?;
    assert_eq!(stats["filter_rules"].as_integer(), Some(1));
    checked(&["stop", "--timeout", "8", &container.0]).await?;
    assert_eq!(
        checked(&["inspect", "--format", "{{.State.ExitCode}}", &container.0]).await?,
        "0"
    );
    Ok(())
}
