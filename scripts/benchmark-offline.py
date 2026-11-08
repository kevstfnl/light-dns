#!/usr/bin/env python3
"""Reproduce test-only experiments; never treat these as DNS network measurements."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SUITES = {
    "response": "dns::server::response::offline::response_serialization",
    "cache": "dns::cache::offline::cache_storage",
    "filtering": "filtering::index::offline::filtering_costs",
    "rewrites": "dns::rewrites::offline::rewrite_lookup",
}


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def command(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/benchmarks/latency-pass")
    parser.add_argument("--suite", choices=SUITES, action="append")
    parser.add_argument("--cache-variant", choices=["paired", "current", "parsed"], default="paired")
    parser.add_argument("--cache-process-pairs", type=int, default=0,
                        help="Run N alternating current/parsed cache pairs in separate processes")
    args = parser.parse_args()
    if not 0 <= args.cache_process_pairs <= 20:
        parser.error("--cache-process-pairs must be between 0 and 20")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if args.cache_process_pairs:
        rows = []
        for trial in range(args.cache_process_pairs):
            variants = ["current", "parsed"] if trial % 2 == 0 else ["parsed", "current"]
            for variant in variants:
                directory = output / f"pair-{trial}-{variant}"
                subprocess.run([os.sys.executable, str(Path(__file__).resolve()), "--suite", "cache",
                                "--cache-variant", variant, "--output", str(directory)], check=True)
                with (directory / f"cache-capacity-{variant}.csv").open() as source:
                    for row in csv.DictReader(source):
                        rows.append({"trial": trial, **row})
        with (output / "cache-process-pairs.csv").open("w") as destination:
            writer = csv.DictWriter(destination, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)
        return
    environment = os.environ.copy()
    environment["LIGHT_DNS_OFFLINE_DIR"] = str(output)
    environment["LIGHT_DNS_CACHE_VARIANT"] = "" if args.cache_variant == "paired" else args.cache_variant
    suites = args.suite or list(SUITES)
    # Compile everything before measurement. Never run this alongside other builds/benchmarks.
    subprocess.run(["cargo", "build", "--release", "--locked"], cwd=ROOT, check=True)
    subprocess.run(["cargo", "test", "--release", "--locked", "--lib", "--no-run"], cwd=ROOT, check=True)
    binary = ROOT / "target/release/light-dns"
    shutil.copy2(binary, output / "reference-light-dns")
    paths = sorted(list((ROOT / "src").rglob("*.rs")) + list((ROOT / "tests").rglob("*.rs")))
    manifest = {
        "kind": "offline_microbenchmarks_not_dns_latency",
        "revision": command("git", "rev-parse", "HEAD"),
        "git_status": command("git", "status", "--short"),
        "rustc": command("rustc", "-Vv"), "cargo": command("cargo", "-V"),
        "platform": platform.platform(), "machine": platform.machine(),
        "cpu": Path("/proc/cpuinfo").read_text().split("model name", 1)[-1].splitlines()[0].strip(),
        "lock_sha256": digest(ROOT / "Cargo.lock"), "binary_sha256": digest(binary),
        "sources_sha256": {str(path.relative_to(ROOT)): digest(path) for path in paths},
        "script_sha256": digest(Path(__file__).resolve()),
        "profile": "release", "cache_variant": args.cache_variant,
        "rustflags": environment.get("RUSTFLAGS", ""),
        "limitations": ["No network, allocation counter, ARM64 or Alpine validation",
                         "CPU and RSS include the test harness; RSS paired runs include both caches",
                         "Percentiles use final 1000 operations, CPU uses all operations over >=250ms",
                         "Concurrent cache operation is a batch, not individual DNS response latency"],
        "suites": {},
    }
    manifest_path = output / "manifest.json"
    for suite in suites:
        manifest["suites"][suite] = "running"
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
        print(f"Measuring {suite} ({args.cache_variant})", flush=True)
        with (output / f"{suite}.log").open("w") as log:
            result = subprocess.run(["cargo", "test", "--release", "--locked", "--lib", SUITES[suite],
                                     "--", "--exact", "--ignored", "--nocapture", "--test-threads=1"],
                                    cwd=ROOT, env=environment, stdout=log, stderr=subprocess.STDOUT)
        manifest["suites"][suite] = "completed" if result.returncode == 0 else "FAILED"
        manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
        if result.returncode:
            raise SystemExit(f"{suite} failed; inspect {output / (suite + '.log')}")
    groups = {}
    # Summarize only CSV files produced by the selected suites in this invocation.
    names = {"response": "response", "cache": f"cache-{args.cache_variant}",
             "filtering": "filtering-offline", "rewrites": "rewrites-offline"}
    for suite in suites:
        with (output / f"{names[suite]}.csv").open() as source:
            for row in csv.DictReader(source):
                groups.setdefault((suite, row["case"], row["variant"]), []).append(row)
    fields = ["p50_ns", "p95_ns", "p99_ns", "cpu_ns_per_op", "rss_after_kib", "process_hwm_kib"]
    with (output / "summary.csv").open("w") as destination:
        writer = csv.writer(destination)
        writer.writerow(["suite", "case", "variant", "trials"] + [f"median_{f}" for f in fields]
                        + ["min_p95_ns", "max_p95_ns", "decision"])
        for key, rows in groups.items():
            writer.writerow(list(key) + [len(rows)] + [statistics.median(float(r[f]) for r in rows) for f in fields]
                            + [min(float(r["p95_ns"]) for r in rows), max(float(r["p95_ns"]) for r in rows),
                               "inconclusive_without_network_validation"])
    print(f"Results: {output}")


if __name__ == "__main__":
    main()
