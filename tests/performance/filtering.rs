use super::measurement::{Failures, Usage, percentile, ticks_per_second};
use crate::support::{TestResult, dns::*, process::TestDirectory};
use hickory_proto::{
    op::{Edns, Message, ResponseCode},
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
    net::{TcpListener, TcpStream, UdpSocket},
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
    let name = request.queries[0].name().to_ascii();
    if request.queries[0].query_type() == RecordType::AAAA {
        response.answers[0].data = RData::AAAA(hickory_proto::rr::rdata::AAAA(
            "2001:db8::1".parse().unwrap(),
        ));
    }
    if name.starts_with("edns-negative") {
        response.answers.clear();
        response.metadata.response_code = ResponseCode::NXDomain;
        response.authorities.push(Record::from_rdata(
            Name::from_ascii("example.").unwrap(),
            60,
            RData::SOA(hickory_proto::rr::rdata::SOA::new(
                Name::from_ascii("ns.example.").unwrap(),
                Name::from_ascii("hostmaster.example.").unwrap(),
                1,
                60,
                60,
                60,
                60,
            )),
        ));
    }
    if name.starts_with("edns-near") {
        response.answers = vec![response.answers[0].clone(); 29];
    }
    if name.starts_with("edns-large") {
        response.answers = vec![response.answers[0].clone(); 100];
    }
    response
}

#[derive(Clone)]
struct Case {
    name: &'static str,
    tcp: bool,
    payload: Option<u16>,
    truncated: bool,
    query: &'static str,
    advanced: bool,
    cached: bool,
    kind: RecordType,
    code: ResponseCode,
}

async fn measure(
    binary: &Path,
    case: &Case,
    base: &str,
    ticks: f64,
    concurrency: usize,
) -> TestResult<(String, bool)> {
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
    let mut request = query(case.query)?;
    request.queries[0].query_type = case.kind;
    request.edns = case.payload.map(|size| {
        let mut edns = Edns::new();
        edns.set_max_payload(size);
        edns
    });
    let mut clients = Vec::new();
    for _ in 0..concurrency {
        let mut client = if case.tcp {
            Client::Tcp(TcpStream::connect(address).await?)
        } else {
            let socket = UdpSocket::bind("127.0.0.1:0").await?;
            socket.connect(address).await?;
            Client::Udp(socket)
        };
        let mut buffer = [0; 4096];
        for index in 0..100 {
            exchange(&mut client, &request, &mut buffer, case)
                .await
                .map_err(|error| {
                    io::Error::other(format!(
                        "échauffement interrompu après {index} succès : {error}"
                    ))
                })?;
        }
        clients.push(client);
    }
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(concurrency + 1));
    let mut tasks = tokio::task::JoinSet::new();
    for (client_index, mut client) in clients.into_iter().enumerate() {
        let case = case.clone();
        let mut request = request.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            let mut latencies = Vec::new();
            let mut failures = Failures::default();
            let mut buffer = [0; 4096];
            let mut consecutive_errors = 0;
            barrier.wait().await;
            for id in (client_index..50_000).step_by(concurrency) {
                request.metadata.id = id as u16;
                let begin = Instant::now();
                match exchange(&mut client, &request, &mut buffer, &case).await {
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
            TestResult::Ok((latencies, failures))
        });
    }
    let before = Usage::read_process(&daemon.0.id().to_string())?;
    let start = Instant::now();
    barrier.wait().await;
    let mut latencies = Vec::with_capacity(50_000);
    let mut failures = Failures::default();
    while let Some(result) = tasks.join_next().await {
        let (samples, errors) = result??;
        latencies.extend(samples);
        failures.add(errors);
    }
    let seconds = start.elapsed().as_secs_f64();
    let after = Usage::read_process(&daemon.0.id().to_string())?;
    latencies.sort_by(f64::total_cmp);
    let cpu_ms = after.cpu_ticks.saturating_sub(before.cpu_ticks) as f64 * 1000.0 / ticks;
    let samples = latencies.len();
    Ok((
        format!(
            "{},{concurrency},{samples},{},{},{},{},{seconds:.3},{:.2},{:.2},{:.2},{:.2},{cpu_ms:.1},{:.3},{},{},{}",
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

enum Client {
    Udp(UdpSocket),
    Tcp(TcpStream),
}

async fn exchange(
    client: &mut Client,
    request: &Message,
    buffer: &mut [u8],
    case: &Case,
) -> TestResult {
    let (response, length) = match client {
        Client::Udp(socket) => {
            socket.send(&request.to_vec()?).await?;
            let length = timeout(Duration::from_secs(3), socket.recv(buffer)).await??;
            (Message::from_vec(&buffer[..length])?, length)
        }
        Client::Tcp(stream) => {
            write_tcp(stream, request).await?;
            (timeout(Duration::from_secs(3), read_tcp(stream)).await??, 0)
        }
    };
    let expected = if case.code != ResponseCode::NoError || case.truncated {
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
    let expected_edns = case.payload.map(|size| size.clamp(512, 1232));
    if response.truncation != case.truncated
        || response.edns.as_ref().map(Edns::max_payload) != expected_edns
        || (!case.tcp && length > usize::from(expected_edns.unwrap_or(512)))
        || response.authorities != self::response(request).authorities
        || response.id != request.id
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
            tcp: false,
            payload: None,
            truncated: false,
            name: "local_exact",
            query: "local.example.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "local_wildcard",
            query: "deep.host.home.arpa.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "user_allowlist",
            query: "safe.block50000.example.",
            advanced: false,
            cached: true,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "simple_miss",
            query: "clean.example.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "simple_hit",
            query: "block50000.example.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NXDomain,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "warm_cache",
            query: "clean.example.",
            advanced: false,
            cached: true,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "cname_cache",
            query: "alias.example.",
            advanced: false,
            cached: true,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "advanced_miss",
            query: "clean.example.",
            advanced: true,
            cached: true,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "advanced_hit",
            query: "ad500x.example.",
            advanced: true,
            cached: false,
            kind: RecordType::AAAA,
            code: ResponseCode::NXDomain,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "advanced_rewrite",
            query: "rewrite.example.",
            advanced: true,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NoError,
        },
        Case {
            tcp: false,
            payload: None,
            truncated: false,
            name: "cname_blocked",
            query: "alias.example.",
            advanced: false,
            cached: false,
            kind: RecordType::A,
            code: ResponseCode::NXDomain,
        },
        Case {
            name: "edns_a",
            query: "edns-a.example.",
            kind: RecordType::A,
            code: ResponseCode::NoError,
            payload: Some(1232),
            tcp: false,
            cached: false,
            truncated: false,
            advanced: false,
        },
        Case {
            name: "edns_aaaa",
            query: "edns-aaaa.example.",
            kind: RecordType::AAAA,
            code: ResponseCode::NoError,
            payload: Some(1232),
            tcp: false,
            cached: false,
            truncated: false,
            advanced: false,
        },
        Case {
            name: "edns_negative",
            query: "edns-negative.example.",
            kind: RecordType::A,
            code: ResponseCode::NXDomain,
            payload: Some(1232),
            tcp: false,
            cached: false,
            truncated: false,
            advanced: false,
        },
        Case {
            name: "edns_cache",
            query: "edns-a.example.",
            kind: RecordType::A,
            code: ResponseCode::NoError,
            payload: Some(1232),
            tcp: false,
            cached: true,
            truncated: false,
            advanced: false,
        },
        Case {
            name: "edns_near",
            query: "edns-near.example.",
            kind: RecordType::A,
            code: ResponseCode::NoError,
            payload: Some(512),
            tcp: false,
            cached: false,
            truncated: false,
            advanced: false,
        },
        Case {
            name: "edns_truncated",
            query: "edns-large.example.",
            kind: RecordType::A,
            code: ResponseCode::NoError,
            payload: Some(512),
            tcp: false,
            cached: false,
            truncated: true,
            advanced: false,
        },
        Case {
            name: "edns_tcp",
            query: "edns-large.example.",
            kind: RecordType::A,
            code: ResponseCode::NoError,
            payload: Some(1232),
            tcp: true,
            cached: false,
            truncated: false,
            advanced: false,
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
        "variant,trial,scenario,concurrency,samples,errors,timeouts,invalid,transport_errors,seconds,qps,p50_us,p95_us,p99_us,cpu_ms,cpu_us_per_query,rss_before_kib,rss_after_kib,process_hwm_kib\n",
    );
    fs::write(&output, &csv)?;
    let error_file = output.with_extension("error.txt");
    if error_file.exists() {
        fs::remove_file(error_file)?;
    }
    for trial in 1..=5 {
        for concurrency in super::measurement::concurrencies()? {
            for case in &cases {
                let mut variants = vec![("current", current.as_path())];
                if let Some(before) = &before {
                    variants.push(("before", before.as_path()));
                }
                if trial % 2 == 0 {
                    variants.reverse();
                }
                for (variant, binary) in variants {
                    let (row, failed) = match measure(binary, case, &base, ticks, concurrency).await
                    {
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
                        return Err(io::Error::other(
                            "erreurs DNS pendant le benchmark ; voir CSV",
                        )
                        .into());
                    }
                }
            }
        }
    }
    Ok(())
}
