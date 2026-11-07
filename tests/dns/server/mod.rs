mod handler;
mod response;

use crate::support::{TestResult, dns::*};
use hickory_proto::op::{Message, ResponseCode};
use light_dns::{
    config::{Config, DnsConfig},
    dns::server::bind,
};
use std::{collections::HashMap, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream, UdpSocket},
    time::timeout,
};

#[tokio::test]
async fn bind_failure_and_shutdown_release_sockets() -> TestResult {
    let occupied = TcpListener::bind("127.0.0.1:0").await?;
    let address = occupied.local_addr()?;
    let config = Config {
        dns: DnsConfig {
            listen: address,
            upstreams: vec![],
            upstream_timeout: Duration::from_secs(1),
        },
        rewrites: HashMap::new(),
    };
    assert!(bind(config).await.is_err());
    let _udp = UdpSocket::bind(address).await?;
    drop(_udp);
    drop(occupied);
    let (mut server, address) = start(vec![]).await?;
    let _idle = TcpStream::connect(address).await?;
    timeout(Duration::from_secs(1), server.shutdown_gracefully()).await??;
    let _udp = UdpSocket::bind(address).await?;
    let _tcp = TcpListener::bind(address).await?;
    Ok(())
}

#[tokio::test]
async fn malformed_packets_do_not_stop_udp_or_tcp() -> TestResult {
    let upstream = Upstream::start(answer, answer).await?;
    let (mut server, address) = start(vec![upstream.address]).await?;
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    socket.send_to(&[0, 1, 2], address).await?;
    let mut malformed = query("broken.test.")?.to_vec()?;
    malformed.pop();
    socket.send_to(&malformed, address).await?;
    let mut bytes = vec![0; 512];
    let (length, _) = timeout(Duration::from_secs(3), socket.recv_from(&mut bytes)).await??;
    assert_eq!(
        Message::from_vec(&bytes[..length])?.response_code,
        ResponseCode::FormErr
    );
    let mut stream = TcpStream::connect(address).await?;
    stream.write_u16(3).await?;
    stream.write_all(&[0, 1, 2]).await?;
    let request = query("healthy.test.")?;
    write_tcp(&mut stream, &request).await?;
    assert_eq!(
        timeout(Duration::from_secs(3), read_tcp(&mut stream))
            .await??
            .answers
            .len(),
        1
    );
    assert_eq!(udp(address, &request).await?.0.answers.len(), 1);
    server.shutdown_gracefully().await?;
    Ok(())
}
