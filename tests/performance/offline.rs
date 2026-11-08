//! Linux microbenchmarks: no sockets, no allocation counter or DNS latency claims.
use std::{
    fmt::Write,
    fs,
    hint::black_box,
    io,
    path::PathBuf,
    time::{Duration, Instant},
};
pub(crate) type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(crate) fn output(name: &str, csv: &str) -> Result {
    let directory = std::env::var_os("LIGHT_DNS_OFFLINE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/benchmarks/latency-pass"));
    fs::create_dir_all(&directory)?;
    fs::write(directory.join(format!("{name}.csv")), csv)?;
    println!("{csv}");
    Ok(())
}

pub(crate) fn usage() -> Result<(u64, u64, u64)> {
    let stat = fs::read_to_string("/proc/self/stat")?;
    let tail = stat
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("invalid stat"))?
        .1;
    let fields: Vec<_> = tail.split_whitespace().collect();
    let ticks = fields[11].parse::<u64>()? + fields[12].parse::<u64>()?;
    let status = fs::read_to_string("/proc/self/status")?;
    let field = |name: &str| -> Result<u64> {
        Ok(status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|value| value.split_whitespace().next())
            .ok_or_else(|| io::Error::other("missing status field"))?
            .parse()?)
    };
    Ok((ticks, field("VmRSS:")?, field("VmHWM:")?))
}

pub(crate) const HEADER: &str = "case,variant,trial,operations,elapsed_ms,p50_ns,p95_ns,p99_ns,cpu_ns_per_op,rss_before_kib,rss_after_kib,process_hwm_kib\n";

pub(crate) fn measure<T>(
    csv: &mut String,
    case: &str,
    variant: &str,
    trial: usize,
    mut operation: impl FnMut() -> T,
) -> Result {
    if cfg!(debug_assertions) {
        return Err(io::Error::other("use --release").into());
    }
    let ticks = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()?;
    if !ticks.status.success() {
        return Err(io::Error::other("getconf failed").into());
    }
    let ticks: f64 = std::str::from_utf8(&ticks.stdout)?.trim().parse()?;
    if ticks <= 0.0 {
        return Err(io::Error::other("invalid CLK_TCK").into());
    }
    for _ in 0..100 {
        black_box(operation());
    }
    // Fixed preallocated timing storage; percentiles describe the final batch of 1000 operations.
    let mut latencies = vec![0u128; 1000];
    let before = usage()?;
    let start = Instant::now();
    let mut operations = 0u64;
    loop {
        for latency in &mut latencies {
            let begin = Instant::now();
            black_box(operation());
            *latency = begin.elapsed().as_nanos();
        }
        operations += latencies.len() as u64;
        if start.elapsed() >= Duration::from_millis(250) {
            break;
        }
    }
    let elapsed = start.elapsed();
    let after = usage()?;
    latencies.sort_unstable();
    writeln!(
        csv,
        "{case},{variant},{trial},{operations},{:.3},{},{},{},{:.2},{},{},{}",
        elapsed.as_secs_f64() * 1000.0,
        latencies[500],
        latencies[950],
        latencies[990],
        after.0.saturating_sub(before.0) as f64 * 1e9 / ticks / operations as f64,
        before.1,
        after.1,
        after.2
    )?;
    Ok(())
}
