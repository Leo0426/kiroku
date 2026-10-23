//! Test-only controls and assets for the real WebKit/JavaScript bridge checks.
use super::*;
use serde_json::{json, Value};
use std::{borrow::Cow, collections::BTreeMap, fs, path::PathBuf, sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}};
use tauri::{Assets, State, utils::assets::{AssetKey, AssetsIter, CspHash}};
use tokio_util::sync::CancellationToken;

#[path = "native_network_fixture.rs"]
mod network;

#[path = "native_rebuild_bridge.rs"]
mod rebuild;

struct Probe {
    root: PathBuf,
    config_path: PathBuf,
    originals: BTreeMap<PathBuf, Vec<u8>>,
    temp: Mutex<Option<tempfile::TempDir>>,
    announce: Mutex<Option<CancellationToken>>,
    writer: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    starts: AtomicUsize,
    cancels: Mutex<Vec<Value>>,
    network: Mutex<Option<network::Fixture>>,
    rebuild: rebuild::RebuildProbe,
    notifications: Mutex<Vec<Value>>,
}

#[tauri::command]
async fn bridge_network_prepare(app: AppHandle, probe: State<'_, Arc<Probe>>, scan_from_ui: Option<bool>) -> Result<Value, String> {
    let probe = probe.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if state.job_running() { return Err("network fixture prepared while busy".into()); }
        let scan_from_ui = scan_from_ui.unwrap_or(false);
        if scan_from_ui && Settings::default_path() != probe.config_path {
            return Err("UI scan requires the runner's isolated settings directory".into());
        }
        if let Some(previous) = probe.network.lock().unwrap().take() { previous.close()?; }
        let fixture = network::Fixture::new(app.clone());
        let mut settings = fixture.settings(state.settings());
        if scan_from_ui {
            settings.scan.min_video_mb = 0;
            settings.organize_mode = kiroku_core::placement::OrganizeMode::Copy;
            settings.cleanup_source = false;
            settings.dir_template = "{code}".into();
            settings.file_template = "{code}".into();
        }
        *state.settings.lock().unwrap() = settings.clone();
        let mut session = state.session();
        session.scan = if scan_from_ui { Vec::new() } else { fixture.items.clone() };
        session.metas.clear(); session.plans.clear(); session.roots.clear(); session.plan_settings = None;
        let response = json!({"items":fixture.items,"settings":settings,"base":fixture.base,"inputRoot":fixture.input_root()});
        *probe.network.lock().unwrap() = Some(fixture);
        Ok(response)
    }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
fn bridge_network_snapshot(state: State<'_, AppState>, probe: State<'_, Arc<Probe>>) -> Value {
    let mut snapshot = probe.network.lock().unwrap().as_ref().unwrap().snapshot();
    let metas: BTreeMap<_,_> = state.session().metas.iter().map(|(code,metadata)|
        (code.clone(), json!({"title":metadata.fields.title,"source":metadata.provenance.get("title")}))).collect();
    snapshot["metas"] = json!(metas);
    snapshot
}

#[tauri::command]
fn bridge_network_release(probe: State<'_, Arc<Probe>>) {
    probe.network.lock().unwrap().as_ref().unwrap().release();
}

#[tauri::command]
fn bridge_network_hold(state: State<'_, AppState>, probe: State<'_, Arc<Probe>>, code: String) -> Result<(), String> {
    if state.job_running() { return Err("cannot move hold point during a job".into()); }
    probe.network.lock().unwrap().as_ref().unwrap().hold_at(&code)
}

#[tauri::command]
async fn bridge_import_verify(probe: State<'_, Arc<Probe>>, journal_id: String, rolled_back: bool) -> Result<(), String> {
    let probe = probe.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let config = Settings::load(&probe.config_path).map_err(|e|e.to_string())?;
        let network = probe.network.lock().unwrap();
        let fixture = network.as_ref().ok_or("missing input fixture")?;
        if config.source_dirs != vec![fixture.input_root().to_string_lossy().into_owned()] || config.library_root.as_deref()!=probe.root.to_str()
            || config.cleanup_source || config.organize_mode != kiroku_core::placement::OrganizeMode::Copy {
            return Err("scan did not persist the isolated fixture settings".into());
        }
        for code in ["NET-101", "NET-102", "NET-103"] {
            if fs::read(fixture.input_root().join(format!("{code}.mp4"))).map_err(|e|e.to_string())? != b"synthetic network video" {
                return Err(format!("source changed: {code}"));
            }
            let dir = probe.root.join(code);
            if rolled_back {
                if dir.exists() { return Err(format!("rollback left target: {code}")); }
                continue;
            }
            if fs::read(dir.join(format!("{code}.mp4"))).map_err(|e|e.to_string())? != b"synthetic network video" {
                return Err(format!("copy differs: {code}"));
            }
            let nfo = fs::read_to_string(dir.join(format!("{code}.nfo"))).map_err(|e|e.to_string())?;
            let meta:kiroku_core::Metadata = serde_json::from_slice(&fs::read(dir.join("kiroku.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
            if !nfo.contains(&format!("<title>{code} Neutral {code}</title>")) || meta.code != code || meta.fields.title.as_deref()!=Some(format!("Neutral {code}").as_str()) {
                return Err(format!("wrong metadata: {code}"));
            }
        }
        if journal_id.contains(['/', '\\']) { return Err("invalid fixture journal ID".into()); }
        let journal = kiroku_core::journal::Journal::load(&probe.root.join(".kiroku").join(format!("journal-{journal_id}.json"))).map_err(|e|e.to_string())?;
        let status = if rolled_back {kiroku_core::journal::JournalStatus::RolledBack} else {kiroku_core::journal::JournalStatus::Applied};
        if journal.status != status || journal.items.len()!=3 { return Err("wrong import journal".into()); }
        verify_files(&probe, 3, false)
    }).await.map_err(|e|e.to_string())?
}

#[tauri::command]
async fn bridge_task(app: AppHandle, state: State<'_, AppState>, probe: State<'_, Arc<Probe>>,
    request_id: String, delayed: bool) -> Result<Value, String> {
    let (id, cancel) = state.begin_job("bridge-controlled")?;
    let _guard = state::JobGuard { app: app.clone(), id };
    probe.starts.fetch_add(1, Ordering::SeqCst);
    let announce = CancellationToken::new();
    *probe.announce.lock().unwrap() = Some(announce.clone());
    state::emit(&app, "bridge:acquired", id, json!({"requestId":request_id}));
    if delayed {
        tokio::select! { _ = announce.cancelled() => {}, _ = cancel.cancelled() => {} }
    }
    state::emit(&app, "job:started", id, json!({"requestId":request_id}));
    cancel.cancelled().await;
    Ok(json!({"jobId":id,"cancelled":true}))
}

#[tauri::command]
fn bridge_release(probe: State<'_, Arc<Probe>>) {
    if let Some(gate) = probe.announce.lock().unwrap().take() { gate.cancel(); }
}

#[tauri::command]
fn bridge_foreign(app: AppHandle, state: State<'_, AppState>) {
    state::emit(&app, "job:started", state.current_job_id().unwrap() + 1000, json!({"requestId":"unrelated"}));
}

#[tauri::command]
fn bridge_snapshot(state: State<'_, AppState>, probe: State<'_, Arc<Probe>>) -> Value {
    let job = state.job.lock().unwrap();
    json!({"jobId":job.as_ref().map(|job|job.id),"cancelled":job.as_ref().is_some_and(|job|job.cancel.is_cancelled()),
        "starts":probe.starts.load(Ordering::SeqCst),"cancels":*probe.cancels.lock().unwrap(),"notifications":*probe.notifications.lock().unwrap()})
}

#[tauri::command]
async fn bridge_lock_writer(app: AppHandle, probe: State<'_, Arc<Probe>>) -> Result<(), String> {
    let (release, waiting) = std::sync::mpsc::channel();
    assert!(probe.writer.lock().unwrap().replace(release).is_none());
    let (ready, acquired) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _writer = state.settings_writer.lock().unwrap();
        let _ = ready.send(());
        // Test-only bounded gate; never changes the production preview command.
        let _ = waiting.recv_timeout(std::time::Duration::from_secs(15));
    });
    acquired.await.map_err(|error|error.to_string())
}

#[tauri::command]
fn bridge_unlock_writer(probe: State<'_, Arc<Probe>>) {
    if let Some(release) = probe.writer.lock().unwrap().take() { let _ = release.send(()); }
}

fn verify_files(probe: &Probe, journals: usize, exported: bool) -> Result<(), String> {
    for (path, bytes) in &probe.originals {
        let actual=fs::read(path).map_err(|error|error.to_string())?;
        let file=path.file_name().unwrap().to_str().unwrap();
        if !exported || file=="sample.mp4" {
            if actual!=*bytes {return Err(format!("fixture changed: {}",path.display()));}
        } else {
            if actual==*bytes {return Err(format!("export did not change: {}",path.display()));}
            if file=="movie.nfo" {
                let text=String::from_utf8(actual).map_err(|error|error.to_string())?;
                let code=path.parent().unwrap().file_name().unwrap().to_str().unwrap();
                if !text.contains(&format!("<title>{code} Original sample</title>")) || !text.contains("<playcount>7</playcount>") {
                    return Err(format!("exported NFO has wrong title or lost custom XML: {code}"));
                }
            } else {
                let metadata:kiroku_core::Metadata=serde_json::from_slice(&actual).map_err(|error|error.to_string())?;
                if metadata.nfo_language!=kiroku_core::localization::NfoLanguage::Original {
                    return Err("exported sidecar has wrong language".into());
                }
            }
        }
    }
    let count = fs::read_dir(probe.root.join(".kiroku")).map_err(|error|error.to_string())?
        .filter_map(Result::ok).filter(|entry|entry.file_name().to_string_lossy().starts_with("journal-")).count();
    if count != journals { return Err(format!("expected {journals} journals, got {count}")); }
    Ok(())
}

#[tauri::command]
async fn bridge_verify(probe: State<'_, Arc<Probe>>, journals: usize, exported: Option<bool>) -> Result<(), String> {
    let probe = probe.inner().clone();
    tauri::async_runtime::spawn_blocking(move || verify_files(&probe, journals, exported.unwrap_or(false))).await.map_err(|error|error.to_string())?
}

#[tauri::command]
async fn bridge_finish(app: AppHandle, probe: State<'_, Arc<Probe>>, passed: Vec<String>, error: Option<String>) -> Result<(), String> {
    let probe = probe.inner().clone();
    let mut ok = error.is_none() && passed.len() == 11 && !app.state::<AppState>().job_running();
    if let Some(error) = error { eprintln!("BRIDGE FAILED: {error}"); }
    // Failure cleanup releases only our gates/current synthetic or fixture task.
    if let Some(gate) = probe.announce.lock().unwrap().take() { gate.cancel(); }
    if let Some(gate) = probe.writer.lock().unwrap().take() { let _ = gate.send(()); }
    probe.rebuild.close_gate(&app);
    app.state::<AppState>().cancel_job_for(None);
    for _ in 0..100 {
        if !app.state::<AppState>().job_running() { break; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    ok &= !app.state::<AppState>().job_running();
    let handle = app.clone();
    let cleaned = tauri::async_runtime::spawn_blocking(move || {
        let network = probe.network.lock().unwrap().take().map(network::Fixture::close).transpose();
        handle.state::<AppState>().replace_library(None);
        let cleaned = probe.temp.lock().unwrap().take().unwrap().close().map_err(|e|e.to_string());
        network.and(cleaned)
    }).await;
    ok &= matches!(cleaned, Ok(Ok(())));
    if ok { for case in passed { println!("PASS bridge_{case}"); } }
    app.exit(if ok { 0 } else { 1 });
    Ok(())
}

struct StaticAssets(BTreeMap<String, Vec<u8>>);
impl Assets<tauri::Wry> for StaticAssets {
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        self.0.get(key.as_ref()).map(|bytes|Cow::Borrowed(bytes.as_slice()))
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(self.0.iter().map(|(path,bytes)|(Cow::Borrowed(path.as_str()),Cow::Borrowed(bytes.as_slice()))))
    }
    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item=CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}
fn load_assets(root: &Path, dir: &Path, files: &mut BTreeMap<String,Vec<u8>>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        assert!(!entry.file_type().unwrap().is_symlink());
        if path.is_dir() { load_assets(root,&path,files); }
        else { files.insert(format!("/{}",path.strip_prefix(root).unwrap().display()),fs::read(path).unwrap()); }
    }
}

fn control_handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![bridge_task,bridge_release,bridge_foreign,bridge_snapshot,
        bridge_lock_writer,bridge_unlock_writer,bridge_verify,bridge_finish,
        bridge_network_prepare,bridge_network_snapshot,bridge_network_release,bridge_network_hold,bridge_import_verify,
        rebuild::bridge_rebuild_prepare,rebuild::bridge_rebuild_release,rebuild::bridge_rebuild_verify,rebuild::bridge_rebuild_restore]
}

pub fn run(root: PathBuf, library: kiroku_core::library::Library, temp: tempfile::TempDir, assets: PathBuf) {
    let mut originals = BTreeMap::new();
    for code in ["SMPL-001","SMPL-002"] {
        for file in ["movie.nfo","kiroku.json","sample.mp4"] {
            let path=root.join(code).join(file); originals.insert(path.clone(),fs::read(path).unwrap());
        }
    }
    let probe = Arc::new(Probe {root:root.clone(),config_path:assets.join(".config/kiroku/settings.json"), originals, temp:Mutex::new(Some(temp)), announce:Mutex::new(None),
        writer:Mutex::new(None),starts:AtomicUsize::new(0),cancels:Mutex::new(Vec::new()),network:Mutex::new(None),
        rebuild:Default::default(),notifications:Mutex::new(Vec::new())});
    let state=AppState::new(Settings{library_root:Some(root.to_string_lossy().into_owned()),..Default::default()});
    state.replace_library(Some(library));
    let mut context=app_context();
    // Keep production CSP/ACL. Load only the probe page; never use the dev server
    // or start the full App shell. AppStore initialization reads fixture settings.
    context.config_mut().app.windows.clear();
    context.config_mut().build.dev_url=None;
    let mut files=BTreeMap::new(); load_assets(&assets,&assets,&mut files);
    assert!(files.contains_key("/index.html"));
    context.set_assets(Box::new(StaticAssets(files)));
    let production=command_handler();
    let controls=control_handler();
    let observations=probe.clone();
    let trace = std::env::var_os("KIROKU_BRIDGE_TRACE").is_some();
    let app=tauri::Builder::default().manage(state).manage(probe).invoke_handler(move |invoke| {
        if trace { eprintln!("BRIDGE COMMAND {}", invoke.message.command()); }
        if invoke.message.command().starts_with("bridge_") {return controls(invoke);}
        if invoke.message.command()=="job_cancel" {
            if let tauri::ipc::InvokeBody::Json(body)=invoke.message.payload() {
                observations.cancels.lock().unwrap().push(body.get("jobId").cloned().unwrap_or(Value::Null));
            }
        }
        if invoke.message.command()=="notify_finish" {
            if let tauri::ipc::InvokeBody::Json(body)=invoke.message.payload() {
                observations.notifications.lock().unwrap().push(body.clone());
            }
        }
        production(invoke)
    }).on_page_load(|_, payload| eprintln!("BRIDGE PAGE {:?}", payload.event())).build(context).unwrap();
    tauri::WebviewWindowBuilder::new(&app,"main",tauri::WebviewUrl::App("index.html".into()))
        .incognito(true).title("kiroku · WebKit 桥接验收").inner_size(640.0,420.0).build().unwrap();
    app.run(|_,event| { if matches!(event, RunEvent::Ready) { eprintln!("BRIDGE READY"); } });
}
