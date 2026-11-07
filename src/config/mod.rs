mod cache;
mod error;
mod filtering;
mod validation;

pub use cache::CacheConfig;
use cache::RawCacheConfig;
pub use error::ConfigError;
use filtering::RawFilteringConfig;
pub use filtering::{FilterSource, FilteringConfig};

use self::{
    error::invalid,
    validation::{parse_endpoint, parse_rewrite_name, parse_upstreams},
};
use hickory_proto::rr::Name;
use serde::Deserialize;
use std::{
    collections::{HashMap, hash_map::Entry},
    fs,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

#[derive(Debug)]
pub struct Config {
    pub dns: DnsConfig,
    pub rewrites: HashMap<Name, IpAddr>,
    pub filtering: FilteringConfig,
    pub cache: CacheConfig,
}

#[derive(Debug)]
pub struct DnsConfig {
    pub listen: SocketAddr,
    pub upstreams: Vec<SocketAddr>,
    pub upstream_timeout: Duration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    dns: RawDnsConfig,
    rewrites: HashMap<String, String>,
    #[serde(default)]
    filtering: RawFilteringConfig,
    #[serde(default)]
    cache: RawCacheConfig,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDnsConfig {
    listen: String,
    upstreams: Vec<String>,
    #[serde(default = "default_upstream_timeout_ms")]
    upstream_timeout_ms: u64,
}

fn default_upstream_timeout_ms() -> u64 {
    2000
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let content = fs::read_to_string("config.toml").map_err(ConfigError::Read)?;
        Self::from_toml(&content)
    }

    pub fn from_toml(content: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig = toml::from_str(content)?;
        let listen = parse_endpoint(&raw.dns.listen, "dns.listen")?;
        if !(1..=60_000).contains(&raw.dns.upstream_timeout_ms) {
            return Err(invalid(
                "dns.upstream_timeout_ms",
                "le délai doit être compris entre 1 et 60000 ms",
            ));
        }

        let upstreams = parse_upstreams(&raw.dns.upstreams, listen)?;

        let mut rewrites = HashMap::with_capacity(raw.rewrites.len());
        for (pattern, value) in raw.rewrites {
            let field = format!("rewrites[{pattern:?}]");
            let name = parse_rewrite_name(&pattern, &field)?;
            let ip = value.parse::<IpAddr>().map_err(|error| {
                invalid(&field, format!("adresse IPv4 ou IPv6 attendue : {error}"))
            })?;
            match rewrites.entry(name) {
                Entry::Vacant(entry) => {
                    entry.insert(ip);
                }
                Entry::Occupied(entry) => {
                    return Err(invalid(
                        &field,
                        format!("règle dupliquée après normalisation : {}", entry.key()),
                    ));
                }
            }
        }

        Ok(Self {
            dns: DnsConfig {
                listen,
                upstreams,
                upstream_timeout: Duration::from_millis(raw.dns.upstream_timeout_ms),
            },
            rewrites,
            filtering: raw.filtering.validate()?,
            cache: raw.cache.validate()?,
        })
    }
}
