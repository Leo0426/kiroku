//! Production media commands, synthetic links, and no opener plugin installed.
use super::{
    smoke::{cancel, invoke, journal_count},
    AppState,
};
use kiroku_core::{
    journal::Journal,
    organize::{self, PlanOpts},
    placement::{self, OrganizeMode},
    Metadata,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{Listener, Manager, WebviewWindow};

pub fn run(window: &WebviewWindow, root: &Path) {
    let app = window.app_handle();
    let state = app.state::<AppState>();
    let starts = Arc::new(Mutex::new(Vec::<Value>::new()));
    let events = starts.clone();
    let stop_at_start = Arc::new(AtomicBool::new(true));
    let stop = stop_at_start.clone();
    let owner = window.clone();
    let listener = app.listen("job:started", move |event| {
        let payload: Value = serde_json::from_str(event.payload()).unwrap();
        events.lock().unwrap().push(payload.clone());
        if stop.load(Ordering::SeqCst) {
            assert!(cancel(&owner, payload["jobId"].as_u64().unwrap()));
        }
    });
    for command in ["play", "reveal"] {
        let request = format!("media-early-{command}");
        // Missing film also proves cancellation is checked before lookup.
        let error = invoke(
            window,
            command,
            json!({"code":"MISSING-999","requestId":request}),
        )
        .unwrap_err();
        assert!(
            error.as_str().unwrap().contains("取消"),
            "{command}: {error}"
        );
        assert!(!state.job_running());
        assert_eq!(
            starts
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e["requestId"] == request)
                .count(),
            1
        );
    }
    let before = starts.lock().unwrap().len();
    for command in ["play", "reveal"] {
        let error = invoke(window, command, json!({"code":"MISSING-999"})).unwrap_err();
        assert!(error.as_str().unwrap().contains("影片不在媒体库中"));
        assert!(!state.job_running());
    }
    assert_eq!(starts.lock().unwrap().len(), before);
    println!("PASS media_early_cancel commands=2 lookup_skipped=true optional_request_id=true");

    // Register an actual symlink with the same journal/placement machinery used
    // by import, keeping the source in another owned temporary directory.
    let source = tempfile::tempdir().unwrap();
    let path = source.path().canonicalize().unwrap().join("MED-101.mp4");
    let bytes = vec![7; 1024 * 1024];
    fs::write(&path, &bytes).unwrap();
    let code = kiroku_core::code::parse_code("MED-101.mp4").unwrap();
    let mut plan = organize::plan_with(
        &path,
        &code,
        &Metadata {
            code: code.id.clone(),
            ..Default::default()
        },
        root,
        "{code}",
        true,
        PlanOpts {
            mode: OrganizeMode::Symlink,
            ..Default::default()
        },
    );
    let mut journal = Journal::new(root);
    placement::bind_plan(&mut plan, &journal).unwrap();
    let outcome = journal.execute(
        &plan,
        &mut |_| unreachable!("fixture has no remote assets"),
        &|| false,
    );
    assert!(outcome.error.is_none());
    let dir = root.join("MED-101");
    state.lib().as_ref().unwrap().reindex_dir(&dir).unwrap();
    let detail = invoke(window, "library_get", json!({"code":"MED-101"})).unwrap();
    let link = PathBuf::from(detail["files"][0]["path"].as_str().unwrap());
    let nfo_path = kiroku_core::assets::nfo_path(&dir).unwrap();
    let original_nfo = fs::read(&nfo_path).unwrap();
    let original_meta = fs::read(dir.join("kiroku.json")).unwrap();
    let original_journal = fs::read(journal.path()).unwrap();
    let count = journal_count(root);
    use std::os::unix::fs::MetadataExt;
    let link_inode = fs::symlink_metadata(&link).unwrap().ino();
    assert_eq!(fs::read_link(&link).unwrap(), path);

    stop_at_start.store(false, Ordering::SeqCst);
    let progress = Arc::new(Mutex::new(Vec::<Value>::new()));
    let reports = progress.clone();
    let owner = window.clone();
    let progress_listener = app.listen("media:progress", move |event| {
        let payload: Value = serde_json::from_str(event.payload()).unwrap();
        reports.lock().unwrap().push(payload.clone());
        // Small fixture may only report at EOF because production throttles
        // progress. Cancel after bytes were read but before opening the path.
        if payload["doneBytes"].as_u64().unwrap() > 0 {
            assert!(cancel(&owner, payload["jobId"].as_u64().unwrap()));
        }
    });
    for command in ["play", "reveal"] {
        progress.lock().unwrap().clear();
        let request = format!("media-hash-{command}");
        let error = invoke(
            window,
            command,
            json!({"code":"MED-101","part":1,"requestId":request}),
        )
        .unwrap_err();
        assert!(
            error.as_str().unwrap().contains("取消"),
            "{command}: {error}"
        );
        let started = starts.lock().unwrap();
        let owned = started.iter().find(|e| e["requestId"] == request).unwrap()["jobId"]
            .as_u64()
            .unwrap();
        drop(started);
        let observed = progress.lock().unwrap();
        assert!(observed
            .iter()
            .any(|e| e["doneBytes"].as_u64().unwrap() > 0));
        for event in observed.iter() {
            assert_eq!(event["jobId"], owned);
            assert_eq!(event["code"], "MED-101");
            assert_eq!(event["kind"], command);
            assert_eq!(event["root"], root.display().to_string());
            if event["doneBytes"].as_u64().unwrap() > 0 {
                assert_eq!(event["totalBytes"], bytes.len());
            }
        }
        drop(observed);
        assert!(!state.job_running());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::symlink_metadata(&link).unwrap().ino(), link_inode);
        assert_eq!(fs::read_link(&link).unwrap(), path);
        assert_eq!(fs::read(&nfo_path).unwrap(), original_nfo);
        assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), original_meta);
        assert_eq!(fs::read(journal.path()).unwrap(), original_journal);
        assert_eq!(journal_count(root), count);
        assert_eq!(
            invoke(window, "library_get", json!({"code":"MED-101"})).unwrap(),
            detail
        );
    }
    app.unlisten(listener);
    app.unlisten(progress_listener);
    // No tauri-plugin-opener is installed by this harness. An accidental call
    // would fail before accessing an external program, even on a regression.
    println!("PASS media_hash_cancel commands=2 source_link_metadata_unchanged=true index_unchanged=true");
}
