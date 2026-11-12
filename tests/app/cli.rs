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

#[test]
fn version_and_lifecycle_usage_do_not_require_a_daemon() -> TestResult {
    let directory = TestDirectory::new()?;
    let binary = env!("CARGO_BIN_EXE_light-dns");
    let version = Command::new(binary)
        .current_dir(&directory.0)
        .arg("version")
        .output()?;
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout)?,
        format!("light-dns {}\n", env!("CARGO_PKG_VERSION"))
    );
    for args in [
        vec!["update", "--version"],
        vec!["update", "--check", "--version", "1.0.0"],
        vec!["uninstall", "--yes"],
        vec!["install", "extra"],
    ] {
        let output = Command::new(binary)
            .current_dir(&directory.0)
            .args(args)
            .output()?;
        assert!(!output.status.success());
        assert!(String::from_utf8(output.stderr)?.contains("commande inconnue"));
    }
    Ok(())
}
