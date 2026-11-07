use crate::support::TestResult;
use std::{fs, io, process::Command};

pub(super) struct Usage {
    pub cpu_ticks: u64,
    pub rss_kib: u64,
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
        Ok(Self {
            cpu_ticks: user + system,
            rss_kib,
        })
    }
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
