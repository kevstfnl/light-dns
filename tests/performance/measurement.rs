use crate::support::TestResult;
use std::{fs, io, process::Command};

pub(super) struct Usage {
    pub cpu_ticks: u64,
    pub rss_kib: u64,
    pub high_water_kib: u64,
}

impl Usage {
    pub fn read() -> TestResult<Self> {
        Self::read_process("self")
    }

    pub fn read_process(pid: &str) -> TestResult<Self> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let tail = stat
            .rsplit_once(')')
            .ok_or_else(|| io::Error::other("stat /proc invalide"))?
            .1;
        let fields: Vec<_> = tail.split_whitespace().collect();
        let user: u64 = fields
            .get(11)
            .ok_or_else(|| io::Error::other("utime absent"))?
            .parse()?;
        let system: u64 = fields
            .get(12)
            .ok_or_else(|| io::Error::other("stime absent"))?
            .parse()?;
        let status = fs::read_to_string(format!("/proc/{pid}/status"))?;
        let rss_kib = status
            .lines()
            .find_map(|line| line.strip_prefix("VmRSS:"))
            .and_then(|value| value.split_whitespace().next())
            .ok_or_else(|| io::Error::other("VmRSS absent"))?
            .parse()?;
        let high_water_kib = status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|value| value.split_whitespace().next())
            .ok_or_else(|| io::Error::other("VmHWM absent"))?
            .parse()?;
        Ok(Self {
            cpu_ticks: user + system,
            rss_kib,
            high_water_kib,
        })
    }
}

#[derive(Default, Clone, Copy)]
pub(super) struct Failures {
    pub timeouts: usize,
    pub invalid: usize,
    pub transport: usize,
}

impl Failures {
    pub fn total(&self) -> usize {
        self.timeouts + self.invalid + self.transport
    }
    pub fn add(&mut self, other: Self) {
        self.timeouts += other.timeouts;
        self.invalid += other.invalid;
        self.transport += other.transport;
    }
    pub fn record(&mut self, error: &(dyn std::error::Error + 'static)) -> io::Result<()> {
        if let Some(error) = error.downcast_ref::<io::Error>() {
            if error.kind() == io::ErrorKind::PermissionDenied {
                return Err(io::Error::new(
                    error.kind(),
                    format!("benchmark interrompu : {error}"),
                ));
            }
            if error.kind() == io::ErrorKind::InvalidData {
                self.invalid += 1;
                return Ok(());
            }
            self.transport += 1;
            return Ok(());
        }
        if error.is::<tokio::time::error::Elapsed>() {
            self.timeouts += 1;
        } else {
            self.invalid += 1;
        }
        Ok(())
    }
}

pub(super) fn percentile(sorted: &[f64], percentile: usize) -> f64 {
    sorted
        .get(sorted.len() * percentile / 100)
        .copied()
        .unwrap_or(f64::NAN)
}

pub(super) fn ticks_per_second() -> TestResult<f64> {
    let output = Command::new("getconf").arg("CLK_TCK").output()?;
    if !output.status.success() {
        return Err(io::Error::other("getconf CLK_TCK a échoué").into());
    }
    let ticks: f64 = std::str::from_utf8(&output.stdout)?.trim().parse()?;
    if ticks <= 0.0 {
        return Err(io::Error::other("CLK_TCK invalide").into());
    }
    Ok(ticks)
}

pub(super) fn concurrencies() -> TestResult<Vec<usize>> {
    let value = std::env::var("LIGHT_DNS_BENCH_CONCURRENCY").unwrap_or_else(|_| "1,32,256".into());
    let values = value
        .split(',')
        .map(str::parse::<usize>)
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() || values.iter().any(|value| ![1, 32, 256].contains(value)) {
        return Err(io::Error::other("concurrence attendue : sous-ensemble de 1,32,256").into());
    }
    Ok(values)
}
