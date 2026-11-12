//! Installation simulée : aucun paquet, compte, service ou binaire ARM exécuté.
use crate::support::{TestResult, process::TestDirectory};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
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
            for name in [
                "config.toml",
                "light-dns.initd",
                "install.sh",
                "bootstrap.sh",
            ] {
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
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
    // Reproduce the manual guide: all downloaded assets coexist in the same directory.
    // The raw binary has the same basename as the archive directory, so extraction needs a subdirectory.
    let extracted = assets.join("extracted");
    fs::create_dir(&extracted)?;
    assert!(assets.join(&name).is_file());
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

#[test]
fn bootstrap_verifies_downloads_rejects_corruption_and_cleans_up() -> TestResult {
    let fixture = Fixture::new(false)?;
    let root = fixture.root();
    let binary = root.join("downloaded-binary");
    fs::write(
        &binary,
        format!(
            "#!/bin/sh\nprintf '%s' \"$*\" > '{}/installed'\n",
            root.display()
        ),
    )?;
    let hash = Command::new("sha256sum").arg(&binary).output()?;
    assert!(hash.status.success());
    let hash = String::from_utf8(hash.stdout)?;
    let digest = hash.split_whitespace().next().ok_or("missing digest")?;
    let name = format!(
        "light-dns-v{}-aarch64-unknown-linux-musl",
        env!("CARGO_PKG_VERSION")
    );
    fs::write(root.join("downloaded-sums"), format!("{digest}  {name}\n"))?;
    let wget = root.join("bin/wget");
    fs::write(
        &wget,
        format!(
            r#"#!/bin/sh
set -eu
while [ "$1" != -O ]; do shift; done
destination=$2
if [ "$destination" = SHA256SUMS ]; then cp '{}/downloaded-sums' "$destination"
else cp '{}/downloaded-binary' "$destination"; fi
"#,
            root.display(),
            root.display()
        ),
    )?;
    fs::set_permissions(wget, fs::Permissions::from_mode(0o755))?;
    let script = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/alpine/bootstrap.sh"),
    )?
    .replace(
        "/etc/alpine-release",
        &format!("{}/alpine-release", root.display()),
    )
    .replace(
        "/tmp/light-dns-bootstrap.",
        &format!("{}/light-dns-bootstrap.", root.display()),
    );
    fs::write(root.join("alpine-release"), "fixture")?;
    let bootstrap = root.join("bootstrap.sh");
    fs::write(&bootstrap, script)?;
    let paths = std::env::join_paths([
        root.join("bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ])?;
    let run = || {
        Command::new("sh")
            .arg(&bootstrap)
            .env("PATH", &paths)
            .env("FIXTURE_ROOT", root)
            .output()
    };
    success(run()?);
    assert_eq!(fs::read_to_string(root.join("installed"))?, "install");
    fs::remove_file(root.join("installed"))?;
    fs::write(&binary, "corrupted")?;
    assert!(!run()?.status.success());
    assert!(!root.join("installed").exists());
    assert!(!fs::read_dir(root)?.filter_map(Result::ok).any(|e| {
        e.file_name()
            .to_string_lossy()
            .starts_with("light-dns-bootstrap.")
    }));
    Ok(())
}
