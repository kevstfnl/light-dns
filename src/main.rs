#![forbid(unsafe_code)]

use std::{io::IsTerminal, process::ExitCode};

#[tokio::main]
async fn main() -> ExitCode {
    match light_dns::app::run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let color = std::io::stderr().is_terminal()
                && std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").is_ok_and(|term| term != "dumb")
                && !std::env::args_os().any(|arg| arg == "--no-color");
            let label = if color {
                "\x1b[1;31mErreur light-dns\x1b[0m"
            } else {
                "Erreur light-dns"
            };
            eprintln!("{label} : {error}");
            ExitCode::FAILURE
        }
    }
}
