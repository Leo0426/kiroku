use crate::state::{emit, AppState, JobGuard, LibraryReadSnapshot};
use kiroku_core::{
    actor_hosting::Config,
    portability::{self, ExportPlan, ExportPreview, FileKind},
};
use serde::Serialize;
use std::{
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

pub struct PendingExport {
    token: String,
    snapshot: LibraryReadSnapshot,
    avatars: Config,
    plan: ExportPlan,
}

#[derive(Serialize)]
pub struct PreviewResponse {
    token: String,
    #[serde(flatten)]
    preview: ExportPreview,
}

fn begin(
    app: &AppHandle,
    kind: &'static str,
    request_id: Option<String>,
) -> Result<(JobGuard, CancellationToken), String> {
    let state = app.state::<AppState>();
    let (id, cancel) = state.begin_job(kind)?;
    if let Some(request_id) = request_id {
        emit(
            app,
            "job:started",
            id,
            serde_json::json!({"requestId":request_id}),
        );
    }
    Ok((
        JobGuard {
            app: app.clone(),
            id,
        },
        cancel,
    ))
}

#[tauri::command]
pub async fn package_preview(
    app: AppHandle,
    codes: Vec<String>,
    kinds: Vec<FileKind>,
    directory: PathBuf,
    request_id: Option<String>,
) -> Result<PreviewResponse, String> {
    let (guard, cancel) = begin(&app, "package-preview", request_id)?;
    let state = app.state::<AppState>();
    let snapshot = state.read_snapshot()?;
    let avatars = state.settings().actor_hosting;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let token = format!("{stamp}-{}", guard.id);
    let name = format!("kiroku-{token}");
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        let state = app.state::<AppState>();
        state
            .package_export
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        state.validate_read(&snapshot)?;
        let reader = snapshot.handle.open().map_err(|e| e.to_string())?;
        let plan = portability::preview(
            &reader, &codes, &kinds, &directory, &name, &avatars, &cancel,
        )
        .map_err(|e| e.to_string())?;
        state.validate_read(&snapshot)?;
        let preview = plan.preview.clone();
        *state
            .package_export
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(PendingExport {
            token: token.clone(),
            snapshot,
            avatars,
            plan,
        });
        Ok(PreviewResponse { token, preview })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn package_export(
    app: AppHandle,
    token: String,
    request_id: Option<String>,
) -> Result<ExportPreview, String> {
    let (guard, cancel) = begin(&app, "package-export", request_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let pending = {
            let mut slot = state
                .package_export
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if slot.as_ref().is_none_or(|p| p.token != token) {
                return Err("资料包预览已失效，请重新预览".into());
            }
            slot.take().expect("matched preview")
        };
        state.validate_read(&pending.snapshot)?;
        if state.settings().actor_hosting != pending.avatars {
            return Err("头像设置已变化，请重新预览".into());
        }
        let last = std::cell::Cell::new(Instant::now());
        let result = portability::export(pending.plan, &cancel, |done, total| {
            if done == total || last.get().elapsed().as_millis() >= 150 {
                emit(
                    &app,
                    "package:progress",
                    guard.id,
                    serde_json::json!({"done":done,"total":total}),
                );
                last.set(Instant::now());
            }
        })
        .map_err(|e| e.to_string());
        drop(guard);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}
