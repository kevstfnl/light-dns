use serde::Deserialize;
use std::{collections::HashMap, error::Error, fs};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub dns: DnsConfig,
    pub rewrites: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct DnsConfig {
    pub listen: String,
    pub upstreams: Vec<String>,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn Error>> {
        let content = fs::read_to_string("config.toml")?;
        let config = toml::from_str(&content)?;
        Ok(config)
    }
}
