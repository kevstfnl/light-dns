use super::measurement::{Failures, Usage, percentile, ticks_per_second};
use crate::support::{TestResult, dns::*, process::TestDirectory};
use hickory_proto::op::{Message, ResponseCode};
use std::{
    fmt::Write,
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    net::{TcpListener, TcpStream, UdpSocket},
    sync::Barrier,
    task::JoinSet,
    time::{sleep, timeout},
};

// Drop order stops the daemon before removing its working directory, including on failure.
struct Daemon(Child, TestDirectory);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Clone, Copy, Debug)]
enum Scenario {
    Udp,
    Tcp,
    Fallback,
    Failover,
}

async fn client(
    address: SocketAddr,
    tcp: bool,
    barrier: Arc<Barrier>,
    duration: Duration,
) -> TestResult<(Vec<f64>, Failures)> {
    let socket = UdpSocket::bind("127.0.0.1:0").await?;
    socket.connect(address).await?;
    let mut stream = if tcp {
        let stream = timeout(Duration::from_secs(3), TcpStream::connect(address)).await??;
        stream.set_nodelay(true)?;
        Some(stream)
    } else {
        None
    };
    let mut request = query("bench.test.")?;
    let mut buffer = [0; 4096];
    let mut latencies = Vec::new();
    let mut errors = Failures::default();
    timeout(Duration::from_secs(5), barrier.wait()).await?;
    let until = Instant::now() + duration;
    while Instant::now() < until {
        if tcp && stream.is_none() {
            match timeout(Duration::from_secs(3), TcpStream::connect(address)).await {
                Ok(Ok(connection)) => {
                    connection.set_nodelay(true)?;
                    stream = Some(connection);
                }
                Err(_) => {
                    errors.timeouts += 1;
                    sleep(Duration::from_millis(10)).await;
                    continue;
                }
                Ok(Err(error)) => {
                    errors.record(&error)?;
                    sleep(Duration::from_millis(10)).await;
                    continue;
                }
            }
        }
        request.metadata.id = request.id.wrapping_add(1);
        let begin = Instant::now();
        let response = timeout(Duration::from_secs(3), async {
            if let Some(stream) = &mut stream {
                write_tcp(stream, &request).await?;
                read_tcp(stream).await
            } else {
                socket.send(&request.to_vec()?).await?;
                let length = socket.recv(&mut buffer).await?;
                Ok(Message::from_vec(&buffer[..length])?)
            }
        })
        .await;
        match response {
            Ok(Err(error))
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|error| error.kind() == io::ErrorKind::PermissionDenied) =>
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("client DNS pendant la charge : {error}"),
                )
                .into());
            }
            Ok(Ok(response))
                if response.id == request.id
                    && response.queries == request.queries
                    && response.response_code == ResponseCode::NoError
                    && response.answers == answer(&request).answers =>
            {
                latencies.push(begin.elapsed().as_secs_f64() * 1e6);
            }
            // Reconnect after TCP I/O errors: a partially read frame cannot be reused.
            Ok(Ok(_)) => errors.invalid += 1,
            Ok(Err(error)) => {
                errors.record(error.as_ref())?;
                if tcp {
                    stream = None;
                }
            }
            Err(_) => {
                errors.timeouts += 1;
                if tcp {
                    stream = None;
                }
            }
        }
    }
    Ok((latencies, errors))
}

async fn load(
    address: SocketAddr,
    tcp: bool,
    concurrency: usize,
    duration: Duration,
) -> TestResult<(Vec<f64>, Failures, f64)> {
    let barrier = Arc::new(Barrier::new(concurrency + 1));
    let mut clients = JoinSet::new();
    for _ in 0..concurrency {
        clients.spawn(client(address, tcp, barrier.clone(), duration));
    }
    tokio::select! {
        result = timeout(Duration::from_secs(5), barrier.wait()) => { result?; }
        result = clients.join_next() => {
            if let Some(result) = result { result??; }
            return Err(io::Error::other("client terminé avant le départ de la charge").into());
        }
    }
    let start = Instant::now();
    let mut latencies = Vec::new();
    let mut errors = Failures::default();
    while let Some(result) = clients.join_next().await {
        let (samples, failures) = result??;
        latencies.extend(samples);
        errors.add(failures);
    }
    Ok((latencies, errors, start.elapsed().as_secs_f64()))
}

async fn measure(
    scenario: Scenario,
    concurrency: usize,
    trial: usize,
    binary: &Path,
    ticks: f64,
) -> TestResult<(String, bool)> {
    let directory = TestDirectory::new()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let upstream = listener.local_addr()?;
    let udp_upstream = UdpSocket::bind(upstream).await?;
    let connections = Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    // Owned JoinSet cancels all fixture connections when this measurement ends.
    let mut fixtures = JoinSet::new();
    fixtures.spawn(async move {
        let mut peers = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut stream, _) = accepted?;
                    stream.set_nodelay(true)?;
                    count.fetch_add(1, Ordering::Relaxed);
                    peers.spawn(async move {
                        while let Ok(request) = read_tcp(&mut stream).await {
                            if write_tcp(&mut stream, &answer(&request)).await.is_err() { break; }
                        }
                    });
                }
                _ = peers.join_next(), if !peers.is_empty() => {}
            }
        }
        #[allow(unreachable_code)]
        TestResult::Ok(())
    });
    fixtures.spawn(async move {
        let mut bytes = [0; 4096];
        loop {
            let (length, peer) = udp_upstream.recv_from(&mut bytes).await.map_err(|error| {
                io::Error::new(error.kind(), format!("fixture UDP recv_from : {error}"))
            })?;
            let request = Message::from_vec(&bytes[..length])?;
            let mut response = answer(&request);
            if matches!(scenario, Scenario::Fallback) {
                response.answers.clear();
                response.metadata.truncation = true;
            }
            // A timed-out requester may have closed its socket before this reply.
            // Ignore that peer's ICMP refusal, but expose other fixture errors.
            if let Err(error) = udp_upstream.send_to(&response.to_vec()?, peer).await
                && error.kind() != io::ErrorKind::ConnectionRefused
            {
                return Err(
                    io::Error::new(error.kind(), format!("fixture UDP send_to : {error}")).into(),
                );
            }
        }
        #[allow(unreachable_code)]
        TestResult::Ok(())
    });
    let silent = UdpSocket::bind("127.0.0.1:0").await?;
    let upstreams = if matches!(scenario, Scenario::Failover) {
        format!("['{}', '{upstream}']", silent.local_addr()?)
    } else {
        format!("['{upstream}']")
    };
    let config = format!(
        "[dns]\nlisten = '127.0.0.1:0'\nupstreams = {upstreams}\nupstream_timeout_ms = 100\n[rewrites]\n[cache]\nenabled = false\n[filtering]\nenabled = false\n"
    );
    // Config validation requires a nonzero port; reserve an ephemeral candidate, then let
    // the child bind it. A race is reported as startup failure, never silently ignored.
    let reserve = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = reserve.local_addr()?;
    fs::write(
        directory.0.join("config.toml"),
        config.replace("127.0.0.1:0", &address.to_string()),
    )?;
    drop(reserve);
    let child = Command::new(binary)
        .current_dir(&directory.0)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut daemon = Daemon(child, directory);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !daemon.1.0.join("light-dns.sock").exists() || TcpStream::connect(address).await.is_err()
    {
        if let Some(status) = daemon.0.try_wait()? {
            return Err(io::Error::other(format!("daemon arrêté : {status}")).into());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("démarrage expiré").into());
        }
        sleep(Duration::from_millis(10)).await;
    }
    let tcp = matches!(scenario, Scenario::Tcp);
    // Mesurer la première panne avant que l'échauffement mette le primaire à l'écart.
    let cold_failover_us = if matches!(scenario, Scenario::Failover) {
        let request = query("cold.test.")?;
        let started = Instant::now();
        let response = udp(address, &request).await?.0;
        if response.queries != request.queries
            || response.id != request.id
            || response.answers != answer(&request).answers
            || response.response_code != ResponseCode::NoError
        {
            return Err(io::Error::other("réponse de bascule initiale invalide").into());
        }
        started.elapsed().as_secs_f64() * 1e6
    } else {
        f64::NAN
    };
    let (_, warmup_errors, _) = load(address, tcp, concurrency, Duration::from_secs(2)).await?;
    if let Some(result) = fixtures.try_join_next() {
        result??;
        return Err(io::Error::other("fixture upstream arrêtée pendant l'échauffement").into());
    }
    let pid = daemon.0.id().to_string();
    let before = Usage::read_process(&pid)?;
    let connections_before = connections.load(Ordering::Relaxed);
    let mut peak = before.rss_kib;
    let measurement = load(address, tcp, concurrency, Duration::from_secs(10));
    tokio::pin!(measurement);
    let (mut latencies, errors, seconds) = loop {
        tokio::select! {
            result = fixtures.join_next() => {
                if let Some(result) = result { result??; }
                return Err(io::Error::other("fixture upstream arrêtée pendant la mesure").into());
            }
            result = &mut measurement => break result?,
            _ = sleep(Duration::from_millis(50)) => { peak = peak.max(Usage::read_process(&pid)?.rss_kib); }
        }
    };
    let after = Usage::read_process(&pid)?;
    peak = peak.max(after.rss_kib);
    latencies.sort_by(f64::total_cmp);
    if let Some(status) = daemon.0.try_wait()? {
        return Err(io::Error::other(format!("daemon arrêté pendant la mesure : {status}")).into());
    }
    let samples = latencies.len();
    let errors_total = errors.total();
    let warmup_total = warmup_errors.total();
    let cpu_ms = (after.cpu_ticks - before.cpu_ticks) as f64 * 1000.0 / ticks;
    let cpu_us_per_query = if samples == 0 {
        f64::NAN
    } else {
        cpu_ms * 1000.0 / samples as f64
    };
    let row = format!(
        "{scenario:?},{concurrency},{trial},{samples},{errors_total},{warmup_total},{seconds:.3},{:.2},{:.2},{:.2},{:.2},{cpu_ms:.1},{:.3},{},{peak},{},{},{},{},{},{cold_failover_us:.2}\n",
        samples as f64 / seconds,
        percentile(&latencies, 50),
        percentile(&latencies, 95),
        percentile(&latencies, 99),
        cpu_us_per_query,
        before.rss_kib,
        connections.load(Ordering::Relaxed) - connections_before,
        after.high_water_kib,
        errors.timeouts,
        errors.invalid,
        errors.transport
    );
    Ok((row, errors_total != 0 || warmup_total != 0))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark forwarding Linux : --release, démon séparé, environ 12 minutes"]
async fn dns_forwarding() -> TestResult {
    if cfg!(debug_assertions) {
        return Err(io::Error::other("exécuter avec --release").into());
    }
    let binary = std::env::var_os("LIGHT_DNS_BENCH_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_light-dns")));
    let binaries = match std::env::var_os("LIGHT_DNS_BENCH_COMPARE_BINARY") {
        Some(candidate) => vec![("before", binary), ("candidate", PathBuf::from(candidate))],
        None => vec![("current", binary)],
    };
    let output = std::env::var_os("LIGHT_DNS_BENCH_CSV")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/benchmarks/forwarding.csv"));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let ticks = ticks_per_second()?;
    let concurrencies = match std::env::var("LIGHT_DNS_BENCH_CONCURRENCY") {
        Ok(value) => {
            let values = value
                .split(',')
                .map(str::parse::<usize>)
                .collect::<Result<Vec<_>, _>>()?;
            if values.is_empty() || values.iter().any(|value| !(1..=256).contains(value)) {
                return Err(io::Error::other("concurrence benchmark attendue : 1 à 256").into());
            }
            values
        }
        Err(std::env::VarError::NotPresent) => vec![1, 32, 256],
        Err(error) => return Err(error.into()),
    };
    let mut csv = String::from(
        "variant,scenario,concurrency,trial,samples,errors,warmup_errors,seconds,qps,p50_us,p95_us,p99_us,cpu_ms,cpu_us_per_query,rss_before_kib,rss_peak_kib,tcp_connections,process_hwm_kib,timeouts,invalid,transport_errors,cold_failover_us\n",
    );
    fs::write(&output, &csv)?;
    let error_file = output.with_extension("error.txt");
    if error_file.exists() {
        fs::remove_file(&error_file)?;
    }
    let mut failed = false;
    for trial in 1..=5 {
        for scenario in [
            Scenario::Udp,
            Scenario::Tcp,
            Scenario::Fallback,
            Scenario::Failover,
        ] {
            for &concurrency in &concurrencies {
                // Alternate order between trials to reduce drift from CPU frequency/load.
                for index in 0..binaries.len() {
                    let index = if trial % 2 == 0 {
                        binaries.len() - index - 1
                    } else {
                        index
                    };
                    let (variant, binary) = &binaries[index];
                    let (row, has_errors) = match measure(
                        scenario,
                        concurrency,
                        trial,
                        binary,
                        ticks,
                    )
                    .await
                    {
                        Ok(row) => row,
                        Err(error) => {
                            fs::write(
                                output.with_extension("error.txt"),
                                format!(
                                    "INCOMPLETE variant={variant} scenario={scenario:?} concurrency={concurrency} trial={trial}: {error}\n"
                                ),
                            )?;
                            return Err(error);
                        }
                    };
                    failed |= has_errors;
                    print!("{variant},{row}");
                    writeln!(csv, "{variant},{}", row.trim_end())?;
                    fs::write(&output, &csv)?;
                }
            }
        }
    }
    if failed {
        fs::write(
            error_file,
            "FAILED : campagne terminée avec erreurs DNS ; voir les compteurs du CSV\n",
        )?;
        return Err(io::Error::other("campagne forwarding avec erreurs DNS").into());
    }
    Ok(())
}
