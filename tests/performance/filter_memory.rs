//! Mesure du chargement seul : aucune socket, aucun worker DNS/client inclus.
use super::{SharedFilters, Updater, index::FilterIndex};
use crate::config::FilteringConfig;
use arc_swap::ArcSwap;
use std::{fmt::Write, fs, io, path::PathBuf, sync::Arc, time::Instant};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn usage() -> Result<(u64, u64, u64)> {
    let stat = fs::read_to_string("/proc/self/stat")?;
    let tail = stat
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("stat invalide"))?
        .1;
    let fields: Vec<_> = tail.split_whitespace().collect();
    let ticks = fields[11].parse::<u64>()? + fields[12].parse::<u64>()?;
    let status = fs::read_to_string("/proc/self/status")?;
    let field = |prefix: &str| -> Result<u64> {
        Ok(status
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .and_then(|value| value.split_whitespace().next())
            .ok_or_else(|| io::Error::other("champ /proc absent"))?
            .parse()?)
    };
    Ok((ticks, field("VmRSS:")?, field("VmHWM:")?))
}

fn sample(csv: &mut String, phase: &str, ticks: f64, baseline: u64, elapsed_ms: f64) -> Result {
    let (cpu, rss, hwm) = usage()?;
    writeln!(
        csv,
        "{phase},{elapsed_ms:.2},{:.1},{rss},{hwm}",
        cpu.saturating_sub(baseline) as f64 * 1000.0 / ticks
    )?;
    Ok(())
}

#[tokio::test]
#[ignore = "mémoire Linux release, chargement/reload sans réseau, corpus synthétique ou fichier local"]
async fn filtering_memory() -> Result {
    if cfg!(debug_assertions) {
        return Err(io::Error::other("exécuter avec --release").into());
    }
    let directory = std::env::temp_dir().join(format!("light-dns-memory-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let directory = Directory(directory);
    let path = if let Some(path) = std::env::var_os("LIGHT_DNS_MEMORY_FILE") {
        PathBuf::from(path)
    } else {
        let domains = std::env::var("LIGHT_DNS_MEMORY_DOMAINS")
            .unwrap_or_else(|_| "100000".into())
            .parse::<usize>()?;
        if !(1..=450_000).contains(&domains) {
            return Err(io::Error::other("1 à 450000 domaines attendus").into());
        }
        let mut text = String::new();
        for index in 0..domains {
            writeln!(text, "block{index}.example")?;
        }
        for index in 0..1000 {
            writeln!(text, "||ad{index}*.example^$dnstype=AAAA")?;
        }
        let path = directory.0.join("list.txt");
        fs::write(&path, text)?;
        path
    };
    let ticks_output = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()?;
    if !ticks_output.status.success() {
        return Err(io::Error::other("getconf a échoué").into());
    }
    let ticks = std::str::from_utf8(&ticks_output.stdout)?
        .trim()
        .parse::<f64>()?;
    if ticks <= 0.0 {
        return Err(io::Error::other("CLK_TCK invalide").into());
    }
    let config = FilteringConfig {
        sources: vec![],
        blocklists: vec![path],
        max_list_bytes: 16 * 1024 * 1024,
        ..FilteringConfig::default()
    };
    let baseline = usage()?.0;
    let start = Instant::now();
    let mut csv = "phase,elapsed_ms,cumulative_cpu_ms,rss_kib,process_hwm_kib\n".to_owned();
    sample(&mut csv, "before", ticks, baseline, 0.0)?;
    let mut shared: SharedFilters = Arc::new(ArcSwap::from_pointee(FilterIndex::default()));
    let mut updater = Updater::prepare(config.clone(), shared.clone()).await?;
    sample(
        &mut csv,
        "loaded",
        ticks,
        baseline,
        start.elapsed().as_secs_f64() * 1000.0,
    )?;
    for generation in 1..=5 {
        let old = Arc::downgrade(&shared.load_full());
        let next: SharedFilters = Arc::new(ArcSwap::from_pointee(FilterIndex::default()));
        let prepared = Updater::prepare(config.clone(), next.clone()).await?;
        sample(
            &mut csv,
            &format!("reload{generation}_coexisting"),
            ticks,
            baseline,
            start.elapsed().as_secs_f64() * 1000.0,
        )?;
        drop(updater);
        drop(shared);
        assert!(old.upgrade().is_none(), "ancien index encore référencé");
        updater = prepared;
        shared = next;
        sample(
            &mut csv,
            &format!("reload{generation}_released"),
            ticks,
            baseline,
            start.elapsed().as_secs_f64() * 1000.0,
        )?;
    }
    let output = std::env::var_os("LIGHT_DNS_MEMORY_CSV")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/benchmarks/filter-memory.csv"));
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, &csv)?;
    println!("{csv}");
    Ok(())
}
