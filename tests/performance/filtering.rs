use super::measurement::{Usage, ticks_per_second};
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

async fn measure(binary: &Path, case: &Case, base: &str, ticks: f64) -> TestResult<String> {
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
            "[dns]\nlisten = '{address}'\nupstreams = ['{}']\n[rewrites]\n[cache]\nenabled = {}\n[filtering]\nsources = []\nblocklists = ['{}']\n",
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
    for _ in 0..100 {
        exchange(&client, &request, &mut buffer, case.code).await?;
    }
    let before = Usage::read_process(&daemon.0.id().to_string())?;
    let mut latencies = Vec::with_capacity(50_000);
    for id in 0..50_000 {
        request.metadata.id = id as u16;
        let begin = Instant::now();
        exchange(&client, &request, &mut buffer, case.code).await?;
        latencies.push(begin.elapsed().as_secs_f64() * 1e6);
    }
    let after = Usage::read_process(&daemon.0.id().to_string())?;
    latencies.sort_by(f64::total_cmp);
    let cpu_ms = after.cpu_ticks.saturating_sub(before.cpu_ticks) as f64 * 1000.0 / ticks;
    Ok(format!(
        "{},{:.2},{:.2},{cpu_ms:.1},{:.3},{},{}",
        case.name,
        latencies[25_000],
        latencies[47_500],
        cpu_ms * 1000.0 / 50_000.0,
        before.rss_kib,
        after.rss_kib
    ))
}

async fn exchange(
    socket: &UdpSocket,
    request: &Message,
    buffer: &mut [u8],
    code: ResponseCode,
) -> TestResult {
    socket.send(&request.to_vec()?).await?;
    let length = timeout(Duration::from_secs(3), socket.recv(buffer)).await??;
    let response = Message::from_vec(&buffer[..length])?;
    assert_eq!(response.id, request.id);
    assert_eq!(response.queries, request.queries);
    assert_eq!(response.response_code, code);
    if code == ResponseCode::NoError {
        assert!(!response.answers.is_empty());
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "benchmark Linux release : 100000 domaines + 1000 motifs, 5 essais, démon séparé"]
async fn dns_filtering() -> TestResult {
    if cfg!(debug_assertions) {
        return Err(io::Error::other("exécuter avec --release").into());
    }
    let current = PathBuf::from(env!("CARGO_BIN_EXE_light-dns"));
    let before = std::env::var_os("LIGHT_DNS_BENCH_COMPARE_BINARY").map(PathBuf::from);
    let mut base = String::new();
    for index in 0..100_000 {
        writeln!(base, "block{index}.example")?;
    }
    let ticks = ticks_per_second()?;
    let cases = [
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
    let output = PathBuf::from("target/benchmarks/filtering.csv");
    fs::create_dir_all(
        output
            .parent()
            .ok_or_else(|| io::Error::other("chemin CSV invalide"))?,
    )?;
    let mut csv = String::from(
        "variant,trial,scenario,p50_us,p95_us,cpu_ms,cpu_us_per_query,rss_before_kib,rss_after_kib\n",
    );
    for trial in 1..=5 {
        for case in &cases {
            let mut variants = vec![("current", current.as_path())];
            if !case.advanced
                && case.name != "cname_blocked"
                && let Some(before) = &before
            {
                variants.push(("before", before.as_path()));
            }
            if trial % 2 == 0 {
                variants.reverse();
            }
            for (variant, binary) in variants {
                let row = measure(binary, case, &base, ticks).await?;
                println!("{variant},{trial},{row}");
                writeln!(csv, "{variant},{trial},{row}")?;
                fs::write(&output, &csv)?;
            }
        }
    }
    Ok(())
}
