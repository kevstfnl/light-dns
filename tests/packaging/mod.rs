//! Installation simulée : aucun paquet, compte, service ou binaire ARM exécuté.
use crate::support::{TestResult, process::TestDirectory};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output},
};

const MOCK: &str = r#"#!/bin/sh
set -eu
name=${0##*/}
printf '%s' "$name" >> "$FIXTURE_ROOT/calls"
printf ' %s' "$@" >> "$FIXTURE_ROOT/calls"
printf '\n' >> "$FIXTURE_ROOT/calls"
[ "$name" != "${FIXTURE_FAIL:-}" ] || exit 1
case "$name" in
    id)
        if [ "$*" = '-u' ]; then printf '%s\n' "${FIXTURE_UID:-0}"
        elif [ ! -f "$FIXTURE_ROOT/account" ]; then exit 1
        elif [ "$1" = '-u' ]; then echo 101
        else echo light-dns; fi ;;
    uname) printf '%s\n' "${FIXTURE_ARCH:-aarch64}" ;;
    adduser) touch "$FIXTURE_ROOT/account" ;;
    rc-service) exit "${FIXTURE_STATUS:-3}" ;;
    install)
        directory=false
        mode=0755
        while [ "$#" -gt 0 ]; do
            case "$1" in
                -o|-g) shift 2 ;;
                -m) mode=$2; shift 2 ;;
                -d) directory=true; shift ;;
                *) break ;;
            esac
        done
        if "$directory"; then exec /usr/bin/install -d -m "$mode" "$@"
        else exec /usr/bin/install -m "$mode" "$@"; fi ;;
    readelf)
        case "$1" in
            -h) printf 'Machine: %s\n' "${FIXTURE_MACHINE:-AArch64}" ;;
            -l) printf '%s\n' "${FIXTURE_INTERP:-}" ;;
            -d) printf '%s\n' "${FIXTURE_DYNAMIC:-}" ;;
        esac ;;
    git) echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa ;;
    cargo) printf '{"packages":[{"name":"light-dns","version":"%s"}]}\n' "$FIXTURE_VERSION" ;;
esac
"#;

struct Fixture {
    directory: TestDirectory,
    script: PathBuf,
}

impl Fixture {
    fn new(installer: bool) -> TestResult<Self> {
        let directory = TestDirectory::new()?;
        let root = &directory.0;
        let bin = root.join("bin");
        fs::create_dir(&bin)?;
        for name in [
            "id",
            "uname",
            "apk",
            "addgroup",
            "adduser",
            "setcap",
            "rc-service",
            "install",
            "readelf",
            "git",
            "cargo",
        ] {
            let path = bin.join(name);
            fs::write(&path, MOCK)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let archive = root.join("archive");
        fs::create_dir(&archive)?;
        for name in ["config.toml", "light-dns.initd"] {
            fs::copy(repo.join("packaging/alpine").join(name), archive.join(name))?;
        }
        fs::write(archive.join("light-dns"), b"binary fixture, never executed")?;
        let script = if installer {
            let mut script = fs::read_to_string(repo.join("packaging/alpine/install.sh"))?;
            for prefix in ["/usr/local", "/etc/", "/var/lib/", "/run/"] {
                script = script.replace(prefix, &format!("{}{prefix}", root.display()));
            }
            let path = archive.join("install.sh");
            fs::write(&path, script)?;
            fs::create_dir_all(root.join("etc/init.d"))?;
            fs::write(root.join("etc/alpine-release"), "fixture")?;
            fs::write(root.join("etc/group"), "root:x:0:\n")?;
            path
        } else {
            fs::create_dir_all(root.join("packaging/alpine"))?;
            for name in ["config.toml", "light-dns.initd", "install.sh"] {
                fs::copy(
                    repo.join("packaging/alpine").join(name),
                    root.join("packaging/alpine").join(name),
                )?;
            }
            fs::create_dir(root.join("docs"))?;
            fs::copy(repo.join("docs/alpine.md"), root.join("docs/alpine.md"))?;
            fs::copy(repo.join("LICENSE"), root.join("LICENSE"))?;
            let binary = root.join("target/aarch64-unknown-linux-musl/release/light-dns");
            fs::create_dir_all(binary.parent().ok_or("missing parent")?)?;
            fs::write(binary, b"ELF fixture, never executed")?;
            repo.join("scripts/package-release.sh")
        };
        Ok(Self { directory, script })
    }

    fn root(&self) -> &Path {
        &self.directory.0
    }

    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> TestResult<Output> {
        let mut paths = vec![self.root().join("bin")];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").ok_or("PATH missing")?,
        ));
        Ok(Command::new("sh")
            .arg(&self.script)
            .args(args)
            .current_dir(self.root())
            .env("PATH", std::env::join_paths(paths)?)
            .env("FIXTURE_ROOT", self.root())
            .env("FIXTURE_VERSION", env!("CARGO_PKG_VERSION"))
            .envs(env.iter().copied())
            .output()?)
    }

    fn calls(&self) -> TestResult<String> {
        Ok(fs::read_to_string(self.root().join("calls"))?)
    }
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fresh_install_and_reinstall_preserve_configuration() -> TestResult {
    let fixture = Fixture::new(true)?;
    success(fixture.run(&[], &[])?);
    let root = fixture.root();
    let binary = root.join("usr/local/bin/light-dns");
    assert_eq!(
        fs::read(&binary)?,
        fs::read(root.join("archive/light-dns"))?
    );
    assert_eq!(fs::metadata(&binary)?.permissions().mode() & 0o777, 0o755);
    let config = root.join("etc/light-dns/config.toml");
    fs::write(&config, "operator configuration")?;
    success(fixture.run(&[], &[])?);
    assert_eq!(fs::read_to_string(config)?, "operator configuration");
    assert_eq!(
        fs::metadata(root.join("run/light-dns"))?
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let calls = fixture.calls()?;
    assert_eq!(
        calls.lines().filter(|c| c.starts_with("setcap ")).count(),
        2
    );
    assert_eq!(
        calls.lines().filter(|c| c.starts_with("adduser ")).count(),
        1
    );
    assert!(
        !calls
            .lines()
            .any(|c| c.starts_with("rc-service ") && !c.ends_with(" status"))
    );
    Ok(())
}

#[test]
fn rejects_wrong_environment_before_installing_packages() -> TestResult {
    let fixture = Fixture::new(true)?;
    for env in [[("FIXTURE_UID", "1000")], [("FIXTURE_ARCH", "x86_64")]] {
        assert!(!fixture.run(&[], &env)?.status.success());
    }
    fs::remove_file(fixture.root().join("etc/alpine-release"))?;
    assert!(!fixture.run(&[], &[])?.status.success());
    assert!(!fixture.calls()?.lines().any(|c| c.starts_with("apk ")));
    Ok(())
}

#[test]
fn refuses_active_or_unknown_service_state() -> TestResult {
    let fixture = Fixture::new(true)?;
    fs::write(fixture.root().join("etc/init.d/light-dns"), "")?;
    for status in ["0", "1"] {
        assert!(
            !fixture
                .run(&[], &[("FIXTURE_STATUS", status)])?
                .status
                .success()
        );
    }
    assert!(!fixture.calls()?.lines().any(|c| c.starts_with("apk ")));
    Ok(())
}

#[test]
fn rejects_incomplete_archive_and_symlinks() -> TestResult {
    let fixture = Fixture::new(true)?;
    let config = fixture.root().join("archive/config.toml");
    fs::rename(&config, fixture.root().join("saved-config"))?;
    assert!(!fixture.run(&[], &[])?.status.success());
    symlink(fixture.root().join("saved-config"), &config)?;
    assert!(!fixture.run(&[], &[])?.status.success());
    fs::remove_file(&config)?;
    fs::rename(fixture.root().join("saved-config"), &config)?;
    fs::create_dir(fixture.root().join("foreign"))?;
    symlink(
        fixture.root().join("foreign"),
        fixture.root().join("etc/light-dns"),
    )?;
    assert!(!fixture.run(&[], &[])?.status.success());
    assert!(!fixture.calls()?.lines().any(|c| c.starts_with("apk ")));
    Ok(())
}

#[test]
fn capability_failure_preserves_binary_and_cleans_stage() -> TestResult {
    let fixture = Fixture::new(true)?;
    success(fixture.run(&[], &[])?);
    let binary = fixture.root().join("usr/local/bin/light-dns");
    fs::write(&binary, b"old")?;
    assert!(
        !fixture
            .run(&[], &[("FIXTURE_FAIL", "setcap")])?
            .status
            .success()
    );
    assert_eq!(fs::read(binary)?, b"old");
    assert_eq!(
        fs::read_dir(fixture.root().join("usr/local/bin"))?.count(),
        1
    );
    Ok(())
}

#[test]
fn package_failure_leaves_no_binary() -> TestResult {
    let fixture = Fixture::new(true)?;
    assert!(
        !fixture
            .run(&[], &[("FIXTURE_FAIL", "apk")])?
            .status
            .success()
    );
    assert!(!fixture.root().join("usr/local/bin/light-dns").exists());
    Ok(())
}

#[test]
fn archive_content_metadata_permissions_and_checksum() -> TestResult {
    let fixture = Fixture::new(false)?;
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    success(fixture.run(&[&tag], &[])?);
    let assets = fixture.root().join("target/release-assets");
    success(
        Command::new("sha256sum")
            .args(["-c", "SHA256SUMS"])
            .current_dir(&assets)
            .output()?,
    );
    let name = format!("light-dns-{tag}-aarch64-unknown-linux-musl");
    let archive = assets.join(format!("{name}.tar.gz"));
    let extracted = fixture.root().join("extracted");
    fs::create_dir(&extracted)?;
    success(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&extracted)
            .output()?,
    );
    let contents = extracted.join(name);
    let mut files = fs::read_dir(&contents)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    files.sort();
    assert_eq!(
        files,
        [
            "INSTALL.md",
            "LICENSE",
            "REVISION",
            "VERSION",
            "config.toml",
            "install.sh",
            "light-dns",
            "light-dns.initd"
        ]
        .map(std::ffi::OsString::from)
    );
    assert_eq!(
        fs::read_to_string(contents.join("VERSION"))?,
        format!("{tag}\n")
    );
    assert_eq!(
        fs::read_to_string(contents.join("REVISION"))?,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"
    );
    for file in ["light-dns", "install.sh", "light-dns.initd"] {
        assert_eq!(
            fs::metadata(contents.join(file))?.permissions().mode() & 0o777,
            0o755
        );
    }
    for file in ["config.toml", "INSTALL.md", "LICENSE"] {
        assert_eq!(
            fs::metadata(contents.join(file))?.permissions().mode() & 0o777,
            0o644
        );
    }
    fs::write(&archive, b"corrupted archive")?;
    assert!(
        !Command::new("sha256sum")
            .args(["-c", "SHA256SUMS"])
            .current_dir(&assets)
            .output()?
            .status
            .success()
    );
    Ok(())
}

#[test]
fn rejects_version_mismatch_wrong_architecture_or_dynamic_elf() -> TestResult {
    let fixture = Fixture::new(false)?;
    let tag = format!("v{}", env!("CARGO_PKG_VERSION"));
    assert!(!fixture.run(&["v9.9.9"], &[])?.status.success());
    assert!(
        !fixture
            .run(&[&tag], &[("FIXTURE_FAIL", "cargo")])?
            .status
            .success()
    );
    for env in [
        ("FIXTURE_MACHINE", "X86-64"),
        ("FIXTURE_INTERP", "INTERP"),
        ("FIXTURE_DYNAMIC", "(NEEDED) libc.so"),
        ("FIXTURE_FAIL", "readelf"),
    ] {
        assert!(!fixture.run(&[&tag], &[env])?.status.success());
    }
    assert!(!fixture.root().join("target/release-assets").exists());
    Ok(())
}
