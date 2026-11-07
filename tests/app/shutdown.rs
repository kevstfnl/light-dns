use crate::support::process::TestDirectory;
use std::{fs, io, process::Command};

#[test]
fn serves_dns_and_stops_on_sigterm() -> Result<(), Box<dyn std::error::Error>> {
    use hickory_proto::{
        op::{Message, Query, ResponseCode},
        rr::{Name, RecordType},
    };
    use std::{
        net::{TcpListener, UdpSocket},
        process::{Child, Stdio},
        thread,
        time::{Duration, Instant},
    };

    struct RunningChild(Child);
    impl Drop for RunningChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let directory = TestDirectory::new()?;
    let udp = UdpSocket::bind("127.0.0.1:0")?;
    let address = udp.local_addr()?;
    let tcp = TcpListener::bind(address)?;
    fs::write(
        directory.0.join("config.toml"),
        format!(
            "[dns]\nlisten = '{address}'\nupstreams = ['127.0.0.1:9']\n[rewrites]\n'*.lan' = '192.168.1.1'\n[filtering]\nenabled = false\n"
        ),
    )?;
    drop((udp, tcp));
    let mut child = RunningChild(
        Command::new(env!("CARGO_BIN_EXE_light-dns"))
            .current_dir(&directory.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?,
    );
    let client = UdpSocket::bind("127.0.0.1:0")?;
    client.set_read_timeout(Some(Duration::from_millis(50)))?;
    let mut request = Message::query();
    request.metadata.recursion_desired = true;
    request.queries.push(Query::query(
        Name::from_ascii("unknown.lan.")?,
        RecordType::A,
    ));
    let bytes = request.to_vec()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buffer = [0; 512];
    loop {
        assert!(Instant::now() < deadline, "le serveur ne démarre pas");
        assert!(child.0.try_wait()?.is_none(), "arrêt prématuré du serveur");
        client.send_to(&bytes, address)?;
        match client.recv_from(&mut buffer) {
            Ok((length, _)) => {
                let response = Message::from_vec(&buffer[..length])?;
                assert_eq!(response.id, request.id);
                assert_eq!(response.response_code, ResponseCode::NoError);
                assert_eq!(response.answers.len(), 1);
                break;
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()?
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.0.try_wait()? {
            assert!(status.success(), "{status}");
            break;
        }
        assert!(Instant::now() < deadline, "le serveur ne s'arrête pas");
        thread::sleep(Duration::from_millis(10));
    }
    let _udp = UdpSocket::bind(address)?;
    let _tcp = TcpListener::bind(address)?;
    Ok(())
}
