use crate::support::TestResult;
use std::{path::Path, process::Command};

#[test]
#[ignore = "Docker AMD64 et binaire x86_64-unknown-linux-musl requis ; exécuté par la CI de release"]
fn alpine_openrc_runs_nonroot_with_only_bind_capability_and_stops() -> TestResult {
    assert_eq!(std::env::consts::ARCH, "x86_64", "aucun test ARM autorisé");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(
        root.join("target/x86_64-unknown-linux-musl/release/light-dns")
            .is_file()
    );
    let mount = format!("{}:/project:ro", root.display());
    let script = r#"
set -eu
apk add --no-cache openrc libcap-setcap
mkdir -p /run/openrc /etc/light-dns /usr/local/bin
touch /run/openrc/softlevel
addgroup -S light-dns
adduser -S -D -H -h /var/lib/light-dns -s /sbin/nologin -G light-dns light-dns
cp /project/target/x86_64-unknown-linux-musl/release/light-dns /usr/local/bin/light-dns
cp /project/packaging/alpine/light-dns.initd /etc/init.d/light-dns
chmod 755 /etc/init.d/light-dns
cp /project/packaging/alpine/config.toml /etc/light-dns/config.toml
sed -i 's/enabled = true/enabled = false/' /etc/light-dns/config.toml
setcap cap_net_bind_service=+ep /usr/local/bin/light-dns
rc-update add light-dns default
rc-service -D light-dns start
trap 'rc-service -D light-dns stop >/dev/null 2>&1 || true' 0
for attempt in 1 2 3 4 5; do
    if light-dns --socket /run/light-dns/control.sock health; then break; fi
    sleep 1
done
light-dns --socket /run/light-dns/control.sock health
rc-service -D light-dns status
pid=$(pidof light-dns)
awk '/^Uid:/ { if ($2 == 0 || $3 == 0) exit 1; found=1 } END { if (!found) exit 1 }' "/proc/$pid/status"
grep -q '^CapEff:.*0000000000000400$' "/proc/$pid/status"
su -s /bin/sh -c 'light-dns --socket /run/light-dns/control.sock health' light-dns
if su -s /bin/sh -c 'light-dns install' light-dns; then exit 1; fi
rc-service -D light-dns reload
rc-service -D light-dns restart
light-dns --socket /run/light-dns/control.sock health
rc-service -D light-dns stop
if rc-service -D light-dns status; then exit 1; else [ "$?" -eq 3 ]; fi
trap - 0
"#;
    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--platform",
            "linux/amd64",
            "--sysctl",
            "net.ipv4.ip_unprivileged_port_start=1024",
            "-v",
            &mount,
            "alpine:3.23@sha256:469c9ea410b9215eaee7976ba6c0639039b444be8e5d638f7abbada189971eb2",
            "sh",
            "-ec",
            script,
        ])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
