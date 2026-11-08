use super::measurement::{Failures, Usage, percentile, ticks_per_second};
use crate::support::{TestResult, dns::*, process::TestDirectory};
use hickory_proto::{
    op::{Message, ResponseCode},
    rr::{Name, RData, Record, RecordType, rdata::CNAME},
};
use std::{
    fmt::Write,
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio::{
    net::{TcpListener, UdpSocket},
    time::{sleep, timeout},
};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn response(request: &Message) -> Message {
    let mut response = answer(request);
    if request.queries[0].name().to_ascii().starts_with("alias") {
        let target = Name::from_ascii("target.example.").unwrap();
        response.answers[0].name = target.clone();
        response.answers.insert(
            0,
            Record::from_rdata(
                request.queries[0].name().clone(),
                120,
                RData::CNAME(CNAME(target)),
            ),
        );
    }
    response
}

struct Case {
    name: &'static str,
    query: &'static str,
    advanced: bool,
    cached: bool,
    kind: RecordType,
    code: ResponseCode,
}

async fn measure(binary: &Path, case: &Case, base: &str, ticks: f64) -> TestResult<(String, bool)> {
    let directory = TestDirectory::new()?;
    let upstream = Upstream::start(response, response).await?;
    let reserve = UdpSocket::bind("127.0.0.1:0").await?;
    let address = reserve.local_addr()?;
    let tcp = TcpListener::bind(address).await?;
    let list = directory.0.join("list.txt");
    let mut text = base.to_owned();
    if case.advanced {
        for index in 0..1000 {
            writeln!(text, "||ad{index}*.example^$dnstype=AAAA")?;
        }
        text.push_str("||rewrite.example^$dnsrewrite=192.0.2.42\n");
    }
    if case.code == ResponseCode::NXDomain && case.query.starts_with("alias") {
        text.push_str("target.example\n");
    }
    fs::write(&list, text)?;
    let socket = directory.0.join("control.sock");
    let config = directory.0.join("config.toml");
    fs::write(
        &config,
        format!(
            "[dns]\nlisten = '{address}'\nupstreams = ['{}']\n[rewrites]\n'local.example' = '192.0.2.42'\n'*.home.arpa' = '192.0.2.42'\n[cache]\nenabled = {}\n[filtering]\nallowlist = ['safe.block50000.example']\nsources = []\nblocklists = ['{}']\n",
            upstream.address,
            case.cached,
            list.display()
        ),
    )?;
    drop((reserve, tcp));
    let mut daemon = Daemon(
        Command::new(binary)
            .args([
                "--config",
                &config.to_string_lossy(),
                "--socket",
                &socket.to_string_lossy(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(mut stream) = tokio::net::UnixStream::connect(&socket).await {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            stream.write_all(b"stats\n").await?;
            let mut result = String::new();
            timeout(Duration::from_secs(2), stream.read_to_string(&mut result)).await??;
            if result.starts_with("OK\n") {
                break;
            }
        }
        if daemon.0.try_wait()?.is_some() || Instant::now() > until {
            return Err(io::Error::other("démarrage benchmark impossible").into());
        }
        sleep(Duration::from_millis(10)).await;
    }
    let client = UdpSocket::bind("127.0.0.1:0").await?;
    client.connect(address).await?;
    let mut request = query(case.query)?;
    request.queries[0].query_type = case.kind;
    let mut buffer = [0; 4096];
    for index in 0..100 {
        exchange(&client, &request, &mut buffer, case)
            .await
            .map_err(|error| {
                io::Error::other(format!(
                    "échauffement interrompu après {index} succès, 1 erreur : {error}"
                ))
            })?;
    }
    let before = Usage::read_process(&daemon.0.id().to_string())?;
    let mut latencies = Vec::with_capacity(50_000);
    let mut failures = Failures::default();
    let start = Instant::now();
    let mut consecutive_errors = 0;
    for id in 0..50_000 {
        request.metadata.id = id as u16;
        let begin = Instant::now();
        match exchange(&client, &request, &mut buffer, case).await {
            Ok(()) => {
                latencies.push(begin.elapsed().as_secs_f64() * 1e6);
                consecutive_errors = 0;
            }
            Err(error) => {
                failures.record(error.as_ref())?;
                consecutive_errors += 1;
                if consecutive_errors == 16 {
                    break;
                }
            }
        }
    }
    let seconds = start.elapsed().as_secs_f64();
    let after = Usage::read_process(&daemon.0.id().to_string())?;
    latencies.sort_by(f64::total_cmp);
    let cpu_ms = after.cpu_ticks.saturating_sub(before.cpu_ticks) as f64 * 1000.0 / ticks;
    let samples = latencies.len();
    Ok((
        format!(
            "{},{samples},{},{},{},{},{seconds:.3},{:.2},{:.2},{:.2},{:.2},{cpu_ms:.1},{:.3},{},{},{}",
            case.name,
            failures.total(),
            failures.timeouts,
            failures.invalid,
            failures.transport,
            samples as f64 / seconds,
            percentile(&latencies, 50),
            percentile(&latencies, 95),
            percentile(&latencies, 99),
            if samples == 0 {
                f64::NAN
            } else {
                cpu_ms * 1000.0 / samples as f64
            },
            before.rss_kib,
            after.rss_kib,
            after.high_water_kib
        ),
        failures.total() != 0,
    ))
}

async fn exchange(
    socket: &UdpSocket,
    request: &Message,
    buffer: &mut [u8],
    case: &Case,
) -> TestResult {
    socket.send(&request.to_vec()?).await?;
    let length = timeout(Duration::from_secs(3), socket.recv(buffer)).await??;
    let response = Message::from_vec(&buffer[..length])?;
    let expected = if case.code != ResponseCode::NoError {
        Vec::new()
    } else if matches!(
        case.name,
        "local_exact" | "local_wildcard" | "advanced_rewrite"
    ) {
        vec![Record::from_rdata(
            request.queries[0].name().clone(),
            60,
            RData::A(hickory_proto::rr::rdata::A("192.0.2.42".parse()?)),
        )]
    } else {
        self::response(request).answers
    };
    if response.id != request.id
        || response.queries != request.queries
        || response.response_code != case.code
        || response.answers.len() != expected.len()
        || !response
            .answers
            .iter()
            .zip(&expected)
            .all(|(actual, expected)| {
                actual.name == expected.name
                    && actual.dns_class == expected.dns_class
                    && actual.data == expected.data
                    && actual.ttl > 0
                    && actual.ttl <= expected.ttl
            })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "réponse DNS du benchmark invalide",
        )
        .into());
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark Linux release : 100000 domaines + 1000 motifs, 5 essais, démon séparé"]
async fn dns_filtering() -> TestResult {
    if cfg!(debug_assertions) {
        return Err(io::Error::other("exécuter avec --release").into());
    }
    let current = std::env::var_os("LIGHT_DNS_BENCH_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_light-dns")));
    let before = std::env::var_os("LIGHT_DNS_BENCH_COMPARE_BINARY").map(PathBuf::from);
    let mut base = String::new();
    for index in 0..100_000 {
        writeln!(base, "block{index}.example")?;
    }
    let ticks = ticks_per_second()?;
    let cases = [
        Case {
            name: "local_exact",
            query: "local.example.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            name: "local_wildcard",
            query: "deep.host.home.arpa.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            name: "user_allowlist",
            query: "safe.block50000.example.",
            advanced: false,
            cached: true,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            name: "simple_miss",
            query: "clean.example.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            name: "simple_hit",
            query: "block50000.example.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NXDomain,
        },
        Case {
            name: "warm_cache",
            query: "clean.example.",
            advanced: false,
            cached: true,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            name: "cname_cache",
            query: "alias.example.",
            advanced: false,
            cached: true,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            name: "advanced_miss",
            query: "clean.example.",
            advanced: true,
            cached: true,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            name: "advanced_hit",
            query: "ad500x.example.",
            advanced: true,
            cached: false,
            kind: RecordType::AAAA,
            code: ResponseCode::NXDomain,
        },
        Case {
            name: "advanced_rewrite",
            query: "rewrite.example.",
            advanced: true,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            name: "cname_blocked",
            query: "alias.example.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NXDomain,
        },
    ];
    let output = std::env::var_os("LIGHT_DNS_BENCH_CSV")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/benchmarks/filtering.csv"));
    fs::create_dir_all(
        output
            .parent()
            .ok_or_else(|| io::Error::other("chemin CSV invalide"))?,
    )?;
    let mut csv = String::from(
        "variant,trial,scenario,samples,errors,timeouts,invalid,transport_errors,seconds,qps,p50_us,p95_us,p99_us,cpu_ms,cpu_us_per_query,rss_before_kib,rss_after_kib,process_hwm_kib\n",
    );
    fs::write(&output, &csv)?;
    let error_file = output.with_extension("error.txt");
    if error_file.exists() {
        fs::remove_file(error_file)?;
    }
    for trial in 1..=5 {
        for case in &cases {
            let mut variants = vec![("current", current.as_path())];
            if let Some(before) = &before {
                variants.push(("before", before.as_path()));
            }
            if trial % 2 == 0 {
                variants.reverse();
            }
            for (variant, binary) in variants {
                let (row, failed) = match measure(binary, case, &base, ticks).await {
                    Ok(row) => row,
                    Err(error) => {
                        fs::write(
                            output.with_extension("error.txt"),
                            format!(
                                "INCOMPLETE variant={variant} scenario={} trial={trial}: {error}\n",
                                case.name
                            ),
                        )?;
                        return Err(error);
                    }
                };
                println!("{variant},{trial},{row}");
                writeln!(csv, "{variant},{trial},{row}")?;
                fs::write(&output, &csv)?;
                // Une ligne avec erreurs reste disponible, mais invalide la campagne.
                if failed {
                    fs::write(
                        output.with_extension("error.txt"),
                        format!("INCOMPLETE variant={variant} trial={trial}: {row}\n"),
                    )?;
                    return Err(
                        io::Error::other("erreurs DNS pendant le benchmark ; voir CSV").into(),
                    );
                }
            }
        }
    }
    Ok(())
}
