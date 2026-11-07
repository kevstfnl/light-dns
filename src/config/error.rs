use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("impossible de lire config.toml : {0}")]
    Read(#[source] io::Error),
    #[error("TOML invalide : {0}")]
    Parse(#[from] toml::de::Error),
    #[error("champ {field} invalide : {reason}")]
    InvalidValue { field: String, reason: String },
}

pub(super) fn invalid(field: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::InvalidValue {
        field: field.to_owned(),
        reason: reason.into(),
    }
}
