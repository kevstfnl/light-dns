use super::*;
use hickory_proto::rr::{RData, Record, RecordType, rdata::CNAME};
use tokio::net::TcpStream;

#[tokio::test]
async fn typed_patterns_exclusions_and_badfilter_keep_rule_conditions() -> TestResult {
    let server = TestServer::local("sentinel.test\n||typed.test^$dnstype=AAAA\n||typed.test^$dnstype=A\n||typed.test^$badfilter,dnstype=A\n||ads*.test^$denyallow=ads-safe.test\n|exact.test|\n|prefix*\nend.test|\n", "", &[]).await?;
    for (name, kind, code) in [
        ("typed.test.", RecordType::A, ResponseCode::NoError),
        ("typed.test.", RecordType::AAAA, ResponseCode::NXDomain),
        ("sub.ads1.test.", RecordType::A, ResponseCode::NXDomain),
        ("ads-safe.test.", RecordType::A, ResponseCode::NoError),
        ("sub.ads-safe.test.", RecordType::A, ResponseCode::NoError),
        ("exact.test.", RecordType::A, ResponseCode::NXDomain),
        ("sub.exact.test.", RecordType::A, ResponseCode::NoError),
        ("prefix.test.", RecordType::A, ResponseCode::NXDomain),
        ("notend.test.", RecordType::A, ResponseCode::NXDomain),
        ("end.test.other.", RecordType::A, ResponseCode::NoError),
    ] {
        let mut request = query(name)?;
        request.queries[0].query_type = kind;
        assert_eq!(
            udp(server.address, &request).await?.0.response_code,
            code,
            "{name} {kind}"
        );
    }
    server.stop().await
}

#[tokio::test]
async fn dnsrewrite_merges_families_and_respects_exceptions_and_priorities() -> TestResult {
    let server = TestServer::local("||multi.test^\n||multi.test^$dnsrewrite=192.0.2.1\n||multi.test^$dnsrewrite=192.0.2.2\n||multi.test^$dnsrewrite=2001:db8::1\n@@||multi.test^$dnsrewrite=192.0.2.2\n||refused.test^$dnsrewrite=REFUSED\n||nodata.test^$dnsrewrite=NOERROR;;\n||except.test^$dnsrewrite=192.0.2.8\n@@||except.test^$dnsrewrite\n||user.test^$dnsrewrite=192.0.2.8\n", "", &["user.test"]).await?;
    for (kind, expected) in [
        (RecordType::A, "192.0.2.1"),
        (RecordType::AAAA, "2001:db8::1"),
    ] {
        let mut request = query("multi.test.")?;
        request.queries[0].query_type = kind;
        let mut stream = TcpStream::connect(server.address).await?;
        write_tcp(&mut stream, &request).await?;
        for response in [
            udp(server.address, &request).await?.0,
            read_tcp(&mut stream).await?,
        ] {
            assert_eq!(response.answers.len(), 1);
            assert_eq!(response.answers[0].data.to_string(), expected);
            assert_eq!(response.answers[0].ttl, 60);
            assert!(!response.authentic_data && !response.authoritative);
        }
    }
    assert_eq!(
        server.ask("refused.test.").await?.response_code,
        ResponseCode::Refused
    );
    assert!(server.ask("nodata.test.").await?.answers.is_empty());
    for name in ["except.test.", "user.test."] {
        assert_eq!(
            server.ask(name).await?.answers[0].data.to_string(),
            "192.0.2.10"
        );
    }
    server.stop().await
}

#[tokio::test]
async fn dnsrewrite_supports_record_data_and_cname_resolution_without_loops() -> TestResult {
    let rules = "||alias.test^$dnsrewrite=target.test\n||target.test^$dnsrewrite=192.0.2.99\n||loop.test^$dnsrewrite=other-loop.test\n||other-loop.test^$dnsrewrite=loop.test\n||remote-alias.test^$dnsrewrite=remote.test\n||ptr.test^$dnsrewrite=NOERROR;PTR;target.test.\n||txt.test^$dnsrewrite=NOERROR;TXT;hello_world\n||mx.test^$dnsrewrite=NOERROR;MX;10 mail.test.\n||srv.test^$dnsrewrite=NOERROR;SRV;10 60 8080 target.test.\n||https.test^$dnsrewrite=NOERROR;HTTPS;1 target.test. alpn=h3\n||svcb.test^$dnsrewrite=NOERROR;SVCB;1 target.test. port=8443\n";
    let server = TestServer::local(rules, "", &[]).await?;
    for (name, kind) in [
        ("ptr.test.", RecordType::PTR),
        ("txt.test.", RecordType::TXT),
        ("mx.test.", RecordType::MX),
        ("srv.test.", RecordType::SRV),
        ("https.test.", RecordType::HTTPS),
        ("svcb.test.", RecordType::SVCB),
    ] {
        let mut request = query(name)?;
        request.queries[0].query_type = kind;
        let response = udp(server.address, &request).await?.0;
        assert_eq!(response.answers.len(), 1, "{name}");
        assert_eq!(response.answers[0].data.record_type(), kind);
    }
    let response = server.ask("alias.test.").await?;
    assert_eq!(response.answers.len(), 2);
    assert_eq!(response.answers[1].data.to_string(), "192.0.2.99");
    assert_eq!(
        server.ask("loop.test.").await?.response_code,
        ResponseCode::ServFail
    );
    assert_eq!(server.ask("remote-alias.test.").await?.answers.len(), 2);
    let mut request = query("remote-alias.test.")?;
    request.metadata.recursion_desired = false;
    // Données target déjà en cache disponibles sans récursion.
    assert_eq!(udp(server.address, &request).await?.0.answers.len(), 2);
    server.stop().await
}

fn cname_answer(request: &Message) -> Message {
    let mut response = answer(request);
    response.answers.insert(
        0,
        Record::from_rdata(
            request.queries[0].name().clone(),
            120,
            RData::CNAME(CNAME(Name::from_ascii("tracker.test.").unwrap())),
        ),
    );
    response.answers[1].name = Name::from_ascii("tracker.test.").unwrap();
    response.answers.push(Record::from_rdata(
        Name::from_ascii("unrelated.test.").unwrap(),
        120,
        RData::CNAME(CNAME(Name::from_ascii("unrelated-block.test.").unwrap())),
    ));
    response
}

#[tokio::test]
async fn cname_filters_cached_answers_after_updates_and_honours_exceptions() -> TestResult {
    let directory = TestDirectory::new()?;
    let (mut config, _default) = TestServer::config(&directory).await?;
    let upstream = Upstream::start(cname_answer, cname_answer).await?;
    config.dns.upstreams = vec![upstream.address];
    let path = directory.0.join("block.txt");
    tokio::fs::write(&path, "unrelated-block.test\n").await?;
    config.filtering.blocklists = vec![path.clone()];
    config.filtering.update_interval = Duration::from_secs(1);
    let (mut server, address) = bind(config).await?;
    let request = query("alias.test.")?;
    assert_eq!(
        udp(address, &request).await?.0.response_code,
        ResponseCode::NoError
    );
    assert_eq!(server.control().stats().await.forwarded, 1);
    tokio::fs::write(
        &path,
        "unrelated-block.test\n||tracker.test^$dnstype=CNAME\n",
    )
    .await?;
    wait_for_code(address, "alias.test.", ResponseCode::NXDomain).await?;
    assert_eq!(server.control().stats().await.forwarded, 1);
    assert!(server.control().stats().await.cname_blocked > 0);
    tokio::fs::write(&path, "tracker.test\n@@||alias.test^\n").await?;
    wait_for_code(address, "alias.test.", ResponseCode::NoError).await?;
    tokio::fs::write(&path, "tracker.test\n@@||tracker.test^\n").await?;
    wait_for_code(address, "alias.test.", ResponseCode::NoError).await?;
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn cname_cycles_and_unordered_chains_are_bounded() -> TestResult {
    fn response(request: &Message) -> Message {
        let mut response = answer(request);
        let first = request.queries[0].name().clone();
        let middle = Name::from_ascii("middle.test.").unwrap();
        let target = if first == Name::from_ascii("cycle.test.").unwrap() {
            first.clone()
        } else {
            Name::from_ascii("blocked.test.").unwrap()
        };
        response.answers = vec![
            Record::from_rdata(middle.clone(), 60, RData::CNAME(CNAME(target))),
            Record::from_rdata(first, 60, RData::CNAME(CNAME(middle))),
        ];
        response
    }
    let directory = TestDirectory::new()?;
    let (mut config, _default) = TestServer::config(&directory).await?;
    let upstream = Upstream::start(response, response).await?;
    config.dns.upstreams = vec![upstream.address];
    let path = directory.0.join("block.txt");
    tokio::fs::write(&path, "blocked.test\n").await?;
    config.filtering.blocklists = vec![path];
    let (mut server, address) = bind(config).await?;
    for (name, code) in [
        ("cycle.test.", ResponseCode::ServFail),
        ("unordered.test.", ResponseCode::NXDomain),
    ] {
        assert_eq!(udp(address, &query(name)?).await?.0.response_code, code);
    }
    assert_eq!(server.cache_stats().await.entries, 0);
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn global_rules_type_exclusions_and_invalid_modifiers_are_precise() -> TestResult {
    let server = TestServer::local("$dnstype=AAAA,denyallow=safe.test,dnsrewrite=NOERROR;;\n||excluded.test^$dnstype=~A|~CNAME\n||mixed.test^$dnstype=~A|AAAA\n||invalid.test^$dnstype=AAAA,unknown\n||txt-comma.test^$dnsrewrite=NOERROR;TXT;hello\\,world\n", "", &[]).await?;
    for (name, kind, code, empty) in [
        (
            "global.test.",
            RecordType::AAAA,
            ResponseCode::NoError,
            true,
        ),
        ("safe.test.", RecordType::AAAA, ResponseCode::NoError, false),
        (
            "excluded.test.",
            RecordType::A,
            ResponseCode::NoError,
            false,
        ),
        (
            "excluded.test.",
            RecordType::MX,
            ResponseCode::NXDomain,
            true,
        ),
        ("mixed.test.", RecordType::MX, ResponseCode::NoError, false),
        (
            "invalid.test.",
            RecordType::MX,
            ResponseCode::NoError,
            false,
        ),
    ] {
        let mut request = query(name)?;
        request.queries[0].query_type = kind;
        let response = udp(server.address, &request).await?.0;
        assert_eq!(response.response_code, code, "{name} {kind}");
        assert_eq!(response.answers.is_empty(), empty, "{name} {kind}");
    }
    let mut request = query("txt-comma.test.")?;
    request.queries[0].query_type = RecordType::TXT;
    let response = udp(server.address, &request).await?.0;
    assert!(
        matches!(&response.answers[0].data, RData::TXT(txt) if txt.txt_data[0].as_ref() == b"hello,world")
    );
    server.stop().await
}

#[tokio::test]
async fn sixteen_cname_links_are_allowed_but_seventeen_are_rejected() -> TestResult {
    fn chain(request: &Message) -> Message {
        let mut response = answer(request);
        let count = if request.queries[0].name().to_ascii().starts_with("ok") {
            16
        } else {
            17
        };
        let mut current = request.queries[0].name().clone();
        response.answers.clear();
        for index in 0..count {
            let target = Name::from_ascii(format!("link{index}.test.")).unwrap();
            response.answers.push(Record::from_rdata(
                current,
                60,
                RData::CNAME(CNAME(target.clone())),
            ));
            current = target;
        }
        response
    }
    let directory = TestDirectory::new()?;
    let (mut config, _default) = TestServer::config(&directory).await?;
    let upstream = Upstream::start(chain, chain).await?;
    config.dns.upstreams = vec![upstream.address];
    let path = directory.0.join("block.txt");
    tokio::fs::write(&path, "sentinel.test\n").await?;
    config.filtering.blocklists = vec![path];
    let (mut server, address) = bind(config).await?;
    for (name, code) in [
        ("ok.test.", ResponseCode::NoError),
        ("too-long.test.", ResponseCode::ServFail),
    ] {
        assert_eq!(udp(address, &query(name)?).await?.0.response_code, code);
    }
    server.shutdown_gracefully().await?;
    let mut rules = String::new();
    for index in 0..17 {
        rules.push_str(&format!(
            "||local{index}.test^$dnsrewrite=local{}.test\n",
            index + 1
        ));
    }
    rules.push_str("||local17.test^$dnsrewrite=192.0.2.1\n");
    let local = TestServer::local(&rules, "", &[]).await?;
    assert_eq!(
        local.ask("local1.test.").await?.response_code,
        ResponseCode::NoError
    );
    assert_eq!(
        local.ask("local0.test.").await?.response_code,
        ResponseCode::ServFail
    );
    local.stop().await
}
