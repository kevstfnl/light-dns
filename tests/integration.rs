#![forbid(unsafe_code)]

mod app;
mod config;
#[cfg(target_os = "linux")]
mod deployment;
mod dns;
mod filtering;
#[cfg(target_os = "linux")]
mod packaging;
#[cfg(target_os = "linux")]
mod performance;
mod support;
