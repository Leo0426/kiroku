//! Explicit macOS smoke check: real Wry command dispatch, synthetic temporary files.
#[cfg(target_os = "macos")]
include!("../apps/desktop/src-tauri/src/lib.rs");

#[cfg(target_os = "macos")]
#[path = "native_webview_bridge.rs"]
mod bridge;

#[cfg(target_os = "macos")]
#[path = "native_catalog_smoke.rs"]
mod catalog_smoke;

#[cfg(target_os = "macos")]
#[path = "native_media_smoke.rs"]
mod media_smoke;

#[cfg(target_os = "macos")]
mod smoke {
    use super::*;
    use serde_json::{json, Value};
    use std::{fs, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};
    use tauri::{Listener, WebviewWindow};

    // Unlike directly calling commands, this exercises production command lookup,
    // argument decoding, State/AppHandle injection, and response serialization.
    pub(super) fn invoke(window: &WebviewWindow, command: &str, body: Value) -> Result<Value, Value> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let webview: &tauri::Webview<tauri::Wry> = window.as_ref();
        webview.clone().on_message(tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: window.app_handle().invoke_key().to_owned(),
        }, Box::new(move |_, _, response, _, _| { tx.send(response).unwrap(); }));
        match rx.recv_timeout(Duration::from_secs(10)).expect("IPC response timed out") {
            tauri::ipc::InvokeResponse::Ok(body) => Ok(body.deserialize().unwrap()),
            tauri::ipc::InvokeResponse::Err(tauri::ipc::InvokeError(error)) => Err(error),
        }
    }

    pub(super) fn cancel(window: &WebviewWindow, id: u64) -> bool {
        invoke(window, "job_cancel", json!({"jobId":id})).unwrap().as_bool().unwrap()
    }

    pub fn run(window: WebviewWindow, root: std::path::PathBuf) {
        let app = window.app_handle();
        let state = app.state::<AppState>();
        let codes = ["SMPL-001", "SMPL-002"];
        let dirs: Vec<_> = codes.iter().map(|code| root.join(code)).collect();
        let before: Vec<_> = dirs.iter().map(|dir| (
            fs::read(dir.join("movie.nfo")).unwrap(),
            fs::read(dir.join("kiroku.json")).unwrap(),
        )).collect();
        let started = Arc::new(Mutex::new(Vec::<Value>::new()));
        let cancel_request = Arc::new(Mutex::new(None::<String>));
        let starts = started.clone();
        let request = cancel_request.clone();
        let cancel_window = window.clone();
        let start_listener = app.listen("job:started", move |event| {
            let payload: Value = serde_json::from_str(event.payload()).unwrap();
            assert_eq!(cancel_window.app_handle().state::<AppState>().current_job_id(), payload["jobId"].as_u64());
            starts.lock().unwrap().push(payload.clone());
            let should_cancel = request.lock().unwrap().as_ref()
                .is_some_and(|request| payload["requestId"] == *request);
            if should_cancel {
                assert!(cancel(&cancel_window, payload["jobId"].as_u64().unwrap()));
            }
        });

        let preview = invoke(&window, "nfo_export_preview", json!({
            "codes":codes,"language":"original","requestId":"prepared"
        })).unwrap();
        assert_eq!(preview["items"].as_array().unwrap().len(), 2);
        let token = preview["token"].as_str().unwrap().to_owned();
        let (busy_id, busy_cancel) = state.begin_job("smoke-owner").unwrap();
        let busy_guard = state::JobGuard { app: app.clone(), id: busy_id };
        let previous_starts = started.lock().unwrap().len();
        let busy_commands = [
            ("sources_check", json!({"ids":[]})),
            ("library_health", json!({})),
            ("maintenance_preview", json!({})),
            ("maintenance_apply", json!({"token":"unused","selected":[]})),
            ("append_targets", json!({"code":codes[0]})),
            ("append_target_videos", json!({"code":codes[0],"target":dirs[0]})),
            ("append_preview", json!({"code":codes[0],"inputs":[],"target":dirs[0]})),
            ("append_apply", json!({"token":"unused"})),
            ("metadata_update_preview", json!({"code":codes[0],"update":{"kind":"favorite","favorite":true}})),
            ("artwork_open", json!({"code":codes[0]})),
            ("artwork_preview", json!({"token":"unused","root":root,"code":codes[0],"crop":{"x":0,"y":0,"width":1,"height":1}})),
            ("nfo_export_preview", json!({"codes":codes,"language":"original"})),
            ("nfo_export_apply", json!({"token":token,"root":root})),
            ("catalog_inventory", json!({})),
            ("catalog_preview", json!({"dirs":dirs})),
            ("catalog_apply", json!({"token":"unused"})),
            ("play", json!({"code":codes[0]})),
            ("reveal", json!({"code":codes[0]})),
        ];
        for (command, mut args) in busy_commands {
            args["requestId"] = json!(format!("busy-{command}"));
            let error = invoke(&window, command, args).unwrap_err();
            assert!(error.as_str().unwrap().contains("已有任务在运行"), "{command}: {error}");
            assert_eq!(state.current_job_id(), Some(busy_id));
            assert!(!busy_cancel.is_cancelled());
        }
        assert_eq!(started.lock().unwrap().len(), previous_starts);
        assert!(!cancel(&window, busy_id + 1));
        assert!(!busy_cancel.is_cancelled());
        drop(busy_guard);
        println!("PASS busy_18_commands no_start_event=true owner_untouched=true");

        // Cancel before apply can consume its prepared candidate. A busy reject
        // above must have preserved the same server-side preview token.
        *cancel_request.lock().unwrap() = Some("cancel-apply".into());
        let stopped = invoke(&window, "nfo_export_apply", json!({
            "token":token,"root":root,"requestId":"cancel-apply"
        })).unwrap();
        assert_eq!(stopped["cancelled"], true);
        assert!(stopped["results"].as_array().unwrap().is_empty());
        assert!(stopped["failed"].as_array().unwrap().is_empty());
        assert!(!state.job_running());
        assert_files(&dirs, &before);
        assert_eq!(journal_count(&root), 0);
        println!("PASS cancel_before_apply writes=0 journals=0 busy_preserved_token=true");

        let early = [
            ("append_targets", json!({"code":codes[0]})),
            ("append_target_videos", json!({"code":codes[0],"target":dirs[0]})),
            ("append_preview", json!({"code":codes[0],"inputs":[],"target":dirs[0]})),
            ("metadata_update_preview", json!({"code":codes[0],"update":{"kind":"favorite","favorite":true}})),
            ("nfo_export_preview", json!({"codes":codes,"language":"original"})),
        ];
        for (command, mut args) in early {
            let request_id = format!("cancel-{command}");
            *cancel_request.lock().unwrap() = Some(request_id.clone());
            args["requestId"] = json!(request_id);
            let error = invoke(&window, command, args).unwrap_err();
            assert!(error.as_str().unwrap().contains("取消"), "{command}: {error}");
            let events = started.lock().unwrap();
            assert_eq!(events.iter().filter(|event| event["requestId"] == request_id).count(), 1);
            assert!(!state.job_running());
        }
        *cancel_request.lock().unwrap() = Some("cancel-health".into());
        let health = invoke(&window, "library_health", json!({"requestId":"cancel-health"})).unwrap();
        assert_eq!(health["cancelled"], true);
        assert_eq!(health["scanned"], 0);
        assert!(!state.job_running());
        assert_files(&dirs, &before);
        assert_eq!(journal_count(&root), 0);
        println!("PASS early_cancel_6_commands files_unchanged=true slot_released=true");

        *cancel_request.lock().unwrap() = None;
        // Optional requestId keeps old callers working; explicitly empty sources
        // prevents network access, independently of default route configuration.
        let previous_starts = started.lock().unwrap().len();
        assert_eq!(invoke(&window, "sources_check", json!({"ids":[]})).unwrap(), json!([]));
        assert_eq!(started.lock().unwrap().len(), previous_starts);
        println!("PASS optional_request_id empty_sources=true");

        let preview = invoke(&window, "nfo_export_preview", json!({
            "codes":codes,"language":"original","requestId":"partial-preview"
        })).unwrap();
        let progress = Arc::new(Mutex::new(Vec::<Value>::new()));
        let observations = progress.clone();
        let stop_after_first = Arc::new(AtomicBool::new(true));
        let should_stop = stop_after_first.clone();
        let progress_window = window.clone();
        let progress_listener = app.listen("nfo:export", move |event| {
            let payload: Value = serde_json::from_str(event.payload()).unwrap();
            observations.lock().unwrap().push(payload.clone());
            if payload["completed"] == 1 && should_stop.swap(false, Ordering::SeqCst) {
                assert!(cancel(&progress_window, payload["jobId"].as_u64().unwrap()));
            }
        });
        let partial = invoke(&window, "nfo_export_apply", json!({
            "token":preview["token"],"root":root,"requestId":"partial-apply"
        })).unwrap();
        assert_eq!(partial["cancelled"], true);
        assert!(partial["failed"].as_array().unwrap().is_empty());
        assert_eq!(partial["results"].as_array().unwrap().len(), 1);
        let starts = started.lock().unwrap();
        let announced = starts.iter().find(|event| event["requestId"] == "partial-apply").unwrap();
        assert_eq!(announced["jobId"], partial["jobId"]);
        drop(starts);
        let first = &partial["results"][0];
        assert_eq!(first["code"], codes[0]);
        assert!(first["error"].is_null() && first["indexError"].is_null());
        let events = progress.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["jobId"], partial["jobId"]);
        drop(events);
        let nfo = fs::read_to_string(dirs[0].join("movie.nfo")).unwrap();
        assert_ne!(nfo.as_bytes(), before[0].0.as_slice());
        assert!(nfo.contains("<title>SMPL-001 Original sample</title>"));
        assert!(nfo.contains("<playcount>7</playcount>"));
        assert_ne!(fs::read(dirs[0].join("kiroku.json")).unwrap(), before[0].1);
        assert_eq!(fs::read(dirs[0].join("sample.mp4")).unwrap(), b"synthetic video bytes");
        assert_files(&dirs[1..], &before[1..]);
        assert_eq!(journal_count(&root), 1);
        assert!(!state.job_running());
        let detail = invoke(&window, "library_get", json!({"code":codes[0]})).unwrap();
        assert_eq!(detail["meta"]["nfo_language"], "original");
        let untouched = invoke(&window, "library_get", json!({"code":codes[1]})).unwrap();
        assert_eq!(untouched["meta"]["nfo_language"], "current");
        println!("PASS partial_apply completed=1 cancelled=true journals=1 index_updated=true");

        let (next, next_cancel) = state.begin_job("smoke-next").unwrap();
        let next_guard = state::JobGuard { app: app.clone(), id: next };
        assert!(!cancel(&window, partial["jobId"].as_u64().unwrap()));
        assert!(!next_cancel.is_cancelled());
        assert!(cancel(&window, next));
        assert!(next_cancel.is_cancelled());
        drop(next_guard);
        let rolled_back = invoke(&window, "rollback", json!({"journalId":first["journalId"]})).unwrap();
        assert!(rolled_back["failures"].as_array().unwrap().is_empty());
        assert!(rolled_back["indexError"].is_null());
        assert_files(&dirs, &before);
        let detail = invoke(&window, "library_get", json!({"code":codes[0]})).unwrap();
        assert_eq!(detail["meta"]["nfo_language"], "current");
        assert!(!state.job_running());
        println!("PASS rollback original_bytes=true index_restored=true stale_cancel_rejected=true");
        app.unlisten(start_listener);
        app.unlisten(progress_listener);
        catalog_smoke::run(&window, &root);
        media_smoke::run(&window, &root);
    }

    pub(super) fn assert_files(dirs: &[std::path::PathBuf], before: &[(Vec<u8>, Vec<u8>)]) {
        for (dir, (nfo, sidecar)) in dirs.iter().zip(before) {
            assert_eq!(&fs::read(dir.join("movie.nfo")).unwrap(), nfo);
            assert_eq!(&fs::read(dir.join("kiroku.json")).unwrap(), sidecar);
            assert_eq!(fs::read(dir.join("sample.mp4")).unwrap(), b"synthetic video bytes");
        }
    }

    pub(super) fn journal_count(root: &Path) -> usize {
        fs::read_dir(root.join(".kiroku")).unwrap().filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("journal-")).count()
    }

    pub fn fixture(root: &Path) -> kiroku_core::library::Library {
        let lib = kiroku_core::library::Library::open(root).unwrap();
        for code in ["SMPL-001", "SMPL-002"] {
            let dir = root.join(code);
            fs::create_dir(&dir).unwrap();
            let mut metadata = kiroku_core::Metadata { code: code.into(), ..Default::default() };
            metadata.fields.title = Some("Localized sample".into());
            metadata.text_versions.original.title = Some("Original sample".into());
            let nfo = kiroku_core::nfo::to_nfo(&metadata).replace("</movie>", "<playcount>7</playcount></movie>");
            fs::write(dir.join("movie.nfo"), nfo).unwrap();
            fs::write(dir.join("kiroku.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
            fs::write(dir.join("sample.mp4"), b"synthetic video bytes").unwrap();
            lib.reindex_dir(&dir).unwrap();
        }
        lib
    }
}

#[cfg(target_os = "macos")]
fn main() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let library = smoke::fixture(&root);
    let mut args = std::env::args().skip(1);
    if let Some(mode) = args.next() {
        assert_eq!(mode, "--webview");
        let assets = args.next().expect("--webview requires an assets directory");
        assert!(args.next().is_none());
        bridge::run(root, library, temp, assets.into());
        return;
    }
    let state = AppState::new(Settings { library_root: Some(root.to_string_lossy().into_owned()), ..Default::default() });
    state.replace_library(Some(library));
    let app = tauri::Builder::default().manage(state).invoke_handler(command_handler())
        .build(tauri::test::mock_context(tauri::test::noop_assets())).unwrap();
    let window = tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
        .title("kiroku · 临时任务验收").inner_size(640.0, 420.0).build().unwrap();
    let mut input = Some((window, root, temp));
    app.run(move |handle, event| {
        if let RunEvent::Ready = event {
            let (window, root, temp) = input.take().unwrap();
            let handle = handle.clone();
            std::thread::spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| smoke::run(window, root)));
                // Close SQLite connections before removing our owned temporary tree.
                handle.state::<AppState>().replace_library(None);
                temp.close().unwrap();
                handle.exit(if outcome.is_ok() { 0 } else { 1 });
            });
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("native_job_smoke requires macOS");
    std::process::exit(1);
}
