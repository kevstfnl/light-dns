//! Diagnostics du processus serveur, lus uniquement sur demande administrative.
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct Process {
    pid: u32,
    version: &'static str,
    os: &'static str,
    arch: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rss_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peak_rss_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    threads: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    measurement_error: Option<String>,
}

impl Process {
    pub async fn read() -> Self {
        let mut process = Self {
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            rss_bytes: None,
            peak_rss_bytes: None,
            data_bytes: None,
            threads: None,
            measurement_error: None,
        };
        #[cfg(target_os = "linux")]
        match tokio::fs::read_to_string("/proc/self/status").await {
            Ok(text) => {
                process.rss_bytes = field(&text, "VmRSS:", true);
                process.peak_rss_bytes = field(&text, "VmHWM:", true);
                process.data_bytes = field(&text, "VmData:", true);
                process.threads = field(&text, "Threads:", false);
                if [
                    process.rss_bytes,
                    process.peak_rss_bytes,
                    process.data_bytes,
                    process.threads,
                ]
                .contains(&None)
                {
                    process.measurement_error = Some("mesures /proc incomplètes".into());
                }
            }
            Err(error) => {
                process.measurement_error = Some(format!("lecture /proc/self/status : {error}"))
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            process.measurement_error =
                Some("mesures mémoire disponibles sous Linux uniquement".into());
        }
        process
    }
}

#[cfg(any(target_os = "linux", test))]
fn field(text: &str, prefix: &str, kib: bool) -> Option<u64> {
    let mut parts = text
        .lines()
        .find_map(|line| line.strip_prefix(prefix))?
        .split_whitespace();
    let value = parts.next()?.parse::<u64>().ok()?;
    if kib {
        if parts.next()? != "kB" {
            return None;
        }
        value.checked_mul(1024)
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proc_units_missing_fields_and_overflow_are_explicit() {
        assert_eq!(
            field("VmRSS: 42 kB\nThreads: 5\n", "VmRSS:", true),
            Some(43008)
        );
        assert_eq!(field("Threads: 5\n", "Threads:", false), Some(5));
        for text in [
            "",
            "VmRSS: invalid kB",
            "VmRSS: 12 MB",
            "VmRSS: 18446744073709551615 kB",
        ] {
            assert_eq!(field(text, "VmRSS:", true), None);
        }
    }
}
