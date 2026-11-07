use crate::support::process::TestDirectory;
use std::fs;

mod cli;
#[cfg(unix)]
mod control;
#[cfg(unix)]
mod shutdown;

#[test]
fn startup_reports_configuration_errors() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::new()?;

    let output = directory.run()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("impossible de lire config.toml"));

    fs::write(directory.0.join("config.toml"), "[dns")?;
    let output = directory.run()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("TOML invalide"));

    let example = include_str!("../../config.toml");
    fs::write(
        directory.0.join("config.toml"),
        example.replace("127.0.0.1:1053", "127.0.0.1:0"),
    )?;
    let output = directory.run()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)?.contains("champ dns.listen invalide"));

    Ok(())
}
