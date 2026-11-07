use super::*;
use std::time::Duration;

#[test]
fn defaults_and_custom_cache_limits_are_validated() -> Result<(), ConfigError> {
    let config = Config::from_toml(&with_rewrites(&[]))?;
    assert!(config.cache.enabled);
    assert_eq!(config.cache.max_entries, 4096);
    assert_eq!(config.cache.max_memory_bytes, 8 * 1024 * 1024);
    let config = Config::from_toml(&format!(
        "{}\n[cache]\nenabled = false\nmax_entries = 12\nmax_memory_bytes = 65536\nmax_ttl_secs = 60\n",
        with_rewrites(&[])
    ))?;
    assert!(!config.cache.enabled);
    assert_eq!(config.cache.max_entries, 12);
    assert_eq!(config.cache.max_ttl, Duration::from_secs(60));
    for limits in [
        "max_entries = 0",
        "max_entries = 65537",
        "max_memory_bytes = 65535",
        "max_memory_bytes = 1073741825",
        "max_ttl_secs = 0",
        "max_ttl_secs = 604801",
        "unknown = true",
    ] {
        assert!(
            Config::from_toml(&format!("{}\n[cache]\n{limits}", with_rewrites(&[]))).is_err(),
            "{limits}"
        );
    }
    Ok(())
}
