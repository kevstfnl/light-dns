"""Installation simulée : aucun compte, paquet, service ou binaire ARM exécuté."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest


REPO = Path(__file__).resolve().parents[2]
VERSION = tomllib.loads((REPO / "Cargo.toml").read_text())["package"]["version"]
MOCK = r'''
import json, os, pathlib, subprocess, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
root = pathlib.Path(os.environ["FIXTURE_ROOT"])
with (root / "calls").open("a") as log:
    log.write(json.dumps([name, *args]) + "\n")
if name == "id":
    if args == ["-u"]:
        print(os.environ.get("FIXTURE_UID", "0"))
    elif not (root / "account").exists():
        sys.exit(1)
    else:
        print("101" if args[0] == "-u" else "light-dns")
elif name == "uname":
    print(os.environ.get("FIXTURE_ARCH", "aarch64"))
elif name == "adduser":
    (root / "account").touch()
elif name == "rc-service":
    sys.exit(int(os.environ.get("FIXTURE_STATUS", "3")))
elif name == "install":
    clean = []
    i = 0
    while i < len(args):
        if args[i] in ("-o", "-g"):
            i += 2
        else:
            clean.append(args[i])
            i += 1
    sys.exit(subprocess.call(["/usr/bin/install", *clean]))
elif name == "readelf":
    if args[0] == "-h":
        print("Machine: " + os.environ.get("FIXTURE_MACHINE", "AArch64"))
    elif args[0] == "-l":
        print(os.environ.get("FIXTURE_INTERP", ""))
    else:
        print(os.environ.get("FIXTURE_DYNAMIC", ""))
elif name == "git":
    print("a" * 40)
elif name == os.environ.get("FIXTURE_FAIL"):
    sys.exit(1)
'''


class Fixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="light-dns-packaging-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("id", "uname", "apk", "addgroup", "adduser", "setcap",
                     "rc-service", "install", "readelf", "git"):
            command = self.bin / name
            command.write_text(f"#!{sys.executable}\n" + MOCK)
            command.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                        FIXTURE_ROOT=str(self.root))

    def calls(self):
        path = self.root / "calls"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []


class InstallerTests(Fixture):
    def setUp(self):
        super().setUp()
        self.archive = self.root / "archive"
        self.archive.mkdir()
        for name in ("config.toml", "light-dns.initd"):
            shutil.copy(REPO / "packaging/alpine" / name, self.archive / name)
        (self.archive / "light-dns").write_bytes(b"new binary fixture, never executed")
        # Remapper seulement les chemins absolus dans une copie temporaire.
        script = (REPO / "packaging/alpine/install.sh").read_text()
        for prefix in ("/usr/local", "/etc/", "/var/lib/", "/run/"):
            script = script.replace(prefix, str(self.root) + prefix)
        self.script = self.archive / "install.sh"
        self.script.write_text(script)
        (self.root / "etc/init.d").mkdir(parents=True)
        (self.root / "etc/alpine-release").write_text("fixture")
        (self.root / "etc/group").write_text("root:x:0:\n")

    def run_installer(self, **env):
        return subprocess.run(["sh", str(self.script)], env=dict(self.env, **env),
                              capture_output=True, text=True, check=False)

    def test_fresh_install_and_reinstall_preserve_configuration(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        binary = self.root / "usr/local/bin/light-dns"
        self.assertEqual(binary.read_bytes(), (self.archive / "light-dns").read_bytes())
        self.assertEqual(binary.stat().st_mode & 0o777, 0o755)
        config = self.root / "etc/light-dns/config.toml"
        config.write_text("operator configuration")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(config.read_text(), "operator configuration")
        self.assertEqual((self.root / "run/light-dns").stat().st_mode & 0o777, 0o700)
        self.assertEqual(len([c for c in self.calls() if c[0] == "setcap"]), 2)
        self.assertEqual(len([c for c in self.calls() if c[0] == "adduser"]), 1)
        self.assertFalse(any(c[0] == "rc-service" and c[-1] != "status" for c in self.calls()))

    def test_rejects_non_root_wrong_architecture_and_non_alpine(self):
        for env in ({"FIXTURE_UID": "1000"}, {"FIXTURE_ARCH": "x86_64"}):
            with self.subTest(env=env):
                self.assertNotEqual(self.run_installer(**env).returncode, 0)
        (self.root / "etc/alpine-release").unlink()
        self.assertNotEqual(self.run_installer().returncode, 0)
        self.assertFalse(any(c[0] == "apk" for c in self.calls()))

    def test_refuses_active_or_unknown_service_state(self):
        (self.root / "etc/init.d/light-dns").touch()
        for status in ("0", "1"):
            with self.subTest(status=status):
                self.assertNotEqual(self.run_installer(FIXTURE_STATUS=status).returncode, 0)
        self.assertFalse(any(c[0] == "apk" for c in self.calls()))

    def test_missing_archive_file_or_symlink_is_rejected(self):
        config = self.archive / "config.toml"
        config.unlink()
        self.assertNotEqual(self.run_installer().returncode, 0)
        config.symlink_to(REPO / "packaging/alpine/config.toml")
        self.assertNotEqual(self.run_installer().returncode, 0)

    def test_capability_failure_preserves_old_binary_and_cleans_stage(self):
        self.assertEqual(self.run_installer().returncode, 0)
        binary = self.root / "usr/local/bin/light-dns"
        binary.write_bytes(b"old")
        result = self.run_installer(FIXTURE_FAIL="setcap")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(binary.read_bytes(), b"old")
        self.assertEqual(list(binary.parent.glob(".light-dns.*")), [])

    def test_package_failure_leaves_no_binary(self):
        self.assertNotEqual(self.run_installer(FIXTURE_FAIL="apk").returncode, 0)
        self.assertFalse((self.root / "usr/local/bin/light-dns").exists())


class PackageTests(Fixture):
    def setUp(self):
        super().setUp()
        for name in ("Cargo.toml", "LICENSE"):
            shutil.copy(REPO / name, self.root / name)
        shutil.copytree(REPO / "packaging", self.root / "packaging")
        (self.root / "docs").mkdir()
        shutil.copy(REPO / "docs/alpine.md", self.root / "docs/alpine.md")
        self.binary = self.root / "target/aarch64-unknown-linux-musl/release/light-dns"
        self.binary.parent.mkdir(parents=True)
        self.binary.write_bytes(b"ELF fixture, never executed")

    def package(self, tag=None, **env):
        tag = tag or f"v{VERSION}"
        return subprocess.run(["sh", str(REPO / "scripts/package-release.sh"), tag],
                              cwd=self.root, env=dict(self.env, **env),
                              capture_output=True, text=True, check=False)

    def test_archive_content_and_checksum(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        assets = self.root / "target/release-assets"
        archive = next(assets.glob("*.tar.gz"))
        digest, filename = (assets / "SHA256SUMS").read_text().split()
        self.assertEqual(filename, archive.name)
        self.assertEqual(digest, hashlib.sha256(archive.read_bytes()).hexdigest())
        with tarfile.open(archive) as tar:
            files = {Path(m.name).name for m in tar.getmembers() if m.isfile()}
            self.assertEqual(files, {"light-dns", "install.sh", "light-dns.initd", "config.toml",
                                     "LICENSE", "INSTALL.md", "VERSION", "REVISION"})

    def test_rejects_version_mismatch_and_non_static_or_wrong_architecture(self):
        self.assertNotEqual(self.package("v9.9.9").returncode, 0)
        for env in ({"FIXTURE_MACHINE": "X86-64"}, {"FIXTURE_INTERP": "INTERP"},
                    {"FIXTURE_DYNAMIC": "(NEEDED) libc.so"}):
            with self.subTest(env=env):
                self.assertNotEqual(self.package(**env).returncode, 0)
        self.assertFalse((self.root / "target/release-assets").exists())


if __name__ == "__main__":
    unittest.main()
