use hickory_proto::rr::Name;
use light_dns::config::{Config, ConfigError};
use std::net::{IpAddr, SocketAddr};

mod cache;
mod filtering;
mod validation;

fn document(listen: &str, upstreams: &[&str], rewrites: &[(&str, &str)]) -> String {
    let upstreams = toml::Value::Array(
        upstreams
            .iter()
            .map(|value| toml::Value::String((*value).into()))
            .collect(),
    );
    let mut content = format!(
        "[dns]\nlisten = {}\nupstreams = {upstreams}\n\n[rewrites]\n",
        toml::Value::String(listen.into()),
    );
    for (name, ip) in rewrites {
        let rule = toml::Table::from_iter([((*name).into(), toml::Value::String((*ip).into()))]);
        content.push_str(
            &toml::to_string(&rule).expect("la règle de test doit pouvoir être sérialisée en TOML"),
        );
    }
    content
}

fn with_rewrites(rewrites: &[(&str, &str)]) -> String {
    document("127.0.0.1:1053", &["1.1.1.1:53"], rewrites)
}

fn assert_invalid(content: &str, expected_field: &str) {
    let result = Config::from_toml(content);
    assert!(
        matches!(
            &result,
            Err(ConfigError::InvalidValue { field, reason })
                if field == expected_field && !reason.is_empty()
        ),
        "champ attendu : {expected_field}, résultat : {result:?}",
    );
}

#[test]
fn provided_configuration_is_valid() -> Result<(), ConfigError> {
    Config::from_toml(include_str!("../../config.toml"))?;
    let docker = Config::from_toml(include_str!("../../docker/config.toml"))?;
    assert_eq!(docker.dns.listen.to_string(), "0.0.0.0:1053");
    assert_eq!(
        docker.filtering.cache_dir.to_string_lossy(),
        "/var/lib/light-dns/filters"
    );
    Ok(())
}

#[test]
fn parses_typed_ipv4_and_ipv6_values() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_toml(&document(
        "[::1]:1053",
        &["127.0.0.1:53", "[2001:db8::1]:5353"],
        &[("nas.lan", "192.168.1.20"), ("nas6.lan.", "fd00::20")],
    ))?;
    assert_eq!(config.dns.listen, "[::1]:1053".parse::<SocketAddr>()?);
    assert_eq!(
        config.dns.upstreams,
        [
            "127.0.0.1:53".parse::<SocketAddr>()?,
            "[2001:db8::1]:5353".parse::<SocketAddr>()?,
        ],
    );
    assert_eq!(
        config.rewrites.get(&Name::from_ascii("nas.lan.")?),
        Some(&"192.168.1.20".parse::<IpAddr>()?),
    );
    assert_eq!(
        config.rewrites.get(&Name::from_ascii("nas6.lan.")?),
        Some(&"fd00::20".parse::<IpAddr>()?),
    );
    Ok(())
}

#[test]
fn rejects_invalid_toml_schema_and_duplicate_keys() {
    for content in [
        "[dns",
        "[rewrites]",
        "[dns]\nlisten = '127.0.0.1:1053'\n[rewrites]",
        "[dns]\nupstreams = ['1.1.1.1:53']\n[rewrites]",
        "[dns]\nlisten = '127.0.0.1:1053'\nupstreams = ['1.1.1.1:53']",
        "[dns]\nlisten = 1053\nupstreams = ['1.1.1.1:53']\n[rewrites]",
        "[dns]\nlisten = '127.0.0.1:1053'\nupstreams = '1.1.1.1:53'\n[rewrites]",
        "[dns]\nlisten = '127.0.0.1:1053'\nupstreams = ['1.1.1.1:53']\n[rewrites]\n'nas.lan' = 1",
        &format!("unknown = true\n{}", with_rewrites(&[])),
        &with_rewrites(&[]).replace("[dns]", "[dns]\nunknown = true"),
        &with_rewrites(&[("nas.lan", "192.168.1.1"), ("nas.lan", "192.168.1.2")]),
    ] {
        let result = Config::from_toml(content);
        assert!(matches!(result, Err(ConfigError::Parse(_))), "{result:?}");
    }
}

#[test]
fn readme_configuration_is_valid() -> Result<(), ConfigError> {
    let readme = include_str!("../../README.md");
    let example = readme
        .split_once("```toml\n")
        .expect("README TOML example")
        .1
        .split_once("```")
        .expect("closed TOML example")
        .0;
    Config::from_toml(example)?;
    Ok(())
}
