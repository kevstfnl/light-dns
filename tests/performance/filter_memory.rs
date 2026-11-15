//! Mesure Linux du chargement et des recherches : aucune socket DNS/client incluse.
use super::{SharedFilters, Updater, index::FilterIndex};
use crate::config::FilteringConfig;
use arc_swap::ArcSwap;
use hickory_proto::rr::{Name, RecordType};
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

fn sample(
    csv: &mut String,
    phase: &str,
    ticks: f64,
    baseline: u64,
    elapsed_ms: f64,
    index: Option<&FilterIndex>,
    queries: usize,
) -> Result {
    let (cpu, rss, hwm) = usage()?;
    writeln!(
        csv,
        "{phase},{elapsed_ms:.2},{:.1},{rss},{hwm},{},{queries},{}",
        cpu.saturating_sub(baseline) as f64 * 1000.0 / ticks,
        index.map_or(0, FilterIndex::len),
        index
            .and_then(FilterIndex::mask_regex)
            .map_or(0, |regex| regex.memory_usage())
    )?;
    Ok(())
}

#[tokio::test]
#[ignore = "mémoire Linux release, chargement/recherches/reload sans réseau, corpus synthétique ou fichier local"]
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
        if domains > 450_000 {
            return Err(io::Error::other("0 à 450000 domaines attendus").into());
        }
        let mut text = String::new();
        for index in 0..domains {
            writeln!(text, "block{index}.example")?;
        }
        let masks = std::env::var("LIGHT_DNS_MEMORY_MASKS")
            .unwrap_or_else(|_| "1000".into())
            .parse::<usize>()?;
        if masks > 1000 || domains + masks == 0 {
            return Err(io::Error::other("0 à 1000 masques, au moins une règle attendue").into());
        }
        for index in 0..masks {
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
    let mut csv = "phase,elapsed_ms,cumulative_cpu_ms,rss_kib,process_hwm_kib,rules,queries,regex_compiled_bytes\n".to_owned();
    sample(&mut csv, "before", ticks, baseline, 0.0, None, 0)?;
    let mut shared: SharedFilters = Arc::new(ArcSwap::from_pointee(FilterIndex::default()));
    let mut updater = Updater::prepare(config.clone(), shared.clone()).await?;
    sample(
        &mut csv,
        "loaded",
        ticks,
        baseline,
        start.elapsed().as_secs_f64() * 1000.0,
        Some(&shared.load_full()),
        0,
    )?;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    sample(
        &mut csv,
        "idle",
        ticks,
        baseline,
        start.elapsed().as_secs_f64() * 1000.0,
        Some(&shared.load_full()),
        0,
    )?;
    let queries = std::env::var("LIGHT_DNS_MEMORY_QUERIES")
        .unwrap_or_else(|_| "100000".into())
        .parse::<usize>()?;
    if !(1..=10_000_000).contains(&queries) {
        return Err(io::Error::other("1 à 10000000 requêtes attendues").into());
    }
    let repeated = Name::from_ascii("ad500x.example.")?;
    for unique in [false, true] {
        for id in 0..queries {
            let name = if unique {
                Name::from_ascii(if id % 2 == 0 {
                    format!("ad{}x.example.", id % 1000)
                } else {
                    format!("unique{id}.example.")
                })?
            } else {
                repeated.clone()
            };
            std::hint::black_box(shared.load().decide(&name, RecordType::AAAA));
            if (id + 1) % 1000 == 0 || id + 1 == queries {
                sample(
                    &mut csv,
                    if unique { "distinct" } else { "repeated" },
                    ticks,
                    baseline,
                    start.elapsed().as_secs_f64() * 1000.0,
                    Some(&shared.load_full()),
                    id + 1 + usize::from(unique) * queries,
                )?;
            }
        }
    }
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
            Some(&shared.load_full()),
            2 * queries,
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
            Some(&shared.load_full()),
            2 * queries,
        )?;
    }
    let workers = std::env::var("LIGHT_DNS_MEMORY_WORKERS")
        .unwrap_or_else(|_| "4".into())
        .parse::<usize>()?;
    if !(1..=4).contains(&workers) {
        return Err(io::Error::other("1 à 4 workers attendus").into());
    }
    let mut readers = Vec::new();
    let barrier = Arc::new(std::sync::Barrier::new(workers));
    for _ in 0..workers {
        let index = shared.load_full();
        let barrier = barrier.clone();
        readers.push(std::thread::spawn(move || -> Result {
            barrier.wait();
            for id in 0..queries {
                let name = Name::from_ascii(format!("ad{}x.example.", id % 1000))?;
                std::hint::black_box(index.decide(&name, RecordType::AAAA));
            }
            Ok(())
        }));
    }
    for reader in readers {
        reader
            .join()
            .map_err(|_| io::Error::other("reader panicked"))??;
    }
    sample(
        &mut csv,
        "concurrent",
        ticks,
        baseline,
        start.elapsed().as_secs_f64() * 1000.0,
        Some(&shared.load_full()),
        (2 + workers) * queries,
    )?;
    drop(updater);
    let old = Arc::downgrade(&shared.load_full());
    shared.store(Arc::new(FilterIndex::default()));
    assert!(old.upgrade().is_none());
    sample(
        &mut csv,
        "dropped",
        ticks,
        baseline,
        start.elapsed().as_secs_f64() * 1000.0,
        Some(&shared.load_full()),
        (2 + workers) * queries,
    )?;
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
