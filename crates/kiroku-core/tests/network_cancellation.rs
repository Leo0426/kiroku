//! Cancellation at source dispatch and real loopback transport boundaries.
use kiroku_core::{
    code::{parse_code, Code, ContentType},
    metadata::PartialMetadata,
    net::Http,
    pipeline::{scrape_all, ScrapeError, Scraper},
    settings::{Settings, SourceCfg, SourceProxy},
    source::{Source, SourceError},
};
use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn settings(base: &str) -> Settings {
    let mut settings = Settings {
        min_interval_secs: 0,
        ..Default::default()
    };
    settings.routes.clear();
    settings
        .routes
        .insert(ContentType::Censored, vec!["fixture".into()]);
    settings.sources = vec![SourceCfg {
        id: "fixture".into(),
        enabled: true,
        base: base.into(),
        cookie: String::new(),
        api_base: String::new(),
        proxy: SourceProxy::Direct,
        solver: false,
    }];
    settings
}

struct ReadySource(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl Source for ReadySource {
    fn id(&self) -> &'static str {
        "fixture"
    }
    fn supports(&self, _: &Code) -> bool {
        true
    }
    async fn fetch(&self, _: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(PartialMetadata {
            title: Some("Neutral sample".into()),
            ..Default::default()
        })
    }
    async fn fetch_detail(
        &self,
        _: &str,
        code: &Code,
        http: &Http,
    ) -> Option<Result<(PartialMetadata, String), SourceError>> {
        Some(
            self.fetch(code, http)
                .await
                .map(|metadata| (metadata, code.id.clone())),
        )
    }
}

#[tokio::test]
async fn cancellation_from_progress_does_not_poll_a_ready_source() {
    let calls = Arc::new(AtomicUsize::new(0));
    let scraper = Scraper::with_sources(
        settings("https://fixture.example"),
        vec![Box::new(ReadySource(calls.clone()))],
    )
    .unwrap();
    let code = parse_code("SMPL-001").unwrap();
    // Both branches are ready in the old unbiased select; repeated attempts
    // expose its random choice. Correct behavior is independent of that order.
    for _ in 0..64 {
        let cancel = CancellationToken::new();
        let result = scraper
            .scrape_item(&code, &cancel, &|_| cancel.cancel())
            .await;
        assert!(
            matches!(result, Err(ScrapeError::Cancelled)),
            "stop already requested: {result:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "must not start source work after stop"
        );
    }
}

#[tokio::test]
async fn precancelled_detail_does_not_poll_a_ready_source() {
    let calls = Arc::new(AtomicUsize::new(0));
    let scraper = Scraper::with_sources(
        settings("https://fixture.example"),
        vec![Box::new(ReadySource(calls.clone()))],
    )
    .unwrap();
    let code = parse_code("SMPL-001").unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    for _ in 0..64 {
        let result = scraper
            .scrape_url(&code, "https://fixture.example/detail", &cancel)
            .await;
        assert!(matches!(result, Err(ScrapeError::Cancelled)));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "cancelled detail request must not start source work"
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Response {
    HoldHeaders,
    HoldBody,
    Retry,
    Immediate,
}

/// A test-only server with an explicit release for the held response. Drop also
/// stops and joins the worker on assertion failure; no detached listener survives.
struct Loopback {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    holding: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    release: std::sync::mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
    received: tokio::sync::mpsc::UnboundedReceiver<String>,
}

impl Loopback {
    fn new(response: Response) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let holding = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (release, released) = std::sync::mpsc::channel();
        let (sent, received) = tokio::sync::mpsc::unbounded_channel();
        let (worker_requests, worker_holding, worker_stop) =
            (requests.clone(), holding.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("fixture accept: {e}"),
                };
                // macOS inherits the listener's nonblocking flag on accept;
                // read timeouts alone do not make an accepted stream blocking.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let Some(path) = request.split_whitespace().nth(1) else {
                    continue;
                };
                worker_requests.lock().unwrap().push(path.to_string());
                let held = path == "/SMPL-002"
                    && matches!(response, Response::HoldHeaders | Response::HoldBody);
                worker_holding.store(held, Ordering::SeqCst);
                if path == "/SMPL-002" && response == Response::Retry {
                    stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 30\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else if response == Response::HoldBody && held {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\nx\r\n").unwrap();
                } else if !held {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 14\r\nConnection: close\r\n\r\nNeutral sample").unwrap();
                }
                let _ = sent.send(path.to_string());
                if held {
                    let _ = released.recv_timeout(Duration::from_secs(10));
                    worker_holding.store(false, Ordering::SeqCst);
                }
            }
        });
        Self {
            base,
            requests,
            holding,
            stop,
            release,
            worker: Some(worker),
            received,
        }
    }

    fn scraper(&self, interval_secs: u64) -> Scraper {
        let mut cfg = settings(&self.base);
        cfg.min_interval_secs = interval_secs;
        Scraper::with_sources(cfg, vec![Box::new(HttpSource(self.base.clone()))]).unwrap()
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.release.send(());
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !std::thread::panicking() {
                result.unwrap();
            }
        }
    }
}

struct HttpSource(String);
#[async_trait::async_trait]
impl Source for HttpSource {
    fn id(&self) -> &'static str {
        "fixture"
    }
    fn supports(&self, _: &Code) -> bool {
        true
    }
    async fn fetch(&self, code: &Code, http: &Http) -> Result<PartialMetadata, SourceError> {
        let page = http
            .get_text(&format!("{}/{}", self.0, code.id))
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        if page.status != 200 {
            return Err(SourceError::Network(format!("HTTP {}", page.status)));
        }
        Ok(PartialMetadata {
            title: Some(page.body),
            ..Default::default()
        })
    }
}

// Observe the existing retry log to synchronize at the backoff, without a
// production test hook or an arbitrary sleep hoping the response was consumed.
struct RetryLog(OnceLock<tokio::sync::mpsc::UnboundedSender<String>>);
static RETRY_LOG: RetryLog = RetryLog(OnceLock::new());
impl log::Log for RetryLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target() == "kiroku::net" && metadata.level() == log::Level::Warn
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            if let Some(sender) = self.0.get() {
                let _ = sender.send(record.args().to_string());
            }
        }
    }
    fn flush(&self) {}
}

async fn cancels_loopback_batch(response: Response) {
    let mut server = Loopback::new(response);
    let scraper = server.scraper(0);
    let codes = ["SMPL-001", "SMPL-002", "SMPL-003"].map(|id| parse_code(id).unwrap());
    let cancel = CancellationToken::new();
    let mut retry_messages = if response == Response::Retry {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        RETRY_LOG.0.set(sender).unwrap();
        log::set_logger(&RETRY_LOG).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        Some(receiver)
    } else {
        None
    };
    let retry_url = format!("{}/SMPL-002 ", server.base);
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(scrape_all(&scraper, &codes, &cancel, &|_| {}), async {
            while server.received.recv().await.unwrap() != "/SMPL-002" {}
            if let Some(messages) = retry_messages.as_mut() {
                while !messages.recv().await.unwrap().starts_with(&retry_url) {}
            }
            cancel.cancel();
        })
        .0
    })
    .await
    .expect("stop must interrupt network waits before EOF or the 30s retry");
    if matches!(response, Response::HoldHeaders | Response::HoldBody) {
        assert!(
            server.holding.load(Ordering::SeqCst),
            "returned before the held response ended"
        );
        server.release.send(()).unwrap();
    }
    assert_eq!(result.len(), 2, "the next item must not start after stop");
    assert_eq!(
        result["SMPL-001"].as_ref().unwrap().fields.title.as_deref(),
        Some("Neutral sample")
    );
    assert!(matches!(result["SMPL-002"], Err(ScrapeError::Cancelled)));

    let next = tokio::time::timeout(
        Duration::from_secs(2),
        scraper.scrape_item(
            &parse_code("SMPL-004").unwrap(),
            &CancellationToken::new(),
            &|_| {},
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(next.fields.title.as_deref(), Some("Neutral sample"));
    assert_eq!(
        *server.requests.lock().unwrap(),
        ["/SMPL-001", "/SMPL-002", "/SMPL-004"]
    );
}

#[tokio::test]
async fn stop_interrupts_response_headers_and_keeps_completed_items() {
    cancels_loopback_batch(Response::HoldHeaders).await;
}

#[tokio::test]
async fn stop_returns_while_partial_response_is_still_open() {
    cancels_loopback_batch(Response::HoldBody).await;
}

#[tokio::test]
async fn stop_interrupts_retry_after_without_another_attempt() {
    cancels_loopback_batch(Response::Retry).await;
}

#[tokio::test]
async fn stop_releases_host_throttle_for_the_next_request() {
    let server = Loopback::new(Response::Immediate);
    let scraper = server.scraper(2);
    scraper
        .scrape_item(
            &parse_code("SMPL-001").unwrap(),
            &CancellationToken::new(),
            &|_| {},
        )
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let code = parse_code("SMPL-002").unwrap();
    let mut waiting = std::pin::pin!(scraper.scrape_item(&code, &cancel, &|_| {}));
    // Poll through source dispatch into the host's interval wait before stopping.
    assert!(futures_util::poll!(&mut waiting).is_pending());
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_millis(250), waiting)
        .await
        .unwrap();
    assert!(matches!(result, Err(ScrapeError::Cancelled)));
    assert_eq!(*server.requests.lock().unwrap(), ["/SMPL-001"]);

    let next = tokio::time::timeout(
        Duration::from_secs(3),
        scraper.scrape_item(
            &parse_code("SMPL-004").unwrap(),
            &CancellationToken::new(),
            &|_| {},
        ),
    )
    .await
    .expect("cancelled wait must release its host lock")
    .unwrap();
    assert_eq!(next.fields.title.as_deref(), Some("Neutral sample"));
    assert_eq!(*server.requests.lock().unwrap(), ["/SMPL-001", "/SMPL-004"]);
}
