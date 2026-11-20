use super::*;
use kiroku_core::package_restore::{self, Package, Rules};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub struct Pending {
    token: String,
    directory: PathBuf,
    manifest: String,
    snapshot: crate::state::LibraryReadSnapshot,
    guard: kiroku_core::pipeline::RootGuard,
    settings: Settings,
    rules: Rules,
    approved: BTreeMap<String, (String, kiroku_core::pipeline::RootGuard)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    code: String,
    status: &'static str,
    destination: Option<String>,
    changes: Vec<kiroku_core::revision::Change>,
    files: Vec<String>,
    warnings: Vec<String>,
}
#[derive(Serialize)]
pub struct Review {
    token: String,
    root: String,
    items: Vec<Row>,
}

#[tauri::command]
pub async fn package_restore_preview(
    app: AppHandle,
    directory: String,
    rules: Rules,
    request_id: Option<String>,
) -> CmdResult<Review> {
    let state = app.state::<AppState>();
    let (id, cancel) =
        reliability::begin_reported_job(&state, "package-restore-preview", request_id, |id, r| {
            emit(&app, "job:started", id, serde_json::json!({"requestId":r}))
        })?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<Review> {
        let _job = guard;
        let state = app.state::<AppState>();
        state
            .package_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let snapshot = state.read_snapshot()?;
        let root = snapshot.handle.root().to_path_buf();
        let root_guard =
            kiroku_core::pipeline::RootGuard::capture(&root, false).map_err(|e| e.to_string())?;
        let reader = snapshot.handle.open().map_err(|e| e.to_string())?;
        let settings = state.settings();
        let directory = PathBuf::from(directory)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let package = Package::open(&directory).map_err(|e| e.to_string())?;
        let mut items = Vec::new();
        let mut approved = BTreeMap::new();
        let mut last = std::time::Instant::now();
        for movie in &package.manifest.movies {
            if cancel.is_cancelled() {
                return Err("导回预览已停止".into());
            }
            let mut row = Row {
                code: movie.code.clone(),
                status: "unmatched",
                destination: None,
                changes: vec![],
                files: vec![],
                warnings: vec![],
            };
            let mut inspect = || -> CmdResult<()> {
                let Some(detail) = reader.get(&movie.code).map_err(|e| e.to_string())? else {
                    return Ok(());
                };
                let dir = Path::new(&detail.dir);
                row.destination = Some(detail.dir.clone());
                let candidate = package_restore::prepare(
                    &package, movie, &root, dir, &rules, &settings, &cancel,
                )
                .map_err(|e| e.to_string())?;
                if let Some(signature) = candidate.signature() {
                    let guard = kiroku_core::pipeline::RootGuard::capture(dir, false)
                        .map_err(|e| e.to_string())?;
                    approved.insert(movie.code.clone(), (signature, guard));
                    row.status = "ready";
                } else {
                    row.status = "unchanged";
                }
                row.changes = candidate.preview.map(|p| p.changes).unwrap_or_default();
                row.files = candidate.files;
                row.warnings = candidate.warnings;
                Ok(())
            };
            if let Err(message) = inspect() {
                row.status = "error";
                row.warnings.push(message);
            }
            items.push(row);
            progress(
                &app,
                id,
                items.len(),
                package.manifest.movies.len(),
                &mut last,
            );
        }
        state.validate_read(&snapshot)?;
        if cancel.is_cancelled() || state.settings() != settings {
            return Err("任务已停止或设置已变化，请重新预览".into());
        }
        let token = id.to_string();
        *state
            .package_restore
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(Pending {
            token: token.clone(),
            directory,
            manifest: package.signature,
            snapshot,
            guard: root_guard,
            settings,
            rules,
            approved,
        });
        Ok(Review {
            token,
            root: root.display().to_string(),
            items,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn package_restore_apply(
    app: AppHandle,
    token: String,
    codes: Vec<String>,
    request_id: Option<String>,
) -> CmdResult<LanguageBatchResult> {
    let state = app.state::<AppState>();
    let (id, cancel) =
        reliability::begin_reported_job(&state, "package-restore-apply", request_id, |id, r| {
            emit(&app, "job:started", id, serde_json::json!({"requestId":r}))
        })?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<LanguageBatchResult> {
        let _job = guard;
        let state = app.state::<AppState>();
        let _writer = state
            .settings_writer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let p = {
            let mut slot = state
                .package_restore
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !slot.as_ref().is_some_and(|p| p.token == token) {
                return Err("导回预览已过期".into());
            }
            slot.take().unwrap()
        };
        state.validate_read(&p.snapshot)?;
        if state.settings() != p.settings {
            return Err("设置已变化，请重新预览".into());
        }
        p.guard.verify().map_err(|e| e.to_string())?;
        let selected: BTreeSet<_> = codes.into_iter().collect();
        if selected.is_empty() || selected.iter().any(|code| !p.approved.contains_key(code)) {
            return Err("只能导回已预览且匹配的影片".into());
        }
        let package = Package::open(&p.directory).map_err(|e| e.to_string())?;
        if package.signature != p.manifest {
            return Err("资料包清单已变化，请重新预览".into());
        }
        let root = p.snapshot.handle.root();
        let reader = p.snapshot.handle.open().map_err(|e| e.to_string())?;
        let mut results = vec![];
        let mut failed = vec![];
        let mut last = std::time::Instant::now();
        for movie in package
            .manifest
            .movies
            .iter()
            .filter(|m| selected.contains(&m.code))
        {
            if cancel.is_cancelled() {
                break;
            }
            let save = || -> CmdResult<RevisionResult> {
                state.validate_read(&p.snapshot)?;
                p.guard.verify().map_err(|e| e.to_string())?;
                let (signature, dir_guard) = &p.approved[&movie.code];
                dir_guard.verify().map_err(|e| e.to_string())?;
                let detail = reader
                    .get(&movie.code)
                    .map_err(|e| e.to_string())?
                    .ok_or("目标影片已移走")?;
                let next = package_restore::prepare(
                    &package,
                    movie,
                    root,
                    Path::new(&detail.dir),
                    &p.rules,
                    &p.settings,
                    &cancel,
                )
                .map_err(|e| e.to_string())?;
                if next.signature().as_ref() != Some(signature) {
                    return Err("目标或资料已变化，请重新预览".into());
                }
                let preview = next.preview.ok_or("当前没有修改")?;
                if cancel.is_cancelled() {
                    return Err("导回已停止".into());
                }
                let out = kiroku_core::revision::apply(&preview).map_err(|e| e.to_string())?;
                Ok(RevisionResult {
                    job_id: id,
                    root: root.display().to_string(),
                    code: movie.code.clone(),
                    journal_id: out.journal.id,
                    error: out.error,
                    index_error: reindex(&app, &[preview.dir]).err(),
                })
            };
            match save() {
                Ok(result) => results.push(result),
                Err(e) => failed.push(format!("{}：{}", movie.code, e)),
            }
            progress(
                &app,
                id,
                results.len() + failed.len(),
                selected.len(),
                &mut last,
            );
        }
        Ok(LanguageBatchResult {
            job_id: id,
            results,
            failed,
            cancelled: cancel.is_cancelled(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

fn progress(app: &AppHandle, id: u64, done: usize, total: usize, last: &mut std::time::Instant) {
    if done == 1 || done == total || last.elapsed().as_millis() >= 150 {
        emit(
            app,
            "package:progress",
            id,
            serde_json::json!({"done":done,"total":total}),
        );
        *last = std::time::Instant::now();
    }
}
