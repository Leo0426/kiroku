//! Loopback-only input/source fixture for the explicit WebKit smoke tool.
use kiroku_core::{
    code::ContentType,
    pipeline::{self, ScanItem},
    settings::{Settings, SourceCfg, SourceProxy},
};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct ServerState {
    stop: AtomicBool,
    holding: AtomicBool,
    requests: Mutex<Vec<String>>,
    hold_path: Mutex<String>,
}

pub struct Fixture {
    temp: Option<tempfile::TempDir>,
    pub base: String,
    pub items: Vec<ScanItem>,
    server: Arc<ServerState>,
    release: std::sync::mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Fixture {
    pub fn input_root(&self) -> std::path::PathBuf {
        self.temp.as_ref().unwrap().path().to_path_buf()
    }
    pub fn new(app: AppHandle) -> Self {
        let temp = tempfile::tempdir().unwrap();
        for code in ["NET-101", "NET-102", "NET-103"] {
            fs::write(
                temp.path().join(format!("{code}.mp4")),
                b"synthetic network video",
            )
            .unwrap();
        }
        let mut settings = Settings::default();
        settings.scan.min_video_mb = 0;
        let scanned = pipeline::scan(
            &[temp.path().to_path_buf()],
            None,
            &settings.video_filter(),
            &CancellationToken::new(),
            &mut |_| {},
        );
        assert!(scanned.summary.complete);
        assert_eq!(scanned.items.len(), 3);
        let mut items = scanned.items;
        items.sort_by(|a, b| {
            a.code
                .as_ref()
                .unwrap()
                .id
                .cmp(&b.code.as_ref().unwrap().id)
        });
        for (id, item) in items.iter_mut().enumerate() {
            item.id = id as u32;
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = Arc::new(ServerState {
            hold_path: Mutex::new("/NET-102".into()),
            ..Default::default()
        });
        let worker_state = server.clone();
        let (release, released) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut held_once = false;
            while !worker_state.stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("loopback accept: {e}"),
                };
                // Darwin inherits O_NONBLOCK from the listener on accept.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&chunk[..n]),
                    }
                }
                if !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    continue;
                }
                let request = String::from_utf8_lossy(&request);
                let path = request.split_whitespace().nth(1).unwrap();
                assert!(
                    ["/NET-101", "/NET-102", "/NET-103"].contains(&path),
                    "unexpected fixture request: {path}"
                );
                worker_state.requests.lock().unwrap().push(path.to_string());
                let hold = path == worker_state.hold_path.lock().unwrap().as_str();
                if hold && !held_once {
                    held_once = true;
                    stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\nx\r\n").unwrap();
                    worker_state.holding.store(true, Ordering::SeqCst);
                    let job_id = app
                        .state::<crate::state::AppState>()
                        .current_job_id()
                        .unwrap();
                    app.emit(
                        "bridge:network_waiting",
                        json!({"jobId":job_id,"path":path}),
                    )
                    .unwrap();
                    // Keep EOF withheld until JavaScript has observed cancelled done.
                    let _ = released.recv_timeout(Duration::from_secs(15));
                    worker_state.holding.store(false, Ordering::SeqCst);
                } else {
                    let code = path.trim_start_matches('/');
                    let html =
                        format!("<div class=\"container\"><h3>{code} Neutral {code}</h3></div>");
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()).unwrap();
                }
            }
        });
        Self {
            temp: Some(temp),
            base,
            items,
            server,
            release,
            worker: Some(worker),
        }
    }

    pub fn settings(&self, mut settings: Settings) -> Settings {
        settings.routes.clear();
        settings
            .routes
            .insert(ContentType::Censored, vec!["javbus".into()]);
        settings.sources = vec![SourceCfg {
            id: "javbus".into(),
            enabled: true,
            base: self.base.clone(),
            cookie: String::new(),
            api_base: String::new(),
            proxy: SourceProxy::Direct,
            solver: false,
        }];
        settings.min_interval_secs = 0;
        settings.proxy = None;
        settings.ai.enabled = false;
        settings.notify_on_finish = false;
        settings.plex.enabled = false;
        settings.media_server.enabled = false;
        settings
    }

    pub fn snapshot(&self) -> Value {
        json!({"requests":*self.server.requests.lock().unwrap(),"holding":self.server.holding.load(Ordering::SeqCst)})
    }
    pub fn release(&self) {
        self.release.send(()).unwrap();
    }
    pub fn hold_at(&self, code: &str) -> Result<(), String> {
        if !self.server.requests.lock().unwrap().is_empty()
            || !self.items.iter().any(|item| {
                item.code
                    .as_ref()
                    .is_some_and(|item_code| item_code.id == code)
            })
        {
            return Err("hold point must be a scanned fixture code before requests start".into());
        }
        *self.server.hold_path.lock().unwrap() = format!("/{code}");
        Ok(())
    }
    fn stop(&mut self) -> Result<(), String> {
        self.server.stop.store(true, Ordering::SeqCst);
        let _ = self.release.send(());
        if let Some(worker) = self.worker.take() {
            worker.join().map_err(|_| "loopback worker panicked")?;
        }
        Ok(())
    }
    pub fn close(mut self) -> Result<(), String> {
        let untouched = self
            .items
            .iter()
            .all(|item| fs::read(&item.path).ok().as_deref() == Some(b"synthetic network video"));
        let stopped = self.stop();
        let cleaned = self.temp.take().unwrap().close().map_err(|e| e.to_string());
        if !untouched {
            return Err("network fixture input changed".into());
        }
        stopped.and(cleaned)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
