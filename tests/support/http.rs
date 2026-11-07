use super::TestResult;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::JoinHandle,
};

#[derive(Clone)]
struct Response {
    status: u16,
    body: String,
    length: bool,
    delay: u64,
}

pub(crate) struct HttpServer {
    pub url: String,
    state: Arc<Mutex<Response>>,
    pub hits: Arc<AtomicUsize>,
    task: JoinHandle<()>,
    release: Arc<Notify>,
}

impl HttpServer {
    pub async fn new(body: &str) -> TestResult<Self> {
        Self::start(body, false).await
    }

    pub async fn gated(body: &str) -> TestResult<Self> {
        Self::start(body, true).await
    }

    async fn start(body: &str, gated: bool) -> TestResult<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/list.txt", listener.local_addr()?);
        let state = Arc::new(Mutex::new(Response {
            status: 200,
            body: body.into(),
            length: true,
            delay: 0,
        }));
        let hits = Arc::new(AtomicUsize::new(0));
        let response = state.clone();
        let requests = hits.clone();
        let release = Arc::new(Notify::new());
        let gate = release.clone();
        let task = tokio::spawn(async move {
            let mut gated = gated;
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut headers = Vec::new();
                while headers.len() < 8192 && !headers.ends_with(b"\r\n\r\n") {
                    match stream.read_u8().await {
                        Ok(byte) => headers.push(byte),
                        Err(_) => break,
                    }
                }
                let response = response.lock().unwrap().clone();
                requests.fetch_add(1, Ordering::SeqCst);
                if gated {
                    gate.notified().await;
                    gated = false;
                }
                tokio::time::sleep(std::time::Duration::from_millis(response.delay)).await;
                let length = if response.length {
                    format!("Content-Length: {}\r\n", response.body.len())
                } else {
                    String::new()
                };
                let bytes = format!(
                    "HTTP/1.1 {} Test\r\n{length}Connection: close\r\n\r\n{}",
                    response.status, response.body
                );
                let _ = stream.write_all(bytes.as_bytes()).await;
            }
        });
        Ok(Self {
            url,
            state,
            hits,
            task,
            release,
        })
    }

    pub fn release(&self) {
        self.release.notify_one();
    }

    pub fn set(&self, status: u16, body: &str, length: bool, delay: u64) {
        *self.state.lock().unwrap() = Response {
            status,
            body: body.into(),
            length,
            delay,
        };
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
