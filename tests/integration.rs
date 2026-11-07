#![forbid(unsafe_code)]

mod app;
mod config;
mod dns;
mod filtering;
#[cfg(target_os = "linux")]
mod performance;
mod support;
