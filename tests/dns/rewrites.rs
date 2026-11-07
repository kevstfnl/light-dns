use crate::support::{TestResult, dns::*};
use hickory_proto::{
    op::{Edns, Message, ResponseCode},
    rr::{DNSClass, Name, RData, RecordType},
};
use light_dns::{
    config::Config,
    dns::server::{DnsServer, bind},
};
use std::{
    io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::{
    net::{TcpStream, UdpSocket},
    time::timeout,
};

async fn bind_rules(
    rules: &[(&str, &str)],
    upstreams: &[SocketAddr],
) -> TestResult<(DnsServer, SocketAddr)> {
    let upstreams = toml::Value::Array(
        upstreams
            .iter()
            .map(|ip| toml::Value::String(ip.to_string()))
            .collect(),
    );
    let mut content =
        format!("[dns]\nlisten = '127.0.0.1:1053'\nupstreams = {upstreams}\n[rewrites]\n");
    for &(name, ip) in rules {
        content.push_str(&toml::to_string(&toml::Table::from_iter([(
            name.into(),
            toml::Value::String(ip.into()),
        )]))?);
    }
    let mut config = Config::from_toml(&content)?;
    config.dns.listen.set_port(0);
    Ok(bind(config).await?)
}

struct LocalServer {
    server: DnsServer,
    address: SocketAddr,
    upstream: UdpSocket,
}

impl LocalServer {
    async fn new(rules: &[(&str, &str)]) -> TestResult<Self> {
        let upstream = UdpSocket::bind("127.0.0.1:0").await?;
        let (server, address) = bind_rules(rules, &[upstream.local_addr()?]).await?;
        Ok(Self {
            server,
            address,
            upstream,
        })
    }

    async fn ask(&self, name: &str, kind: RecordType) -> TestResult<Message> {
        let mut request = query(name)?;
        request.queries[0].query_type = kind;
        let response = udp(self.address, &request).await?.0;
        assert_eq!(response.id, request.id);
        assert_eq!(response.queries, request.queries);
        Ok(response)
    }

    async fn finish(mut self) -> TestResult {
        let mut bytes = [0; 512];
        assert!(
            matches!(self.upstream.try_recv_from(&mut bytes), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
            "une requête locale a été transmise à l'upstream"
        );
        self.server.shutdown_gracefully().await?;
        Ok(())
    }
}

fn assert_address(response: &Message, name: &str, ip: &str) -> TestResult {
    assert_eq!(response.response_code, ResponseCode::NoError);
    assert_eq!(response.answers.len(), 1);
    let record = &response.answers[0];
    assert!(record.name.eq_case(&Name::from_ascii(name)?));
    assert_eq!(record.dns_class, DNSClass::IN);
    assert_eq!(record.ttl, 60);
    match (&record.data, ip.parse::<IpAddr>()?) {
        (RData::A(value), IpAddr::V4(expected)) => assert_eq!(value.0, expected),
        (RData::AAAA(value), IpAddr::V6(expected)) => assert_eq!(value.0, expected),
        (data, expected) => panic!("{data:?} ne correspond pas à {expected}"),
    }
    assert!(!response.truncation);
    assert!(!response.authentic_data);
    Ok(())
}

#[tokio::test]
async fn exact_ipv4_ipv6_names_and_normalization() -> TestResult {
    let local = LocalServer::new(&[
        ("NAS.LAN", "192.168.1.20"),
        ("nas6.lan.", "fd00::20"),
        ("XN--BCHER-KVA.LAN", "192.168.1.30"),
        ("_service.lan", "192.168.1.40"),
    ])
    .await?;
    for (name, kind, ip) in [
        ("NaS.lan.", RecordType::A, "192.168.1.20"),
        ("NAS6.LAN.", RecordType::AAAA, "fd00::20"),
        ("xn--bcher-kva.lan.", RecordType::A, "192.168.1.30"),
        ("_service.LAN.", RecordType::A, "192.168.1.40"),
    ] {
        assert_address(&local.ask(name, kind).await?, name, ip)?;
    }
    local.finish().await
}

#[tokio::test]
async fn exact_rules_win_even_if_the_requested_family_is_missing() -> TestResult {
    let local = LocalServer::new(&[("*.lan", "192.168.1.10"), ("nas.lan", "fd00::20")]).await?;
    assert_address(
        &local.ask("nas.lan.", RecordType::AAAA).await?,
        "nas.lan.",
        "fd00::20",
    )?;
    let response = local.ask("nas.lan.", RecordType::A).await?;
    assert_eq!(response.response_code, ResponseCode::NoError);
    assert!(response.answers.is_empty());
    assert_address(
        &local.ask("other.lan.", RecordType::A).await?,
        "other.lan.",
        "192.168.1.10",
    )?;
    local.finish().await
}

#[tokio::test]
async fn wildcards_cover_descendants_but_not_their_apex() -> TestResult {
    let local = LocalServer::new(&[("*.lan", "192.168.1.10")]).await?;
    for name in ["nas.lan.", "deep.unknown.LAN.", "*.lan."] {
        assert_address(&local.ask(name, RecordType::A).await?, name, "192.168.1.10")?;
    }
    let response = local.ask("lan.", RecordType::A).await?;
    assert_eq!(response.response_code, ResponseCode::NoError);
    assert!(response.answers.is_empty());
    local.finish().await
}

#[tokio::test]
async fn closest_wildcard_wins_without_type_or_apex_fallback() -> TestResult {
    let local = LocalServer::new(&[("*.lan", "192.168.1.10"), ("*.dev.lan", "fd00::10")]).await?;
    assert_address(
        &local.ask("deep.app.dev.lan.", RecordType::AAAA).await?,
        "deep.app.dev.lan.",
        "fd00::10",
    )?;
    for name in ["app.dev.lan.", "dev.lan."] {
        let response = local.ask(name, RecordType::A).await?;
        assert_eq!(response.response_code, ResponseCode::NoError);
        assert!(response.answers.is_empty());
    }
    local.finish().await
}

#[tokio::test]
async fn existing_names_and_empty_ancestors_block_broader_wildcards() -> TestResult {
    let local = LocalServer::new(&[
        ("*.lan", "192.168.1.10"),
        ("router.lan", "192.168.1.1"),
        ("deep.host.lan", "192.168.1.20"),
    ])
    .await?;
    for name in [
        "child.router.lan.",
        "missing.host.lan.",
        "child.deep.host.lan.",
    ] {
        let response = local.ask(name, RecordType::A).await?;
        assert_eq!(response.response_code, ResponseCode::NXDomain);
        assert!(response.answers.is_empty());
    }
    let empty = local.ask("host.lan.", RecordType::A).await?;
    assert_eq!(empty.response_code, ResponseCode::NoError);
    assert!(empty.answers.is_empty());
    assert_address(
        &local.ask("deep.host.lan.", RecordType::A).await?,
        "deep.host.lan.",
        "192.168.1.20",
    )?;
    local.finish().await
}

#[tokio::test]
async fn unavailable_types_are_nodata_and_any_returns_the_available_record() -> TestResult {
    let local = LocalServer::new(&[("v4.lan", "192.168.1.10"), ("v6.lan", "fd00::10")]).await?;
    for (name, kind) in [
        ("v4.lan.", RecordType::AAAA),
        ("v6.lan.", RecordType::A),
        ("v4.lan.", RecordType::MX),
        ("v6.lan.", RecordType::TXT),
    ] {
        let response = local.ask(name, kind).await?;
        assert_eq!(response.response_code, ResponseCode::NoError);
        assert!(response.answers.is_empty());
    }
    for (name, ip) in [("v4.lan.", "192.168.1.10"), ("v6.lan.", "fd00::10")] {
        assert_address(&local.ask(name, RecordType::ANY).await?, name, ip)?;
    }
    local.finish().await
}

#[tokio::test]
async fn nonrecursive_local_answers_preserve_flags_and_edns_without_authentication() -> TestResult {
    let local = LocalServer::new(&[("nas.lan", "192.168.1.10")]).await?;
    let mut request = query("nas.lan.")?;
    request.metadata.recursion_desired = false;
    request.metadata.checking_disabled = true;
    request.metadata.authentic_data = true;
    let mut edns = Edns::new();
    edns.set_dnssec_ok(true);
    request.edns = Some(edns);
    let response = udp(local.address, &request).await?.0;
    assert_address(&response, "nas.lan.", "192.168.1.10")?;
    assert!(!response.recursion_desired);
    assert!(response.recursion_available);
    assert!(response.checking_disabled);
    assert!(!response.authoritative);
    assert!(
        response
            .edns
            .as_ref()
            .is_some_and(|edns| edns.flags().dnssec_ok)
    );
    local.finish().await
}

#[tokio::test]
async fn local_answers_use_tcp_as_well_as_udp() -> TestResult {
    let local = LocalServer::new(&[("v4.lan", "192.168.1.10"), ("*.v6.lan", "fd00::10")]).await?;
    let mut stream = TcpStream::connect(local.address).await?;
    for (name, kind, ip) in [
        ("v4.lan.", RecordType::A, "192.168.1.10"),
        ("host.v6.lan.", RecordType::AAAA, "fd00::10"),
    ] {
        let mut request = query(name)?;
        request.queries[0].query_type = kind;
        write_tcp(&mut stream, &request).await?;
        let response = timeout(Duration::from_secs(3), read_tcp(&mut stream)).await??;
        assert_eq!(response.id, request.id);
        assert_address(&response, name, ip)?;
    }
    local.finish().await
}

#[tokio::test]
async fn unsupported_classes_and_zone_transfers_are_never_forwarded() -> TestResult {
    let local = LocalServer::new(&[("*.lan", "192.168.1.10")]).await?;
    let mut request = query("host.lan.")?;
    request.queries[0].query_class = DNSClass::CH;
    assert_eq!(
        udp(local.address, &request).await?.0.response_code,
        ResponseCode::Refused
    );
    for kind in [RecordType::AXFR, RecordType::IXFR] {
        assert_eq!(
            local.ask("lan.", kind).await?.response_code,
            ResponseCode::Refused
        );
    }
    local.finish().await
}

#[tokio::test]
async fn unrelated_names_and_exact_rule_parents_still_use_upstreams() -> TestResult {
    let upstream = Upstream::start(answer, answer).await?;
    let (mut server, address) = bind_rules(
        &[
            ("*.lan", "192.168.1.10"),
            ("service.example.com", "192.168.1.20"),
        ],
        &[upstream.address],
    )
    .await?;
    for name in [
        "example.com.",
        "other.example.com.",
        "host.notlan.",
        "lan.example.",
        "host.lan.example.",
    ] {
        let request = query(name)?;
        assert_eq!(
            udp(address, &request).await?.0.answers,
            answer(&request).answers
        );
    }
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn wildcard_lookup_handles_the_maximum_wire_name_length() -> TestResult {
    let local = LocalServer::new(&[("*.lan", "192.168.1.10")]).await?;
    let name = format!("{}lan.", "x.".repeat(125));
    let mut request = query(&name)?;
    let mut edns = Edns::new();
    edns.set_max_payload(1232);
    request.edns = Some(edns);
    assert_address(
        &udp(local.address, &request).await?.0,
        &name,
        "192.168.1.10",
    )?;
    local.finish().await
}
