use super::*;
use std::os::unix::fs::PermissionsExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

#[tokio::test]
async fn malformed_and_slow_control_clients_do_not_block_dns() -> TestResult {
    let upstream = Upstream::start(answer, answer).await?;
    let daemon = Daemon::new(upstream.address).await?;
    let path = daemon.directory.0.join("admin.sock");
    assert_eq!(
        tokio::fs::metadata(&path).await?.permissions().mode() & 0o777,
        0o600
    );
    for frame in [b"unknown\n".to_vec(), vec![b'x'; 65], vec![]] {
        let mut stream = UnixStream::connect(&path).await?;
        stream.write_all(&frame).await?;
        stream.shutdown().await?;
        let mut response = String::new();
        let result = timeout(Duration::from_secs(3), stream.read_to_string(&mut response)).await?;
        // Une fermeture avec des octets excédentaires non lus peut produire ECONNRESET.
        match result {
            Ok(_) => assert!(response.starts_with("ERR\n")),
            Err(error)
                if frame.len() > 64 && error.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut slow = UnixStream::connect(&path).await?;
    slow.write_all(b"sta").await?;
    assert_eq!(
        address(&udp(daemon.address, &query("local.test.")?).await?.0),
        "192.168.1.1"
    );
    assert!(daemon.command(&["stats"]).await?.status.success());
    let mut response = String::new();
    timeout(Duration::from_secs(3), slow.read_to_string(&mut response)).await??;
    assert!(response.starts_with("ERR\n"));
    Ok(())
}

#[tokio::test]
async fn existing_control_paths_are_preserved() -> TestResult {
    let upstream = Upstream::start(answer, answer).await?;
    let daemon = Daemon::new(upstream.address).await?;
    let duplicate = cli(
        &daemon.directory.0,
        &["--config", "settings.toml", "--socket", "admin.sock"],
    )
    .await?;
    assert!(!duplicate.status.success());
    assert!(daemon.command(&["stats"]).await?.status.success());
    let file = daemon.directory.0.join("important.txt");
    tokio::fs::write(&file, "conserver ce fichier").await?;
    let result = cli(
        &daemon.directory.0,
        &["--config", "settings.toml", "--socket", "important.txt"],
    )
    .await?;
    assert!(!result.status.success());
    assert_eq!(
        tokio::fs::read_to_string(file).await?,
        "conserver ce fichier"
    );
    Ok(())
}

#[tokio::test]
async fn killed_daemon_recovers_socket_without_removing_protected_paths() -> TestResult {
    let upstream = Upstream::start(answer, answer).await?;
    let mut daemon = Daemon::new(upstream.address).await?;
    daemon.child.kill()?;
    daemon.child.wait()?;
    assert!(daemon.directory.0.join("admin.sock").exists());
    daemon.child = Command::new(env!("CARGO_BIN_EXE_light-dns"))
        .current_dir(&daemon.directory.0)
        .args(["--config", "settings.toml", "--socket", "admin.sock"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        if daemon.command(&["stats"]).await?.status.success() {
            break;
        }
        assert!(daemon.child.try_wait()?.is_none());
        assert!(tokio::time::Instant::now() < until);
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        address(&udp(daemon.address, &query("local.test.")?).await?.0),
        "192.168.1.1"
    );
    for suffix in ["link.sock", "lock-link.sock.lock"] {
        let target = daemon.directory.0.join("protected.txt");
        tokio::fs::write(&target, "keep").await?;
        std::os::unix::fs::symlink(&target, daemon.directory.0.join(suffix))?;
        let socket = if suffix.ends_with(".lock") {
            "lock-link.sock"
        } else {
            suffix
        };
        assert!(
            !cli(
                &daemon.directory.0,
                &["--config", "settings.toml", "--socket", socket]
            )
            .await?
            .status
            .success()
        );
        assert_eq!(tokio::fs::read_to_string(target).await?, "keep");
    }
    Ok(())
}

#[tokio::test]
async fn concurrent_startup_keeps_one_control_socket_owner() -> TestResult {
    let upstream = Upstream::start(answer, answer).await?;
    let mut daemon = Daemon::new(upstream.address).await?;
    daemon.child.kill()?;
    daemon.child.wait()?;
    let launch = || {
        Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(&daemon.directory.0)
            .args(["--config", "settings.toml", "--socket", "admin.sock"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    };
    let mut first = launch()?;
    let mut second = launch()?;
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let first_status = first.try_wait()?;
        let second_status = second.try_wait()?;
        if first_status.is_some() || second_status.is_some() {
            assert_ne!(first_status.is_some(), second_status.is_some());
            if let Some(status) = first_status {
                assert!(!status.success());
                daemon.child = second;
            } else {
                assert!(!second_status.is_some_and(|status| status.success()));
                daemon.child = first;
            }
            break;
        }
        if tokio::time::Instant::now() >= until {
            let _ = first.kill();
            let _ = first.wait();
            let _ = second.kill();
            let _ = second.wait();
            return Err(std::io::Error::other("démarrage concurrent expiré").into());
        }
        sleep(Duration::from_millis(10)).await;
    }
    let until = tokio::time::Instant::now() + Duration::from_secs(3);
    while !daemon.command(&["health"]).await?.status.success() {
        assert!(tokio::time::Instant::now() < until);
        sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

#[tokio::test]
async fn homelab_keeps_local_zones_filters_and_persisted_lists_across_crashes() -> TestResult {
    use hickory_proto::op::ResponseCode;
    use std::sync::atomic::Ordering;
    let healthy = Upstream::start(answer, answer).await?;
    let silent = UdpSocket::bind("127.0.0.1:0").await?;
    let mut daemon = Daemon::new(healthy.address).await?;
    let cache = daemon.directory.0.join("filters");
    tokio::fs::create_dir(&cache).await?;
    let persisted = "! light-dns source https://lists.invalid/list\nremote.block.test\n||redirect.test^$dnsrewrite=192.0.2.99\n";
    tokio::fs::write(cache.join("remote.txt"), persisted).await?;
    tokio::fs::write(daemon.directory.0.join("block.txt"), "ads.test\n").await?;
    let config = format!(
        "[dns]\nlisten = '{}'\nupstreams = ['{}', '{}']\nupstream_timeout_ms = 100\n[rewrites]\n'*.home.arpa' = '192.168.1.10'\n'router.home.arpa' = '192.168.1.1'\n[filtering]\ncache_dir = 'filters'\nblocklists = ['block.txt']\nallowlist = ['safe.ads.test']\nsources = [{{name = 'remote', url = 'https://lists.invalid/list'}}]\n",
        daemon.address,
        silent.local_addr()?,
        healthy.address
    );
    daemon.replace_config(&config).await?;
    assert!(daemon.command(&["reload"]).await?.status.success());
    for name in [
        "router.home.arpa.",
        "nas.home.arpa.",
        "deep.host.home.arpa.",
        "home.arpa.",
        "ads.test.",
        "remote.block.test.",
    ] {
        udp(daemon.address, &query(name)?).await?;
    }
    assert_eq!(healthy.hits.load(Ordering::SeqCst), 0);
    let mut bytes = [0; 4096];
    assert!(
        matches!(silent.try_recv_from(&mut bytes), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    // Déclencher la bascule puis vérifier que le cache évite tout nouvel échange.
    for _ in 0..2 {
        assert_eq!(
            address(&udp(daemon.address, &query("public.test.")?).await?.0),
            "192.0.2.10"
        );
    }
    assert_eq!(healthy.hits.load(Ordering::SeqCst), 1);
    let mut clients = tokio::task::JoinSet::new();
    for (name, expected) in [
        ("router.home.arpa.", "192.168.1.1"),
        ("nas.home.arpa.", "192.168.1.10"),
        ("safe.ads.test.", "192.0.2.10"),
        ("redirect.test.", "192.0.2.10"),
    ] {
        let endpoint = daemon.address;
        clients.spawn(async move {
            assert_eq!(address(&udp(endpoint, &query(name)?).await?.0), expected);
            TestResult::Ok(())
        });
    }
    while let Some(result) = clients.join_next().await {
        result??;
    }
    for name in ["ads.test.", "remote.block.test."] {
        assert_eq!(
            udp(daemon.address, &query(name)?).await?.0.response_code,
            ResponseCode::NXDomain
        );
    }
    let mut tcp = TcpStream::connect(daemon.address).await?;
    write_tcp(&mut tcp, &query("nas.home.arpa.")?).await?;
    assert_eq!(address(&read_tcp(&mut tcp).await?), "192.168.1.10");
    daemon
        .replace_config(&config.replace("192.168.1.10", "192.168.1.20"))
        .await?;
    assert!(daemon.command(&["reload"]).await?.status.success());
    write_tcp(&mut tcp, &query("nas.home.arpa.")?).await?;
    assert_eq!(address(&read_tcp(&mut tcp).await?), "192.168.1.20");
    drop(tcp);
    for _ in 0..3 {
        daemon.child.kill()?;
        daemon.child.wait()?;
        daemon.child = Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(&daemon.directory.0)
            .args(["--config", "settings.toml", "--socket", "admin.sock"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        while !daemon.command(&["health"]).await?.status.success() {
            assert!(daemon.child.try_wait()?.is_none());
            assert!(tokio::time::Instant::now() < until);
            sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(
            address(&udp(daemon.address, &query("nas.home.arpa.")?).await?.0),
            "192.168.1.20"
        );
        assert_eq!(
            udp(daemon.address, &query("remote.block.test.")?)
                .await?
                .0
                .response_code,
            ResponseCode::NXDomain
        );
        assert_eq!(
            tokio::fs::read_to_string(cache.join("remote.txt")).await?,
            persisted
        );
    }
    Ok(())
}
