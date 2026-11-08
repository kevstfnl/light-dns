//! Serveur DNS léger : configuration, transports et cycle de vie du service.
#![forbid(unsafe_code)]

pub mod app;
pub mod config;
pub mod dns;
mod filtering;

#[cfg(all(test, target_os = "linux"))]
#[path = "../tests/performance/offline.rs"]
mod offline;
