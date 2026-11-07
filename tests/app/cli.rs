use crate::support::{TestResult, process::TestDirectory};
use std::process::Command;

#[test]
fn help_usage_errors_and_unavailable_daemon_are_reported_without_config() -> TestResult {
    let directory = TestDirectory::new()?;
    for args in [vec!["--help"], vec!["--config", "missing.toml", "--help"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(&directory.0)
            .args(args)
            .output()?;
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout)?;
        for word in ["cache clear", "reload", "stats"] {
            assert!(text.contains(word));
        }
    }
    for args in [
        vec!["cache"],
        vec!["--unknown"],
        vec!["--config"],
        vec!["--socket"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(&directory.0)
            .args(args)
            .output()?;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
    }
    #[cfg(unix)]
    {
        let output = Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(&directory.0)
            .arg("stats")
            .output()?;
        assert!(!output.status.success());
        assert!(!String::from_utf8(output.stderr)?.contains("config.toml"));
    }
    Ok(())
}
