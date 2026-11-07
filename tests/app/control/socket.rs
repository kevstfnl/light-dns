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
