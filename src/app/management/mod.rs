//! Alpine lifecycle operations. Never called by the DNS request path.
mod releases;

use super::{cli::Command, control};
use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command as Process, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const BIN: &str = "/usr/local/bin/light-dns";
const INIT: &str = "/etc/init.d/light-dns";
const CONFIG: &str = "/etc/light-dns/config.toml";
const STATE: &str = "/etc/light-dns/install-state.toml";
const TX: &str = "/etc/light-dns/.transaction";
const SOCKET: &str = "/run/light-dns/control.sock";
const INIT_TEXT: &str = include_str!("../../../packaging/alpine/light-dns.initd");
const CONFIG_TEXT: &str = include_str!("../../../packaging/alpine/config.toml");

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    user: bool,
    group: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    binary: bool,
    init: bool,
    running: bool,
    enable: bool,
}

struct System {
    root: PathBuf,
    // Only unit tests construct a non-root filesystem or override tools.
    tools: Option<PathBuf>,
}

impl System {
    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    fn output(&self, name: &str, args: &[&str]) -> Result<Output> {
        let program = self
            .tools
            .as_ref()
            .map_or_else(|| PathBuf::from(name), |p| p.join(name));
        let mut child = Process::new(program)
            .args(args)
            .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if child.try_wait()?.is_some() {
                return Ok(child.wait_with_output()?);
            }
            if Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                return Err(format!("Délai dépassé : {name} {}", args.join(" ")).into());
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn command(&self, name: &str, args: &[&str]) -> Result<()> {
        let output = self.output(name, args)?;
        if !output.status.success() {
            return Err(format!("Échec : {name} {} ({})", args.join(" "), output.status).into());
        }
        Ok(())
    }

    fn service(&self, action: &str) -> Result<()> {
        self.command("rc-service", &["light-dns", action])
    }

    fn running(&self) -> Result<bool> {
        if !self.path(INIT).exists() {
            return Ok(false);
        }
        match self
            .output("rc-service", &["light-dns", "status"])?
            .status
            .code()
        {
            Some(0) => Ok(true),
            Some(3) => Ok(false),
            other => Err(format!("État OpenRC indéterminé : {other:?}").into()),
        }
    }

    fn check_path(&self, path: &Path) -> Result<()> {
        for ancestor in path.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(m) => {
                    if m.file_type().is_symlink() {
                        return Err(
                            format!("Lien symbolique refusé : {}", ancestor.display()).into()
                        );
                    }
                    if self.root == Path::new("/") && (m.uid() != 0 || m.mode() & 0o022 != 0) {
                        return Err(format!(
                            "Chemin non contrôlé par root : {}",
                            ancestor.display()
                        )
                        .into());
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    fn preflight(&self) -> Result<()> {
        if self.output("id", &["-u"])?.stdout != b"0\n" {
            return Err("Exécuter avec les privilèges root.".into());
        }
        if !self.path("/etc/alpine-release").is_file()
            || self.output("uname", &["-m"])?.stdout != b"aarch64\n"
        {
            return Err("Installation réservée à Alpine Linux AArch64 avec OpenRC.".into());
        }
        self.command("rc-service", &["--version"])?;
        for p in [
            BIN,
            INIT,
            CONFIG,
            STATE,
            TX,
            "/run/light-dns-management.lock",
        ] {
            self.check_path(&self.path(p))?;
        }
        // Service-owned directories are checked for links, but need not be root-owned.
        for p in [
            "/var/lib/light-dns",
            "/var/lib/light-dns/filters",
            "/run/light-dns",
        ] {
            let path = self.path(p);
            if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(format!("Lien symbolique refusé : {p}").into());
            }
            if let Some(parent) = path.parent()
                && p != "/var/lib/light-dns/filters"
            {
                self.check_path(parent)?;
            }
        }
        Ok(())
    }

    fn lock(&self) -> Result<File> {
        let path = self.path("/run/light-dns-management.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        lock.try_lock()
            .map_err(|e| format!("Une autre opération de gestion est active : {e}"))?;
        // Keep the inode: unlinking a lock allows two different lock owners.
        Ok(lock)
    }

    fn write(&self, path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
        let temporary = path.with_extension("light-dns-new");
        self.check_path(&temporary)?;
        // A prior power loss may leave this root-owned staging file behind.
        self.remove(&temporary)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)?;
        let result = (|| -> Result<()> {
            file.write_all(bytes)?;
            // The bootstrap uses umask 077; installed files still need explicit service access.
            file.set_permissions(fs::Permissions::from_mode(mode))?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            if let Some(parent) = path.parent() {
                File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }

    fn remove(&self, path: &Path) -> Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn account(&self) -> Result<()> {
        let path = self.path(STATE);
        let mut owned: Ownership = if path.exists() {
            toml::from_str(&fs::read_to_string(&path)?)?
        } else {
            Ownership::default()
        };
        if !self.output("id", &["-u", "light-dns"])?.status.success() {
            let groups = fs::read_to_string(self.path("/etc/group"))?;
            if !groups.lines().any(|line| line.starts_with("light-dns:")) {
                self.command("addgroup", &["-S", "light-dns"])?;
                owned.group = true;
                self.write(&path, toml::to_string(&owned)?.as_bytes(), 0o600)?;
            }
            self.command(
                "adduser",
                &[
                    "-S",
                    "-D",
                    "-H",
                    "-h",
                    "/var/lib/light-dns",
                    "-s",
                    "/sbin/nologin",
                    "-G",
                    "light-dns",
                    "light-dns",
                ],
            )?;
            owned.user = true;
            self.write(&path, toml::to_string(&owned)?.as_bytes(), 0o600)?;
        }
        let uid = self.output("id", &["-u", "light-dns"])?;
        if !uid.status.success()
            || String::from_utf8(uid.stdout)?
                .trim()
                .parse::<u32>()
                .map_or(true, |u| u == 0)
            || self.output("id", &["-gn", "light-dns"])?.stdout != b"light-dns\n"
        {
            return Err("Compte light-dns incompatible.".into());
        }
        // Existing accounts must also be system accounts with the expected home and shell.
        let passwd = fs::read_to_string(self.path("/etc/passwd"))?;
        let valid = passwd.lines().any(|line| {
            let p: Vec<_> = line.split(':').collect();
            p.len() == 7
                && p[0] == "light-dns"
                && p[2].parse::<u32>().is_ok_and(|u| u > 0 && u < 1000)
                && p[5] == "/var/lib/light-dns"
                && p[6] == "/sbin/nologin"
        });
        if !valid {
            return Err(
                "Compte light-dns existant incompatible (UID système, home ou shell).".into(),
            );
        }
        Ok(())
    }

    fn capability(&self, binary: &Path) -> Result<()> {
        let config = Config::load_from(&self.path(CONFIG))?;
        let threshold =
            fs::read_to_string(self.path("/proc/sys/net/ipv4/ip_unprivileged_port_start"))
                .ok()
                .and_then(|s| s.trim().parse::<u16>().ok())
                .unwrap_or(1024);
        if config.dns.listen.port() < threshold {
            if !self
                .output("sh", &["-c", "command -v setcap >/dev/null"])?
                .status
                .success()
            {
                self.command("apk", &["add", "--no-cache", "libcap-setcap"])?;
            }
            self.command(
                "setcap",
                &[
                    "cap_net_bind_service=+ep",
                    binary.to_str().ok_or("Chemin UTF-8 requis")?,
                ],
            )?;
        }
        Ok(())
    }

    async fn health(&self) -> Result<()> {
        if self.tools.is_some() {
            return self.command("health", &[]);
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match tokio::time::timeout(
                Duration::from_secs(2),
                control::request(self.path(SOCKET), Command::Health(false)),
            )
            .await
            {
                Ok(Ok(_)) => return Ok(()),
                _ if Instant::now() >= deadline => {
                    return Err("Contrôle de santé local expiré.".into());
                }
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }

    async fn recover(&self) -> Result<()> {
        let directory = self.path(TX);
        if !directory.exists() {
            return Ok(());
        }
        let journal = directory.join("journal.toml");
        if !journal.exists() {
            fs::remove_dir_all(directory)?; // Preparation only: no system files changed yet.
            return Ok(());
        }
        let j: Journal = toml::from_str(&fs::read_to_string(journal)?)?;
        if self.running()? {
            self.service("stop")?;
        }
        for (destination, saved, existed) in [(BIN, "binary", j.binary), (INIT, "init", j.init)] {
            let path = self.path(destination);
            if existed {
                self.write(&path, &fs::read(directory.join(saved))?, 0o755)?;
                if destination == BIN {
                    self.capability(&path)?;
                }
            } else {
                self.remove(&path)?;
            }
        }
        if j.enable {
            self.command("rc-update", &["del", "light-dns", "default"])?;
        }
        if j.running {
            self.service("start")?;
            self.health().await?;
        }
        fs::remove_dir_all(directory)?;
        eprintln!("Ancienne installation restaurée.");
        Ok(())
    }

    async fn install(&self, binary: &[u8], init: &[u8]) -> Result<()> {
        for p in ["/etc/light-dns", "/usr/local", "/usr/local/bin"] {
            let path = self.path(p);
            if !path.exists() {
                fs::create_dir_all(&path)?;
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
            }
        }
        self.recover().await?;
        if !self.path(CONFIG).exists() {
            self.write(&self.path(CONFIG), CONFIG_TEXT.as_bytes(), 0o644)?;
        }
        Config::load_from(&self.path(CONFIG))?;
        self.account()?;
        for (p, mode) in [
            ("/var/lib/light-dns", "0750"),
            ("/var/lib/light-dns/filters", "0700"),
            ("/run/light-dns", "0700"),
        ] {
            let path = self.path(p);
            fs::create_dir_all(&path)?;
            self.command(
                "chown",
                &[
                    "light-dns:light-dns",
                    path.to_str().ok_or("Chemin UTF-8 requis")?,
                ],
            )?;
            fs::set_permissions(
                path,
                fs::Permissions::from_mode(u32::from_str_radix(mode, 8)?),
            )?;
        }
        let first = !self.path(BIN).exists();
        let j = Journal {
            binary: !first,
            init: self.path(INIT).exists(),
            running: self.running()?,
            enable: first && !self.path("/etc/runlevels/default/light-dns").exists(),
        };
        let directory = self.path(TX);
        fs::create_dir(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        for (path, saved, exists) in [(BIN, "binary", j.binary), (INIT, "init", j.init)] {
            if exists {
                self.write(&directory.join(saved), &fs::read(self.path(path))?, 0o600)?;
            }
        }
        self.write(
            &directory.join("journal.toml"),
            toml::to_string(&j)?.as_bytes(),
            0o600,
        )?;
        let result = async {
            if j.running {
                self.service("stop")?;
            }
            self.write(&self.path(BIN), binary, 0o755)?;
            self.capability(&self.path(BIN))?;
            self.write(&self.path(INIT), init, 0o755)?;
            if j.enable {
                self.command("rc-update", &["add", "light-dns", "default"])?;
            }
            if first || j.running {
                self.service("start")?;
                self.health().await?;
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
        .await;
        if let Err(error) = result {
            if let Err(rollback) = self.recover().await {
                return Err(format!("Installation échouée : {error}. Rollback échoué : {rollback}. Sauvegardes conservées dans {TX}.").into());
            }
            return Err(error);
        }
        // Removing the journal commits the transaction. A leftover backup directory is harmless.
        self.remove(&directory.join("journal.toml"))?;
        File::open(&directory)?.sync_all()?;
        fs::remove_dir_all(directory)?;
        println!("Installation terminée ; configuration et données conservées.");
        Ok(())
    }

    async fn uninstall(&self, purge: bool) -> Result<()> {
        self.recover().await?;
        if self.running()? {
            self.service("stop")?;
        }
        let runlevels = self.path("/etc/runlevels");
        if runlevels.exists() {
            for entry in fs::read_dir(runlevels)? {
                let entry = entry?;
                if fs::symlink_metadata(entry.path().join("light-dns")).is_ok() {
                    self.command(
                        "rc-update",
                        &[
                            "del",
                            "light-dns",
                            entry.file_name().to_str().ok_or("Runlevel UTF-8 requis")?,
                        ],
                    )?;
                }
            }
        }
        self.remove(&self.path(INIT))?;
        // Runtime directories can contain live control locks only until the service has stopped.
        if self.path("/run/light-dns").exists() {
            fs::remove_dir_all(self.path("/run/light-dns"))?;
        }
        if purge {
            let owned: Ownership = if self.path(STATE).exists() {
                toml::from_str(&fs::read_to_string(self.path(STATE))?)?
            } else {
                Ownership::default()
            };
            if owned.user && self.output("id", &["-u", "light-dns"])?.status.success() {
                self.command("deluser", &["light-dns"])?;
            }
            if owned.group
                && fs::read_to_string(self.path("/etc/group"))?
                    .lines()
                    .any(|l| l.starts_with("light-dns:"))
            {
                self.command("delgroup", &["light-dns"])?;
            }
            for p in ["/var/lib/light-dns", "/etc/light-dns"] {
                if self.path(p).exists() {
                    fs::remove_dir_all(self.path(p))?;
                }
            }
        }
        self.remove(&self.path(BIN))?;
        println!(
            "Désinstallation terminée{}.",
            if purge {
                " avec purge"
            } else {
                " ; configuration, données et compte conservés"
            }
        );
        Ok(())
    }
}

fn confirm_purge() -> Result<()> {
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|_| "La purge exige une confirmation sur un terminal interactif.")?;
    tty.write_all(
        b"Supprimer configuration, donnees et compte cree par light-dns ? Taper PURGE : ",
    )?;
    tty.flush()?;
    let mut answer = String::new();
    io::BufReader::new(tty).read_line(&mut answer)?;
    if answer.trim() != "PURGE" {
        return Err("Purge annulée.".into());
    }
    Ok(())
}

pub(super) async fn run(command: Command) -> Result<()> {
    let system = System {
        root: PathBuf::from("/"),
        tools: None,
    };
    if let Command::Update { check: true, .. } = command {
        return releases::update(&system, true, None).await;
    }
    system.preflight()?;
    if matches!(command, Command::Uninstall(true)) {
        confirm_purge()?;
    }
    let _lock = system.lock()?;
    system.recover().await?;
    match command {
        Command::Install => {
            system
                .install(&fs::read(std::env::current_exe()?)?, INIT_TEXT.as_bytes())
                .await
        }
        Command::Update { check, version } => {
            releases::update(&system, check, version.as_deref()).await
        }
        Command::Uninstall(purge) => system.uninstall(purge).await,
        _ => Err("Commande de gestion invalide".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Fixture(System);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0.root);
        }
    }
    impl Fixture {
        fn new() -> Result<Self> {
            let directory = releases::test_directory()?;
            let root = directory;
            let tools = root.join("tools");
            for p in [
                "tools",
                "etc/init.d",
                "etc/light-dns",
                "run",
                "usr/local/bin",
                "etc/runlevels/default",
            ] {
                fs::create_dir_all(root.join(p))?;
            }
            fs::write(root.join("etc/group"), "root:x:0:\n")?;
            fs::write(root.join("etc/alpine-release"), "fixture")?;
            fs::write(root.join("etc/passwd"), "root:x:0:0:root:/root:/bin/sh\n")?;
            let fixture = Self(System {
                root,
                tools: Some(tools.clone()),
            });
            let script = format!("#!/bin/sh\nset -eu\nroot='{}'\n", fixture.0.root.display())
                + r#"
name=${0##*/}
printf '%s %s\n' "$name" "$*" >> "$root/calls"
case "$name" in
id)
    if [ "$*" = '-u' ]; then if [ -f "$root/uid" ]; then cat "$root/uid"; else echo 0; fi
    elif [ ! -f "$root/account" ]; then exit 1
    elif [ "$1" = '-u' ]; then echo 101
    else echo light-dns; fi ;;
uname) if [ -f "$root/arch" ]; then cat "$root/arch"; else echo aarch64; fi ;;
addgroup) echo 'light-dns:x:101:' >> "$root/etc/group" ;;
adduser)
    touch "$root/account"
    echo 'light-dns:x:101:101::/var/lib/light-dns:/sbin/nologin' >> "$root/etc/passwd" ;;
deluser) rm -f "$root/account" ;;
delgroup) echo 'root:x:0:' > "$root/etc/group" ;;
rc-service)
    [ "$1" != --version ] || exit 0
    case "$2" in
    status) [ -f "$root/running" ] && exit 0; exit 3 ;;
    start) touch "$root/running" ;;
    stop) rm -f "$root/running" ;;
    esac ;;
rc-update)
    if [ "$1" = add ]; then touch "$root/etc/runlevels/default/light-dns"
    else rm -f "$root/etc/runlevels/${3:-default}/light-dns"; fi ;;
health)
    [ ! -f "$root/fail-health-permanent" ] || exit 1
    if [ -f "$root/fail-health" ]; then rm "$root/fail-health"; exit 1; fi ;;
setcap)
    if [ -f "$root/fail-setcap" ]; then rm "$root/fail-setcap"; exit 1; fi ;;
esac
"#;
            for name in [
                "id",
                "uname",
                "addgroup",
                "adduser",
                "deluser",
                "delgroup",
                "rc-service",
                "rc-update",
                "health",
                "chown",
                "setcap",
                "sh",
            ] {
                let path = tools.join(name);
                fs::write(&path, &script)?;
                fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
            }
            Ok(fixture)
        }
        async fn install(&self, binary: &[u8]) -> Result<()> {
            self.0.install(binary, INIT_TEXT.as_bytes()).await
        }
    }

    #[tokio::test]
    async fn initial_and_repeated_install_preserve_config_and_data() -> Result<()> {
        let f = Fixture::new()?;
        f.install(b"old").await?;
        assert!(f.0.path("/running").exists());
        assert!(f.0.path("/etc/runlevels/default/light-dns").exists());
        let config = format!("{CONFIG_TEXT}\n# custom setting\n");
        fs::write(f.0.path(CONFIG), &config)?;
        fs::write(f.0.path("/var/lib/light-dns/filters/kept"), "data")?;
        f.install(b"new").await?;
        assert_eq!(fs::read_to_string(f.0.path(CONFIG))?, config);
        assert_eq!(fs::read(f.0.path(BIN))?, b"new");
        assert_eq!(fs::metadata(f.0.path(BIN))?.mode() & 0o777, 0o755);
        assert_eq!(fs::metadata(f.0.path(CONFIG))?.mode() & 0o777, 0o644);
        assert_eq!(
            fs::read_to_string(f.0.path("/var/lib/light-dns/filters/kept"))?,
            "data"
        );
        let calls = fs::read_to_string(f.0.path("/calls"))?;
        assert_eq!(
            calls.lines().filter(|s| s.starts_with("adduser ")).count(),
            1
        );
        assert!(calls.contains("chown light-dns:light-dns"));
        assert!(INIT_TEXT.contains("command_user=\"light-dns:light-dns\""));
        assert_eq!(
            fs::metadata(f.0.path("/var/lib/light-dns/filters"))?.mode() & 0o777,
            0o700
        );
        Ok(())
    }

    #[tokio::test]
    async fn stopped_service_stays_stopped_and_high_port_needs_no_capability() -> Result<()> {
        let f = Fixture::new()?;
        fs::write(
            f.0.path(CONFIG),
            CONFIG_TEXT.replace("127.0.0.1:53", "127.0.0.1:1053"),
        )?;
        f.install(b"old").await?;
        f.0.service("stop")?;
        f.install(b"new").await?;
        assert!(!f.0.path("/running").exists());
        assert!(!fs::read_to_string(f.0.path("/calls"))?.contains("setcap "));
        Ok(())
    }

    #[tokio::test]
    async fn health_or_capability_failure_restores_old_files_and_service() -> Result<()> {
        for marker in ["/fail-health", "/fail-setcap"] {
            let f = Fixture::new()?;
            f.install(b"old").await?;
            fs::write(f.0.path(marker), "")?;
            assert!(f.install(b"new").await.is_err());
            assert_eq!(fs::read(f.0.path(BIN))?, b"old");
            assert_eq!(fs::read(f.0.path(INIT))?, INIT_TEXT.as_bytes());
            assert!(f.0.path("/running").exists());
            assert!(!f.0.path(TX).exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn failed_first_start_removes_binary_and_activation_but_keeps_config() -> Result<()> {
        let f = Fixture::new()?;
        fs::write(f.0.path("/fail-health"), "")?;
        assert!(f.install(b"new").await.is_err());
        assert!(!f.0.path(BIN).exists());
        assert!(!f.0.path(INIT).exists());
        assert!(!f.0.path("/running").exists());
        assert!(!f.0.path("/etc/runlevels/default/light-dns").exists());
        assert!(f.0.path(CONFIG).exists());
        Ok(())
    }

    #[tokio::test]
    async fn interrupted_transaction_recovers_including_stale_staging_file() -> Result<()> {
        let f = Fixture::new()?;
        f.install(b"old").await?;
        let directory = f.0.path(TX);
        fs::create_dir(&directory)?;
        fs::write(directory.join("binary"), b"old")?;
        fs::write(directory.join("init"), INIT_TEXT)?;
        fs::write(
            directory.join("journal.toml"),
            toml::to_string(&Journal {
                binary: true,
                init: true,
                running: true,
                enable: false,
            })?,
        )?;
        fs::write(f.0.path(BIN), b"partial update")?;
        fs::write(
            f.0.path(BIN).with_extension("light-dns-new"),
            b"incomplete stage",
        )?;
        f.0.recover().await?;
        assert_eq!(fs::read(f.0.path(BIN))?, b"old");
        assert!(f.0.path("/running").exists());
        Ok(())
    }

    #[tokio::test]
    async fn failed_rollback_keeps_backups_until_recovery_succeeds() -> Result<()> {
        let f = Fixture::new()?;
        f.install(b"old").await?;
        fs::write(f.0.path("/fail-health-permanent"), "")?;
        let error = f
            .install(b"new")
            .await
            .err()
            .ok_or("rollback failure expected")?;
        assert!(error.to_string().contains("Rollback échoué"));
        assert!(f.0.path(TX).join("journal.toml").exists());
        assert_eq!(fs::read(f.0.path(TX).join("binary"))?, b"old");
        fs::remove_file(f.0.path("/fail-health-permanent"))?;
        f.0.recover().await?;
        assert!(!f.0.path(TX).exists());
        assert_eq!(fs::read(f.0.path(BIN))?, b"old");
        Ok(())
    }

    #[tokio::test]
    async fn uninstall_preserves_state_then_purge_removes_only_managed_paths() -> Result<()> {
        let f = Fixture::new()?;
        f.install(b"old").await?;
        f.0.uninstall(false).await?;
        assert!(!f.0.path(BIN).exists());
        assert!(!f.0.path(INIT).exists());
        assert!(f.0.path(CONFIG).exists());
        assert!(f.0.path("/account").exists());
        assert!(f.0.path("/var/lib/light-dns").exists());
        let foreign = f.0.path("/foreign");
        fs::write(&foreign, "keep")?;
        symlink(&foreign, f.0.path("/var/lib/light-dns/link"))?;
        f.0.uninstall(true).await?;
        assert_eq!(fs::read_to_string(foreign)?, "keep");
        assert!(!f.0.path(CONFIG).exists());
        assert!(!f.0.path("/account").exists());
        assert!(!fs::read_to_string(f.0.path("/calls"))?.contains("apk del"));
        Ok(())
    }

    #[tokio::test]
    async fn purge_does_not_remove_preexisting_accounts() -> Result<()> {
        let f = Fixture::new()?;
        f.install(b"old").await?;
        fs::write(f.0.path(STATE), toml::to_string(&Ownership::default())?)?;
        f.0.uninstall(true).await?;
        assert!(f.0.path("/account").exists());
        Ok(())
    }

    #[test]
    fn exclusive_lock_and_symlink_refusal() -> Result<()> {
        let f = Fixture::new()?;
        let first = f.0.lock()?;
        assert!(f.0.lock().is_err());
        drop(first);
        assert!(f.0.lock().is_ok());
        symlink("/tmp", f.0.path(BIN))?;
        assert!(f.0.check_path(&f.0.path(BIN)).is_err());
        Ok(())
    }

    #[test]
    fn rejects_non_root_wrong_arch_and_non_alpine_before_mutation() -> Result<()> {
        let f = Fixture::new()?;
        f.0.preflight()?;
        fs::write(f.0.path("/uid"), "1000\n")?;
        assert!(f.0.preflight().is_err());
        fs::remove_file(f.0.path("/uid"))?;
        fs::write(f.0.path("/arch"), "x86_64\n")?;
        assert!(f.0.preflight().is_err());
        fs::remove_file(f.0.path("/arch"))?;
        fs::remove_file(f.0.path("/etc/alpine-release"))?;
        assert!(f.0.preflight().is_err());
        assert!(!f.0.path("/account").exists());
        assert!(!f.0.path(BIN).exists());
        Ok(())
    }

    #[tokio::test]
    async fn replacing_and_removing_an_executing_inode_is_supported() -> Result<()> {
        let f = Fixture::new()?;
        f.install(&fs::read("/bin/sleep")?).await?;
        let mut process = Process::new(f.0.path(BIN)).arg("30").spawn()?;
        let result = async {
            f.install(b"replacement").await?;
            assert!(process.try_wait()?.is_none());
            f.0.uninstall(false).await?;
            assert!(process.try_wait()?.is_none());
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        }
        .await;
        let _ = process.kill();
        process.wait()?;
        result
    }
}
