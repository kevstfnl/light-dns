use super::{assert_invalid, document, with_rewrites};
use hickory_proto::rr::Name;
use light_dns::config::{Config, ConfigError};
use std::time::Duration;

#[test]
fn alpine_release_config_uses_local_port_53_and_persistent_filters() -> Result<(), ConfigError> {
    let config = Config::from_toml(include_str!("../../packaging/alpine/config.toml"))?;
    assert_eq!(config.dns.listen.to_string(), "127.0.0.1:53");
    assert_eq!(config.dns.upstreams.len(), 2);
    assert!(config.rewrites.is_empty());
    assert_eq!(
        config.filtering.cache_dir.to_string_lossy(),
        "/var/lib/light-dns/filters"
    );
    Ok(())
}

#[test]
fn accepts_unspecified_listeners_and_empty_rewrites() -> Result<(), ConfigError> {
    for listen in ["0.0.0.0:1053", "[::]:1053"] {
        let config = Config::from_toml(&document(listen, &["127.0.0.1:53"], &[]))?;
        assert!(config.rewrites.is_empty());
    }
    Ok(())
}

#[test]
fn accepts_the_maximum_port() -> Result<(), ConfigError> {
    Config::from_toml(&document("127.0.0.1:65535", &["[::1]:65535"], &[]))?;
    Ok(())
}

#[test]
fn normalizes_exact_wildcard_and_punycode_names() -> Result<(), ConfigError> {
    let config = Config::from_toml(&with_rewrites(&[
        ("NAS.LAN", "192.168.1.20"),
        ("*.LAN.", "192.168.1.10"),
        ("XN--BCHER-KVA.LAN", "192.168.1.30"),
        ("_service._tcp.LAN", "192.168.1.40"),
        ("router", "192.168.1.1"),
    ]))?;
    let mut names: Vec<_> = config.rewrites.keys().map(Name::to_ascii).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "*.lan.",
            "_service._tcp.lan.",
            "nas.lan.",
            "router.",
            "xn--bcher-kva.lan.",
        ],
    );
    Ok(())
}

#[test]
fn accepts_unspecified_rewrite_destinations() -> Result<(), ConfigError> {
    Config::from_toml(&with_rewrites(&[
        ("zero.lan", "0.0.0.0"),
        ("zero6.lan", "::"),
    ]))?;
    Ok(())
}

#[test]
fn rejects_invalid_listeners() {
    for listen in [
        "",
        "localhost:53",
        "127.0.0.1",
        "256.0.0.1:53",
        "::1:53",
        "[gggg::1]:53",
        "127.0.0.1:65536",
        "127.0.0.1:0",
        "[::1]:0",
        " 127.0.0.1:53",
    ] {
        assert_invalid(&document(listen, &["1.1.1.1:53"], &[]), "dns.listen");
    }
}

#[test]
fn rejects_missing_upstreams() {
    assert_invalid(&document("127.0.0.1:1053", &[], &[]), "dns.upstreams");
}

#[test]
fn validates_upstream_timeout_and_preserves_default() -> Result<(), ConfigError> {
    assert_eq!(
        Config::from_toml(&with_rewrites(&[]))?.dns.upstream_timeout,
        Duration::from_secs(2)
    );
    for value in [1, 250, 60_000] {
        let content =
            with_rewrites(&[]).replace("[dns]", &format!("[dns]\nupstream_timeout_ms = {value}"));
        assert_eq!(
            Config::from_toml(&content)?.dns.upstream_timeout,
            Duration::from_millis(value)
        );
    }
    for value in [0, 60_001] {
        let content =
            with_rewrites(&[]).replace("[dns]", &format!("[dns]\nupstream_timeout_ms = {value}"));
        assert_invalid(&content, "dns.upstream_timeout_ms");
    }
    Ok(())
}

#[test]
fn rejects_obvious_forwarding_loops() {
    for (listen, upstream) in [
        ("127.0.0.1:1053", "127.0.0.1:1053"),
        ("[::1]:1053", "[::1]:1053"),
        ("0.0.0.0:1053", "127.0.0.1:1053"),
        ("[::]:1053", "[::1]:1053"),
        ("127.0.0.1:1053", "[::ffff:127.0.0.1]:1053"),
    ] {
        assert_invalid(&document(listen, &[upstream], &[]), "dns.upstreams[0]");
    }
}

#[test]
fn rejects_invalid_upstreams_with_their_index() {
    for upstream in [
        "",
        "dns.example:53",
        "1.1.1.1",
        "1.1.1.1:0",
        "1.1.1.1:65536",
        "::1:53",
        "0.0.0.0:53",
        "[::]:53",
        "224.0.0.1:53",
        "239.255.255.255:53",
        "[ff02::1]:53",
        "255.255.255.255:53",
        "[::ffff:0.0.0.0]:53",
        "[::ffff:224.0.0.1]:53",
        "[::ffff:255.255.255.255]:53",
    ] {
        assert_invalid(
            &document("127.0.0.1:1053", &["1.1.1.1:53", upstream], &[]),
            "dns.upstreams[1]",
        );
    }
}

#[test]
fn rejects_invalid_rewrite_destinations() {
    for ip in ["", "nas.lan", "256.1.1.1", "[::1]", "127.0.0.1:53", " ::1"] {
        assert_invalid(&with_rewrites(&[("nas.lan", ip)]), "rewrites[\"nas.lan\"]");
    }
}

#[test]
fn rejects_malformed_names() {
    for name in [
        "",
        ".",
        ".lan",
        "nas..lan",
        "nas.lan..",
        "-nas.lan",
        "nas lan",
        "nas.lan\n",
        "nas\0.lan",
        "café.lan",
        "nas/lan",
        "nas@lan",
        r"nas\.lan",
        r"nas\056lan",
        r"nas.lan\",
    ] {
        assert_invalid(
            &with_rewrites(&[(name, "192.168.1.1")]),
            &format!("rewrites[{name:?}]"),
        );
    }
}

#[test]
fn rejects_malformed_wildcards() {
    for name in [
        "*", "*.", "*..", "*..lan", "*nas.lan", "nas.*", "nas*.lan", "*.*.lan",
    ] {
        assert_invalid(
            &with_rewrites(&[(name, "192.168.1.1")]),
            &format!("rewrites[{name:?}]"),
        );
    }
}

#[test]
fn rejects_duplicates_after_normalization() {
    for (first, second) in [
        ("nas.lan", "NAS.LAN"),
        ("nas.lan", "nas.lan."),
        ("*.lan", "*.LAN."),
    ] {
        let content = with_rewrites(&[(first, "192.168.1.1"), (second, "192.168.1.2")]);
        let error = Config::from_toml(&content);
        assert!(
            matches!(&error, Err(ConfigError::InvalidValue { reason, .. })
                if reason.contains("dupliquée")),
            "{error:?}",
        );
    }
}

#[test]
fn enforces_label_and_wire_name_length_limits() -> Result<(), ConfigError> {
    let valid = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    for name in [&valid, &format!("{valid}.")] {
        Config::from_toml(&with_rewrites(&[(name, "192.168.1.1")]))?;
    }
    for name in [format!("{}.lan", "a".repeat(64)), format!("{valid}e")] {
        assert_invalid(
            &with_rewrites(&[(&name, "192.168.1.1")]),
            &format!("rewrites[{name:?}]"),
        );
    }
    Ok(())
}

#[test]
fn includes_the_wildcard_in_the_wire_length_limit() -> Result<(), ConfigError> {
    let valid = format!(
        "*.{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(59)
    );
    Config::from_toml(&with_rewrites(&[(&valid, "192.168.1.1")]))?;
    let too_long = format!("{valid}e");
    assert_invalid(
        &with_rewrites(&[(&too_long, "192.168.1.1")]),
        &format!("rewrites[{too_long:?}]"),
    );
    Ok(())
}
