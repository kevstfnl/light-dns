use super::TestResult;
use hickory_proto::{
    op::{Message, Query},
    rr::{Name, RData, Record, RecordType, rdata::A},
    serialize::binary::{BinEncodable, BinEncoder},
};
use light_dns::{
    config::{Config, DnsConfig},
    dns::server::{DnsServer, bind},
};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    task::JoinHandle,
    time::timeout,
};

pub(crate) fn query(name: &str) -> TestResult<Message> {
    let mut message = Message::query();
    message.metadata.recursion_desired = true;
    message
        .queries
        .push(Query::query(Name::from_ascii(name)?, RecordType::A));
    Ok(message)
}

pub(crate) fn answer(request: &Message) -> Message {
    let mut response = Message::response(request.id, request.op_code);
    response.metadata.recursion_available = true;
    response.metadata.recursion_desired = request.recursion_desired;
    response.queries = request.queries.clone();
    response.edns = request.edns.clone();
    response.answers.push(Record::from_rdata(
        request.queries[0].name().clone(),
        123,
        RData::A(A(Ipv4Addr::new(192, 0, 2, 10))),
    ));
    response
}

pub(crate) struct Upstream {
    pub(crate) address: SocketAddr,
    pub(crate) hits: Arc<AtomicUsize>,
    udp: JoinHandle<()>,
    tcp: JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.udp.abort();
        self.tcp.abort();
    }
}

impl Upstream {
    pub(crate) async fn start(
        udp_response: fn(&Message) -> Message,
        tcp_response: fn(&Message) -> Message,
    ) -> TestResult<Self> {
        Self::start_on("127.0.0.1:0", udp_response, tcp_response).await
    }

    pub(crate) async fn start_on(
        listen: &str,
        udp_response: fn(&Message) -> Message,
        tcp_response: fn(&Message) -> Message,
    ) -> TestResult<Self> {
        let udp = UdpSocket::bind(listen).await?;
        let address = udp.local_addr()?;
        let tcp = TcpListener::bind(address).await?;
        let hits = Arc::new(AtomicUsize::new(0));
        let udp_hits = hits.clone();
        let tcp_hits = hits.clone();
        let udp = tokio::spawn(async move {
            let mut bytes = vec![0; 65535];
            while let Ok((length, peer)) = udp.recv_from(&mut bytes).await {
                let request = Message::from_vec(&bytes[..length]).unwrap();
                udp_hits.fetch_add(1, Ordering::SeqCst);
                let response = udp_response(&request);
                let mut bytes = Vec::new();
                let mut encoder = BinEncoder::new(&mut bytes);
                encoder.set_max_size(request.max_payload());
                response.emit(&mut encoder).unwrap();
                udp.send_to(&bytes, peer).await.unwrap();
            }
        });
        let tcp = tokio::spawn(async move {
            while let Ok((mut stream, _)) = tcp.accept().await {
                let request = read_tcp(&mut stream).await.unwrap();
                tcp_hits.fetch_add(1, Ordering::SeqCst);
                write_tcp(&mut stream, &tcp_response(&request))
                    .await
                    .unwrap();
            }
        });
        Ok(Self {
            address,
            hits,
            udp,
            tcp,
        })
    }
}

pub(crate) async fn start(upstreams: Vec<SocketAddr>) -> TestResult<(DnsServer, SocketAddr)> {
    Ok(bind(test_config(upstreams)?).await?)
}

pub(crate) fn test_config(upstreams: Vec<SocketAddr>) -> TestResult<Config> {
    Ok(Config {
        cache: light_dns::config::CacheConfig::disabled(),
        filtering: light_dns::config::FilteringConfig::disabled(),
        dns: DnsConfig {
            listen: "127.0.0.1:0".parse()?,
            upstreams,
            upstream_timeout: Duration::from_millis(200),
        },
        rewrites: HashMap::new(),
    })
}

pub(crate) async fn udp(address: SocketAddr, request: &Message) -> TestResult<(Message, usize)> {
    let socket = UdpSocket::bind(if address.is_ipv6() {
        "[::1]:0"
    } else {
        "127.0.0.1:0"
    })
    .await?;
    socket.send_to(&request.to_vec()?, address).await?;
    let mut bytes = vec![0; 65535];
    let (length, _) = timeout(Duration::from_secs(3), socket.recv_from(&mut bytes)).await??;
    Ok((Message::from_vec(&bytes[..length])?, length))
}

pub(crate) async fn read_tcp(stream: &mut TcpStream) -> TestResult<Message> {
    let length = stream.read_u16().await? as usize;
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(Message::from_vec(&bytes)?)
}

pub(crate) async fn write_tcp(stream: &mut TcpStream, message: &Message) -> TestResult {
    let bytes = message.to_vec()?;
    stream.write_u16(bytes.len().try_into()?).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}
