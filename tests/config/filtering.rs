use super::*;
use std::time::Duration;

#[test]
fn enables_classic_dns_lists_by_default_and_accepts_opt_out() -> Result<(), ConfigError> {
    let config = Config::from_toml(&with_rewrites(&[]))?;
    assert!(config.filtering.enabled);
    assert_eq!(config.filtering.sources.len(), 2);
    assert_eq!(config.filtering.sources[0].name, "adguard-dns");
    assert_eq!(config.filtering.sources[1].name, "peter-lowe");
    assert!(
        config
            .filtering
            .sources
            .iter()
            .all(|source| !source.allow_dnsrewrite)
    );
    assert_eq!(config.filtering.update_interval, Duration::from_secs(86400));
    let disabled = Config::from_toml(&format!(
        "{}\n[filtering]\nenabled = false\nsources = []",
        with_rewrites(&[])
    ))?;
    assert!(!disabled.filtering.enabled);
    assert!(disabled.filtering.sources.is_empty());
    Ok(())
}

#[test]
fn validates_filter_paths_exceptions_sources_and_resource_bounds() {
    for section in [
        "update_interval_secs = 0",
        "download_timeout_secs = 0",
        "max_list_bytes = 0",
        "update_interval_secs = 604801",
        "download_timeout_secs = 121",
        "max_list_bytes = 16777217",
        "cache_dir = ''",
        "blocklists = ['']",
        "allowlist_files = ['']",
        "allowlist = ['*.test']",
        "allowlist = ['']",
        "allowlist = ['é.test']",
        "unknown = 1",
        "sources = [{ name = '../bad', url = 'https://example.test/list' }]",
        "sources = [{ name = 'bad', url = 'file:///etc/hosts' }]",
        "sources = [{ name = 'bad', url = 'https://user:password@example.test/list' }]",
        "sources = [{ name = 'bad', url = 'https://example.test/list#fragment' }]",
        "sources = [{ name = 'same', url = 'https://a.test/' }, { name = 'same', url = 'https://b.test/' }]",
        "sources = [{ name = 'bad', url = 'relative/path' }]",
        "sources = [{ name = 'bad', url = 'http://example.test/list', allow_dnsrewrite = true }]",
        "sources = [{ name = 'bad', url = 'https://example.test/list', allow_dnsrewrite = 'yes' }]",
    ] {
        assert!(
            Config::from_toml(&format!("{}\n[filtering]\n{section}", with_rewrites(&[]))).is_err(),
            "{section}"
        );
    }
}

#[test]
fn allows_custom_local_and_remote_sources_and_normalizes_allowlist() -> Result<(), ConfigError> {
    let config = Config::from_toml(&format!(
        "{}\n[filtering]\nblocklists = ['block.txt']\nallowlist_files = ['allow.txt']\nallowlist = ['SAFE.Example', 'xn--bcher-kva.test.']\nsources = [{{ name = 'custom', url = 'http://127.0.0.1:8080/list.txt' }}]\n",
        with_rewrites(&[])
    ))?;
    assert_eq!(config.filtering.allowlist[0].to_ascii(), "safe.example.");
    assert_eq!(config.filtering.sources[0].name, "custom");
    assert!(!config.filtering.sources[0].allow_dnsrewrite);
    assert_eq!(
        config.filtering.blocklists[0].to_string_lossy(),
        "block.txt"
    );
    Ok(())
}

#[test]
fn trusts_only_explicit_https_sources() -> Result<(), ConfigError> {
    let config = Config::from_toml(&format!(
        "{}\n[filtering]\nsources = [{{name = 'trusted', url = 'https://example.test/list', allow_dnsrewrite = true}}, {{name = 'ordinary', url = 'https://other.test/list'}}]",
        with_rewrites(&[])
    ))?;
    assert!(config.filtering.sources[0].allow_dnsrewrite);
    assert!(!config.filtering.sources[1].allow_dnsrewrite);
    Ok(())
}
