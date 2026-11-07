use super::{ConfigError, error::invalid};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub enabled: bool,
    pub max_entries: u64,
    pub max_memory_bytes: u64,
    pub max_ttl: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: 4096,
            max_memory_bytes: 8 * 1024 * 1024,
            max_ttl: Duration::from_secs(86400),
        }
    }
}

impl CacheConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RawCacheConfig {
    enabled: bool,
    max_entries: u64,
    max_memory_bytes: u64,
    max_ttl_secs: u64,
}

impl Default for RawCacheConfig {
    fn default() -> Self {
        let config = CacheConfig::default();
        Self {
            enabled: config.enabled,
            max_entries: config.max_entries,
            max_memory_bytes: config.max_memory_bytes,
            max_ttl_secs: config.max_ttl.as_secs(),
        }
    }
}

impl RawCacheConfig {
    pub(super) fn validate(self) -> Result<CacheConfig, ConfigError> {
        for (field, valid) in [
            ("max_entries", (1..=65536).contains(&self.max_entries)),
            (
                "max_memory_bytes",
                (65536..=1024 * 1024 * 1024).contains(&self.max_memory_bytes),
            ),
            ("max_ttl_secs", (1..=604800).contains(&self.max_ttl_secs)),
        ] {
            if !valid {
                return Err(invalid(
                    &format!("cache.{field}"),
                    "valeur hors des limites autorisées",
                ));
            }
        }
        Ok(CacheConfig {
            enabled: self.enabled,
            max_entries: self.max_entries,
            max_memory_bytes: self.max_memory_bytes,
            max_ttl: Duration::from_secs(self.max_ttl_secs),
        })
    }
}
