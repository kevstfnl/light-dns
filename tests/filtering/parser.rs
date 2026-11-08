use super::*;
use hickory_proto::{op::Edns, rr::RecordType};
use tokio::{
    net::{TcpStream, UdpSocket},
    time::timeout,
};

#[tokio::test]
async fn imports_domains_hosts_and_dns_adblock_rules() -> TestResult {
    let server = TestServer::local("\u{feff}! test\n# comment\n[Adblock Plus 2.0]\nADS.TEST\n0.0.0.0 hosts.test other.test # comment $var | wildcard*\n:: ipv6.test\n127.0.0.1 loopback.test localhost localhost.localdomain\n||anchored.test^\n", "", &[]).await?;
    for domain in [
        "ads.test.",
        "sub.ADS.TEST.",
        "hosts.test.",
        "other.test.",
        "ipv6.test.",
        "loopback.test.",
        "anchored.test.",
    ] {
        assert_eq!(
            server.ask(domain).await?.response_code,
            ResponseCode::NXDomain,
            "{domain}"
        );
    }
    for domain in [
        "badads.test.",
        "hosts.test.example.",
        "localhost.",
        "localhost.localdomain.",
    ] {
        assert_eq!(server.ask(domain).await?.answers.len(), 1, "{domain}");
    }
    server.stop().await
}

#[tokio::test]
async fn applies_exceptions_important_badfilter_and_user_allowlists() -> TestResult {
    let server = TestServer::local("||ads.test^\n@@||allowed.ads.test^\n||critical.test^$important\n@@||critical.test^\n@@||forced.critical.test^$important\n||disabled.test^\n||disabled.test^$badfilter\n||disabled-important.test^$important\n||disabled-important.test^$badfilter,important\n||user.test^$important\n||file.test^$important\n", "file.test\n", &["user.test."]).await?;
    for domain in ["ads.test.", "critical.test.", "sub.critical.test."] {
        assert_eq!(
            server.ask(domain).await?.response_code,
            ResponseCode::NXDomain
        );
    }
    for domain in [
        "allowed.ads.test.",
        "forced.critical.test.",
        "disabled.test.",
        "disabled-important.test.",
        "user.test.",
        "sub.user.test.",
        "file.test.",
    ] {
        assert_eq!(server.ask(domain).await?.answers.len(), 1, "{domain}");
    }
    server.stop().await
}

#[tokio::test]
async fn ignores_web_filters_unsupported_options_and_nonblocking_hosts() -> TestResult {
    let server = TestServer::local("sentinel.test\nexample.test##.ad\nexample.test#@#.ad\nexample.test#%#script\n||conditional.test^$third-party\n||typed.test^$dnstype=AAAA\n||redirect.test^$redirect=noopjs\n/regex/\nhttps://url.test/ad.js\n192.168.1.1 local.test\n0.0.0.0\ncom\n*.wild.test\n!#if env_firefox\nconditional-body.test\n!#else\nconditional-else.test\n!#endif\n!#if\tenv_firefox\nconditional-tab.test\n!#endif\n!#include other.txt\n", "", &[]).await?;
    for domain in [
        "example.test.",
        "conditional.test.",
        "typed.test.",
        "redirect.test.",
        "url.test.",
        "local.test.",
        "safe.com.",
        "conditional-body.test.",
        "conditional-else.test.",
        "conditional-tab.test.",
    ] {
        assert_eq!(server.ask(domain).await?.answers.len(), 1, "{domain}");
    }
    assert_eq!(
        server.ask("a.wild.test.").await?.response_code,
        ResponseCode::NXDomain
    );
    assert_eq!(
        server.ask("sentinel.test.").await?.response_code,
        ResponseCode::NXDomain
    );
    server.stop().await
}

#[tokio::test]
async fn blocked_udp_and_tcp_answers_are_local_for_all_query_types() -> TestResult {
    let directory = TestDirectory::new()?;
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    let silent = UdpSocket::bind("127.0.0.1:0").await?;
    config.dns.upstreams = vec![silent.local_addr()?];
    let path = directory.0.join("block.txt");
    tokio::fs::write(&path, "ads.test\n").await?;
    config.filtering.blocklists = vec![path];
    let (mut server, address) = bind(config).await?;
    let mut tcp = TcpStream::connect(address).await?;
    for kind in [
        RecordType::A,
        RecordType::AAAA,
        RecordType::HTTPS,
        RecordType::MX,
        RecordType::ANY,
    ] {
        let mut request = query("ADS.test.")?;
        request.queries[0].query_type = kind;
        request.metadata.recursion_desired = false;
        request.metadata.authentic_data = true;
        request.edns = Some(Edns::new());
        let udp_response = udp(address, &request).await?.0;
        write_tcp(&mut tcp, &request).await?;
        let tcp_response = timeout(Duration::from_secs(3), read_tcp(&mut tcp)).await??;
        for response in [udp_response, tcp_response] {
            assert_eq!(response.id, request.id);
            assert_eq!(response.queries, request.queries);
            assert_eq!(response.response_code, ResponseCode::NXDomain);
            assert!(response.answers.is_empty());
            assert!(
                !response.authoritative && !response.authentic_data && !response.recursion_desired
            );
            assert!(response.recursion_available && response.edns.is_some());
        }
    }
    let mut bytes = [0; 512];
    assert!(
        matches!(silent.try_recv_from(&mut bytes), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn rewrites_precede_blocklists_and_allowlists() -> TestResult {
    let directory = TestDirectory::new()?;
    let (mut config, _upstream) = TestServer::config(&directory).await?;
    let path = directory.0.join("block.txt");
    tokio::fs::write(&path, "||lan.test^$important\n").await?;
    config.filtering.blocklists = vec![path];
    config.filtering.allowlist = vec![Name::from_ascii("allowed.lan.test.")?];
    config
        .rewrites
        .insert(Name::from_ascii("*.lan.test.")?, "192.168.1.10".parse()?);
    let (mut server, address) = bind(config).await?;
    for name in ["host.lan.test.", "allowed.lan.test."] {
        let response = udp(address, &query(name)?).await?.0;
        assert_eq!(response.answers.len(), 1);
        assert_eq!(response.answers[0].ttl, 60);
    }
    server.shutdown_gracefully().await?;
    Ok(())
}

#[tokio::test]
async fn reloads_local_files_and_keeps_the_index_if_the_new_list_is_invalid() -> TestResult {
    let server = TestServer::local("first.test\n", "", &[]).await?;
    assert_eq!(
        server.ask("first.test.").await?.response_code,
        ResponseCode::NXDomain
    );
    let path = server.directory.0.join("block.txt");
    tokio::fs::write(&path, "second.test\n").await?;
    wait_for_code(server.address, "second.test.", ResponseCode::NXDomain).await?;
    wait_for_code(server.address, "first.test.", ResponseCode::NoError).await?;
    tokio::fs::write(&path, "<!doctype html>\n<html>invalid</html>").await?;
    sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        server.ask("second.test.").await?.response_code,
        ResponseCode::NXDomain
    );
    // Une nouvelle version valide doit encore pouvoir remplacer le dernier index.
    tokio::fs::write(&path, "third.test\n").await?;
    wait_for_code(server.address, "third.test.", ResponseCode::NXDomain).await?;
    wait_for_code(server.address, "second.test.", ResponseCode::NoError).await?;
    server.stop().await
}

#[tokio::test]
async fn badfilter_applies_across_lists_independently_of_load_order() -> TestResult {
    let directory = TestDirectory::new()?;
    let block = directory.0.join("block.txt");
    let disable = directory.0.join("disable.txt");
    tokio::fs::write(&block, "||ADS.test^\n").await?;
    tokio::fs::write(&disable, "||ads.test^$badfilter\n").await?;
    for paths in [
        vec![block.clone(), disable.clone()],
        vec![disable.clone(), block.clone()],
    ] {
        let (mut config, _upstream) = TestServer::config(&directory).await?;
        config.filtering.blocklists = paths;
        let (mut server, address) = bind(config).await?;
        assert_eq!(udp(address, &query("ads.test.")?).await?.0.answers.len(), 1);
        server.shutdown_gracefully().await?;
    }
    Ok(())
}
