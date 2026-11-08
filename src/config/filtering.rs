use super::{ConfigError, error::invalid, validation::parse_rewrite_name};
use hickory_proto::rr::Name;
use serde::Deserialize;
use std::{collections::HashSet, path::PathBuf, time::Duration};

#[derive(Debug, Clone)]
pub struct FilterSource {
    pub name: String,
    pub url: String,
    /// Autorise la famille `$dnsrewrite` pour cette source HTTPS uniquement.
    pub allow_dnsrewrite: bool,
}

#[derive(Debug, Clone)]
pub struct FilteringConfig {
    pub enabled: bool,
    pub blocklists: Vec<PathBuf>,
    pub allowlist_files: Vec<PathBuf>,
    pub allowlist: Vec<Name>,
    pub sources: Vec<FilterSource>,
    pub cache_dir: PathBuf,
    pub update_interval: Duration,
    pub download_timeout: Duration,
    pub max_list_bytes: usize,
}

impl Default for FilteringConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            blocklists: Vec::new(),
            allowlist_files: Vec::new(),
            allowlist: Vec::new(),
            sources: vec![
                FilterSource { name: "adguard-dns".into(), url: "https://adguardteam.github.io/HostlistsRegistry/assets/filter_1.txt".into(), allow_dnsrewrite: false },
                FilterSource { name: "peter-lowe".into(), url: "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=hosts&showintro=0&mimetype=plaintext".into(), allow_dnsrewrite: false },
            ],
            cache_dir: "data/filters".into(),
            update_interval: Duration::from_secs(86400),
            download_timeout: Duration::from_secs(15),
            max_list_bytes: 8 * 1024 * 1024,
        }
    }
}

impl FilteringConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct RawFilteringConfig {
    enabled: bool,
    blocklists: Vec<PathBuf>,
    allowlist_files: Vec<PathBuf>,
    allowlist: Vec<String>,
    sources: Vec<RawSource>,
    cache_dir: PathBuf,
    update_interval_secs: u64,
    download_timeout_secs: u64,
    max_list_bytes: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSource {
    name: String,
    url: String,
    #[serde(default)]
    allow_dnsrewrite: bool,
}

impl Default for RawFilteringConfig {
    fn default() -> Self {
        let defaults = FilteringConfig::default();
        Self {
            enabled: defaults.enabled,
            blocklists: defaults.blocklists,
            allowlist_files: defaults.allowlist_files,
            allowlist: Vec::new(),
            sources: defaults
                .sources
                .into_iter()
                .map(|source| RawSource {
                    name: source.name,
                    url: source.url,
                    allow_dnsrewrite: source.allow_dnsrewrite,
                })
                .collect(),
            cache_dir: defaults.cache_dir,
            update_interval_secs: defaults.update_interval.as_secs(),
            download_timeout_secs: defaults.download_timeout.as_secs(),
            max_list_bytes: defaults.max_list_bytes,
        }
    }
}

impl RawFilteringConfig {
    pub(super) fn validate(self) -> Result<FilteringConfig, ConfigError> {
        for (field, valid) in [
            (
                "update_interval_secs",
                (1..=604800).contains(&self.update_interval_secs),
            ),
            (
                "download_timeout_secs",
                (1..=120).contains(&self.download_timeout_secs),
            ),
            (
                "max_list_bytes",
                (1024..=16 * 1024 * 1024).contains(&self.max_list_bytes),
            ),
            ("sources", self.sources.len() <= 32),
            (
                "blocklists",
                self.blocklists.len() + self.allowlist_files.len() <= 32,
            ),
            ("allowlist", self.allowlist.len() <= 10000),
        ] {
            if !valid {
                return Err(invalid(
                    &format!("filtering.{field}"),
                    "valeur hors des limites autorisées",
                ));
            }
        }
        if self.cache_dir.as_os_str().is_empty()
            || self
                .blocklists
                .iter()
                .chain(&self.allowlist_files)
                .any(|path| path.as_os_str().is_empty())
        {
            return Err(invalid(
                "filtering",
                "les chemins ne peuvent pas être vides",
            ));
        }
        let mut names = HashSet::new();
        let sources = self
            .sources
            .into_iter()
            .enumerate()
            .map(|(index, source)| {
                let field = format!("filtering.sources[{index}]");
                if source.name.is_empty()
                    || source.name.len() > 64
                    || !source
                        .name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                    || !names.insert(source.name.clone())
                {
                    return Err(invalid(
                        &field,
                        "nom unique requis : 1 à 64 lettres ASCII, chiffres, tirets ou underscores",
                    ));
                }
                let url = reqwest::Url::parse(&source.url)
                    .map_err(|error| invalid(&field, error.to_string()))?;
                if !matches!(url.scheme(), "https" | "http")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.fragment().is_some()
                    || source.url.len() > 2048
                {
                    return Err(invalid(
                        &field,
                        "URL HTTP(S) absolue requise, sans identifiants ni fragment",
                    ));
                }
                if source.allow_dnsrewrite && url.scheme() != "https" {
                    return Err(invalid(&field, "allow_dnsrewrite exige une URL HTTPS"));
                }
                Ok(FilterSource {
                    name: source.name,
                    url: url.to_string(),
                    allow_dnsrewrite: source.allow_dnsrewrite,
                })
            })
            .collect::<Result<_, _>>()?;
        let allowlist = self
            .allowlist
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let field = format!("filtering.allowlist[{index}]");
                if value.contains('*') {
                    return Err(invalid(
                        &field,
                        "indiquer un domaine sans wildcard ; ses sous-domaines sont déjà inclus",
                    ));
                }
                parse_rewrite_name(value, &field)
            })
            .collect::<Result<_, _>>()?;
        Ok(FilteringConfig {
            enabled: self.enabled,
            blocklists: self.blocklists,
            allowlist_files: self.allowlist_files,
            allowlist,
            sources,
            cache_dir: self.cache_dir,
            update_interval: Duration::from_secs(self.update_interval_secs),
            download_timeout: Duration::from_secs(self.download_timeout_secs),
            max_list_bytes: self.max_list_bytes,
        })
    }
}
