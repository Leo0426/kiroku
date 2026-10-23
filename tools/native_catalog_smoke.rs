//! Name-rule cancellation against production commands and a temporary library.
use super::{
    smoke::{assert_files, cancel, invoke, journal_count},
    AppState,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};
use tauri::{Listener, Manager, WebviewWindow};

pub fn run(window: &WebviewWindow, root: &Path) {
    let app = window.app_handle();
    let state = app.state::<AppState>();
    let dirs = [root.join("SMPL-001"), root.join("SMPL-002")];
    // Extend only these owned fixtures; settings remain in memory.
    for dir in &dirs {
        let mut meta: kiroku_core::Metadata =
            serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap();
        meta.fields.studio = Some("Original studio".into());
        fs::write(dir.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        let nfo =
            kiroku_core::nfo::to_nfo(&meta).replace("</movie>", "<playcount>7</playcount></movie>");
        fs::write(dir.join("movie.nfo"), nfo).unwrap();
        state.lib().as_ref().unwrap().reindex_dir(dir).unwrap();
    }
    state
        .settings
        .lock()
        .unwrap()
        .name_rules
        .studio
        .push(kiroku_core::names::NameRule {
            canonical: "Canonical studio".into(),
            aliases: vec!["Original studio".into()],
        });
    let before: Vec<_> = dirs
        .iter()
        .map(|dir| {
            (
                fs::read(dir.join("movie.nfo")).unwrap(),
                fs::read(dir.join("kiroku.json")).unwrap(),
            )
        })
        .collect();
    let count = journal_count(root);
    let cancel_request = Arc::new(Mutex::new(Some("catalog-read".to_string())));
    let requests = cancel_request.clone();
    let started = Arc::new(Mutex::new(Vec::<Value>::new()));
    let starts = started.clone();
    let owner = window.clone();
    let listener = app.listen("job:started", move |event| {
        let payload: Value = serde_json::from_str(event.payload()).unwrap();
        starts.lock().unwrap().push(payload.clone());
        if requests
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| payload["requestId"] == *r)
        {
            assert!(cancel(&owner, payload["jobId"].as_u64().unwrap()));
        }
    });
    let inventory = invoke(
        window,
        "catalog_inventory",
        json!({"requestId":"catalog-read"}),
    )
    .unwrap();
    assert_eq!(inventory["cancelled"], true);
    assert!(inventory["movies"].as_array().unwrap().is_empty());
    assert!(!state.job_running());

    *cancel_request.lock().unwrap() = Some("catalog-preview".into());
    let error = invoke(
        window,
        "catalog_preview",
        json!({"dirs":dirs,"requestId":"catalog-preview"}),
    )
    .unwrap_err();
    assert!(error.as_str().unwrap().contains("取消"));
    assert!(state.catalog.lock().unwrap().is_none());
    assert!(!state.job_running());

    *cancel_request.lock().unwrap() = None;
    let preview = invoke(window, "catalog_preview", json!({"dirs":dirs})).unwrap();
    assert_eq!(preview["items"].as_array().unwrap().len(), 2);
    *cancel_request.lock().unwrap() = Some("catalog-apply".into());
    let result = invoke(
        window,
        "catalog_apply",
        json!({"token":preview["token"],"requestId":"catalog-apply"}),
    )
    .unwrap();
    assert_eq!(result["cancelled"], true);
    assert!(result["items"].as_array().unwrap().is_empty());
    assert!(result["indexError"].is_null());
    assert_eq!(journal_count(root), count);
    assert_files(&dirs, &before);
    assert!(!state.job_running());
    for request in ["catalog-read", "catalog-preview", "catalog-apply"] {
        assert_eq!(
            started
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e["requestId"] == request)
                .count(),
            1
        );
    }
    println!("PASS catalog_early_cancel commands=3 writes=0 journals=0");

    *cancel_request.lock().unwrap() = None;
    let preview = invoke(window, "catalog_preview", json!({"dirs":dirs})).unwrap();
    let owner = window.clone();
    let progress = app.listen("library:catalog", move |event| {
        let payload: Value = serde_json::from_str(event.payload()).unwrap();
        if payload["phase"] == "apply" && payload["done"] == 1 {
            assert!(cancel(&owner, payload["jobId"].as_u64().unwrap()));
        }
    });
    let result = invoke(
        window,
        "catalog_apply",
        json!({"token":preview["token"],"requestId":"catalog-partial"}),
    )
    .unwrap();
    assert_eq!(result["cancelled"], true);
    assert!(result["indexError"].is_null());
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    let first = &result["items"][0];
    assert_eq!(first["code"], "SMPL-001");
    assert!(first["error"].is_null());
    let announced = started.lock().unwrap();
    assert_eq!(
        announced
            .iter()
            .find(|e| e["requestId"] == "catalog-partial")
            .unwrap()["jobId"],
        result["jobId"]
    );
    drop(announced);
    assert!(!state.job_running());
    let nfo = fs::read_to_string(dirs[0].join("movie.nfo")).unwrap();
    assert!(
        nfo.contains("<studio>Canonical studio</studio>")
            && nfo.contains("<playcount>7</playcount>")
    );
    let metadata: kiroku_core::Metadata =
        serde_json::from_slice(&fs::read(dirs[0].join("kiroku.json")).unwrap()).unwrap();
    assert_eq!(metadata.fields.studio.as_deref(), Some("Canonical studio"));
    assert_files(&dirs[1..], &before[1..]);
    assert_eq!(
        fs::read(dirs[0].join("sample.mp4")).unwrap(),
        b"synthetic video bytes"
    );
    assert_eq!(journal_count(root), count + 1);
    let journal = kiroku_core::journal::Journal::load(&root.join(".kiroku").join(format!(
        "journal-{}.json",
        first["journalId"].as_str().unwrap()
    )))
    .unwrap();
    assert_eq!(journal.status, kiroku_core::journal::JournalStatus::Applied);
    let detail = invoke(window, "library_get", json!({"code":"SMPL-001"})).unwrap();
    assert_eq!(detail["meta"]["studio"], "Canonical studio");
    let untouched = invoke(window, "library_get", json!({"code":"SMPL-002"})).unwrap();
    assert_eq!(untouched["meta"]["studio"], "Original studio");

    let undone = invoke(window, "rollback", json!({"journalId":first["journalId"]})).unwrap();
    assert_eq!(undone["status"], "rolledBack");
    assert!(undone["failures"].as_array().unwrap().is_empty() && undone["indexError"].is_null());
    assert_files(&dirs, &before);
    let detail = invoke(window, "library_get", json!({"code":"SMPL-001"})).unwrap();
    assert_eq!(detail["meta"]["studio"], "Original studio");
    assert!(!state.job_running());
    app.unlisten(listener);
    app.unlisten(progress);
    println!("PASS catalog_partial_rollback completed=1 restored_bytes=true index_restored=true");
}
