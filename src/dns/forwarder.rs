use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
    time::Duration,
};

use hickory_proto::op::{DnsRequest, DnsResponse, Message, MessageType, ResponseCode};
use hickory_server::net::{
    NetError,
    runtime::TokioRuntimeProvider,
    tcp::TcpClientStream,
    udp::UdpClientStream,
    xfer::{DnsHandle, DnsRequestSender, FirstAnswer, Protocol},
};
use tokio::time::timeout;

pub(super) struct Forwarder {
    inner: Arc<Inner>,
    probes: Mutex<tokio::task::JoinSet<()>>,
}

struct Inner {
    upstreams: Vec<SocketAddr>,
    states: Vec<[AtomicU64; 2]>,
    started: tokio::time::Instant,
    timeout: Duration,
    permits: Arc<tokio::sync::Semaphore>,
    counters: Arc<super::server::stats::Counters>,
    stopped: AtomicBool,
}

impl Drop for Forwarder {
    fn drop(&mut self) {
        self.stop_probes();
    }
}

impl Forwarder {
    pub(super) fn new(
        upstreams: Vec<SocketAddr>,
        timeout: Duration,
        counters: Arc<super::server::stats::Counters>,
        permits: Arc<tokio::sync::Semaphore>,
    ) -> Self {
        let states = upstreams
            .iter()
            .map(|_| std::array::from_fn(|_| AtomicU64::new(0)))
            .collect();
        Self {
            inner: Arc::new(Inner {
                upstreams,
                states,
                timeout,
                started: tokio::time::Instant::now(),
                permits,
                stopped: AtomicBool::new(false),
                counters,
            }),
            probes: Mutex::new(tokio::task::JoinSet::new()),
        }
    }

    pub(super) fn budget(&self) -> Duration {
        self.inner
            .timeout
            .saturating_mul(self.inner.upstreams.len().try_into().unwrap_or(u32::MAX))
    }

    pub(super) async fn check_local(
        &self,
        message: &Message,
        protocol: Protocol,
    ) -> Result<Message, NetError> {
        self.inner
            .attempt(self.inner.upstreams[0], message, protocol)
            .await
    }

    pub(super) fn stop_probes(&self) {
        self.inner
            .stopped
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Ok(mut probes) = self.probes.lock() {
            probes.abort_all();
        }
    }

    pub(super) async fn forward(
        &self,
        message: Message,
        protocol: Protocol,
    ) -> Result<Message, NetError> {
        use std::sync::atomic::Ordering::Relaxed;
        let _permit = self
            .inner
            .permits
            .try_acquire()
            .map_err(|_| NetError::from("limite de concurrence upstream"))?;
        self.inner.counters.forwarded.fetch_add(1, Relaxed);
        let transport = usize::from(protocol == Protocol::Tcp);
        for (index, &upstream) in self.inner.upstreams.iter().enumerate() {
            let state = self.inner.states[index][transport].load(Relaxed);
            if state != 0 {
                if state == 1 || state > self.inner.now() {
                    self.inner.counters.upstream_skipped.fetch_add(1, Relaxed);
                    continue;
                }
                if self
                    .inner
                    .states
                    .iter()
                    .any(|states| states[transport].load(Relaxed) == 0)
                {
                    self.probe(index, transport, state, message.clone(), protocol);
                    self.inner.counters.upstream_skipped.fetch_add(1, Relaxed);
                    continue;
                }
                if self.inner.states[index][transport]
                    .compare_exchange(state, 1, Relaxed, Relaxed)
                    .is_err()
                {
                    continue;
                }
            }
            let mut recovery = Recovery {
                inner: self.inner.clone(),
                index,
                transport,
                finished: false,
            };
            let result = self.inner.attempt(upstream, &message, protocol).await;
            recovery.finished = true;
            match result {
                Ok(response) => {
                    self.inner.states[index][transport].store(0, Relaxed);
                    if !matches!(
                        response.response_code,
                        ResponseCode::ServFail | ResponseCode::Refused
                    ) {
                        return Ok(response);
                    }
                }
                Err(error) => {
                    self.inner.failed(index, transport);
                    tracing::debug!(%upstream, %error, "upstream temporairement écarté");
                }
            }
        }
        Err(NetError::from(
            "aucun upstream n'a fourni de réponse utilisable",
        ))
    }

    fn probe(
        &self,
        index: usize,
        transport: usize,
        state: u64,
        message: Message,
        protocol: Protocol,
    ) {
        use std::sync::atomic::Ordering::Relaxed;
        let Ok(permit) = self.inner.permits.clone().try_acquire_owned() else {
            return;
        };
        let Ok(mut probes) = self.probes.lock() else {
            return;
        };
        if self.inner.stopped.load(Relaxed) {
            return;
        }
        while probes.try_join_next().is_some() {}
        if self.inner.states[index][transport]
            .compare_exchange(state, 1, Relaxed, Relaxed)
            .is_err()
        {
            return;
        }
        let recovery = Recovery {
            inner: self.inner.clone(),
            index,
            transport,
            finished: false,
        };
        self.inner.counters.upstream_probes.fetch_add(1, Relaxed);
        probes.spawn(async move {
            let _permit = permit;
            let mut recovery = recovery;
            let result = recovery
                .inner
                .attempt(recovery.inner.upstreams[index], &message, protocol)
                .await;
            recovery.finished = true;
            if result.is_ok() {
                recovery.inner.states[index][transport].store(0, Relaxed);
            } else {
                recovery.inner.failed(index, transport);
            }
        });
    }
}

// En cas d'annulation (arrêt, reload, délai global), ne jamais laisser une sonde réservée.
struct Recovery {
    inner: Arc<Inner>,
    index: usize,
    transport: usize,
    finished: bool,
}
impl Drop for Recovery {
    fn drop(&mut self) {
        if !self.finished {
            self.inner.failed(self.index, self.transport);
        }
    }
}

impl Inner {
    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis())
            .unwrap_or(u64::MAX - 5002)
            .saturating_add(2)
    }
    fn failed(&self, index: usize, transport: usize) {
        use std::sync::atomic::Ordering::Relaxed;
        self.states[index][transport].store(self.now().saturating_add(5000), Relaxed);
        self.counters.upstream_failures.fetch_add(1, Relaxed);
    }
    async fn attempt(
        &self,
        upstream: SocketAddr,
        message: &Message,
        protocol: Protocol,
    ) -> Result<Message, NetError> {
        timeout(self.timeout, self.exchange(upstream, message, protocol))
            .await
            .map_err(|_| NetError::from("timeout upstream"))?
    }

    async fn exchange(
        &self,
        upstream: SocketAddr,
        message: &Message,
        protocol: Protocol,
    ) -> Result<Message, NetError> {
        let response = if protocol == Protocol::Tcp {
            self.tcp(upstream, message).await?
        } else {
            let mut stream = UdpClientStream::builder(upstream, TokioRuntimeProvider::new())
                .with_timeout(Some(self.timeout))
                .with_max_retries(1)
                .build();
            let response = stream
                .send_message(DnsRequest::from(message.clone()))
                .first_answer()
                .await?;
            self.validate(&response, message)?;
            if response.truncation {
                self.tcp(upstream, message).await?
            } else {
                response
            }
        };
        self.validate(&response, message)?;
        if response.truncation {
            return Err(NetError::from("réponse TCP tronquée"));
        }
        Ok(response.into_message())
    }

    async fn tcp(&self, upstream: SocketAddr, message: &Message) -> Result<DnsResponse, NetError> {
        // Le provider possède les tâches Hickory et doit vivre jusqu'à la réponse.
        let provider = TokioRuntimeProvider::new();
        let exchange =
            TcpClientStream::exchange(upstream, None, self.timeout, Some(1), provider.clone())
                .await?;
        exchange
            .send(DnsRequest::from(message.clone()))
            .first_answer()
            .await
    }

    fn validate(&self, response: &Message, request: &Message) -> Result<(), NetError> {
        if response.message_type != MessageType::Response
            || response.op_code != request.op_code
            || response.queries != request.queries
            || response.signature.is_some()
        {
            return Err(NetError::from(
                "réponse upstream incompatible avec la requête",
            ));
        }
        Ok(())
    }
}
