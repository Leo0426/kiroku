//! Fixture-only transaction gate for the WebKit rebuild checks.
use super::*;
use std::sync::{atomic::AtomicBool, mpsc};
use tauri::Listener;

#[derive(Default)]
pub(super) struct RebuildProbe {
    files: Mutex<Option<BTreeMap<PathBuf, Vec<u8>>>>,
    release: Mutex<Option<mpsc::Sender<()>>>,
    listener: Mutex<Option<tauri::EventId>>,
    timed_out: AtomicBool,
}

impl RebuildProbe {
    pub(super) fn release(&self) {
        if let Some(sender) = self.release.lock().unwrap().take() {
            let _ = sender.send(());
        }
    }

    pub(super) fn close_gate(&self, app: &AppHandle) {
        self.release();
        if let Some(listener) = self.listener.lock().unwrap().take() {
            app.unlisten(listener);
        }
    }
}

#[tauri::command]
pub(super) async fn bridge_rebuild_prepare(
    app: AppHandle,
    probe: State<'_, Arc<Probe>>,
) -> Result<(), String> {
    let probe = probe.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if app.state::<AppState>().job_running() {
            return Err("cannot prepare rebuild fixture while busy".into());
        }
        if probe.rebuild.files.lock().unwrap().is_some() {
            return Err("rebuild fixture already prepared".into());
        }
        verify_files(&probe, 3, false)?;
        let changed = probe.root.join("SMPL-001");
        let mut metadata: kiroku_core::Metadata = serde_json::from_slice(
            &fs::read(changed.join("kiroku.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        metadata.fields.title = Some("Rebuilt sample".into());
        fs::write(
            changed.join("kiroku.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        fs::write(
            changed.join("movie.nfo"),
            kiroku_core::nfo::to_nfo(&metadata),
        )
        .map_err(|e| e.to_string())?;
        let added = probe.root.join("IDX-003");
        fs::create_dir(&added).map_err(|e| e.to_string())?;
        metadata.code = "IDX-003".into();
        metadata.fields.title = Some("New index sample".into());
        fs::write(
            added.join("kiroku.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        fs::write(added.join("movie.nfo"), kiroku_core::nfo::to_nfo(&metadata))
            .map_err(|e| e.to_string())?;
        fs::write(added.join("sample.mp4"), b"synthetic rebuild video")
            .map_err(|e| e.to_string())?;
        let mut files = BTreeMap::new();
        for code in ["SMPL-001", "SMPL-002", "IDX-003"] {
            for name in ["movie.nfo", "kiroku.json", "sample.mp4"] {
                let path = probe.root.join(code).join(name);
                files.insert(path.clone(), fs::read(path).map_err(|e| e.to_string())?);
            }
        }
        for entry in fs::read_dir(probe.root.join(".kiroku")).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name().to_string_lossy().starts_with("journal-") {
                let path = entry.path();
                files.insert(path.clone(), fs::read(path).map_err(|e| e.to_string())?);
            }
        }
        *probe.rebuild.files.lock().unwrap() = Some(files);

        let (release, receiver) = mpsc::channel();
        *probe.rebuild.release.lock().unwrap() = Some(release);
        let receiver = Mutex::new(Some(receiver));
        let held_app = app.clone();
        let held_probe = probe.clone();
        let listener = app.listen("library:progress", move |event| {
            let payload: Value = serde_json::from_str(event.payload()).unwrap();
            if payload["root"] != held_probe.root.to_string_lossy().as_ref() || payload["done"] != 1
            {
                return;
            }
            let Some(receiver) = receiver.lock().unwrap().take() else {
                return;
            };
            let id = payload["jobId"].as_u64().unwrap();
            // Production rebuild has deleted/reindexed within its transaction.
            // Hold its worker here so WebKit can send a real scoped cancellation.
            state::emit(&held_app, "bridge:rebuild_waiting", id, payload);
            if receiver
                .recv_timeout(std::time::Duration::from_secs(15))
                .is_err()
            {
                held_probe.rebuild.timed_out.store(true, Ordering::SeqCst);
            }
        });
        *probe.rebuild.listener.lock().unwrap() = Some(listener);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) fn bridge_rebuild_release(probe: State<'_, Arc<Probe>>) {
    probe.rebuild.release();
}

fn verify_prepared(probe: &Probe) -> Result<(), String> {
    if probe.rebuild.timed_out.load(Ordering::SeqCst) {
        return Err("rebuild gate timed out instead of being released".into());
    }
    let files = probe.rebuild.files.lock().unwrap();
    for (path, expected) in files.as_ref().ok_or("missing rebuild fixture")? {
        if fs::read(path).map_err(|e| e.to_string())? != *expected {
            return Err(format!(
                "rebuild changed media, metadata or journal: {}",
                path.display()
            ));
        }
    }
    let journals = fs::read_dir(probe.root.join(".kiroku"))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("journal-"))
        .count();
    if journals != 3 {
        return Err("rebuild changed journal count".into());
    }
    Ok(())
}

#[tauri::command]
pub(super) async fn bridge_rebuild_verify(
    app: AppHandle,
    probe: State<'_, Arc<Probe>>,
) -> Result<(), String> {
    let probe = probe.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if app.state::<AppState>().job_running() {
            return Err("rebuild did not release its slot".into());
        }
        verify_prepared(&probe)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(super) async fn bridge_rebuild_restore(
    app: AppHandle,
    probe: State<'_, Arc<Probe>>,
) -> Result<(), String> {
    let probe = probe.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        if app.state::<AppState>().job_running() {
            return Err("cannot restore rebuild fixture while busy".into());
        }
        verify_prepared(&probe)?;
        probe.rebuild.close_gate(&app);
        for (path, bytes) in &probe.originals {
            fs::write(path, bytes).map_err(|e| e.to_string())?;
        }
        // This directory was created exclusively by bridge_rebuild_prepare.
        fs::remove_dir_all(probe.root.join("IDX-003")).map_err(|e| e.to_string())?;
        verify_files(&probe, 3, false)?;
        // Keep the original journal snapshots while switching media expectations
        // to the verified restored bytes for the final production rebuild.
        let mut files = probe.rebuild.files.lock().unwrap();
        let expected = files.as_mut().ok_or("missing rebuild fixture")?;
        expected.retain(|path, _| path.parent() == Some(probe.root.join(".kiroku").as_path()));
        expected.extend(probe.originals.clone());
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
