use super::*;
use kiroku_core::catalog;
#[derive(Serialize)]
pub struct CatalogEnvelope {
    token: String,
    #[serde(flatten)]
    preview: catalog::Preview,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResult {
    job_id: u64,
    #[serde(flatten)]
    outcome: catalog::Outcome,
    index_error: Option<String>,
}
#[tauri::command]
pub async fn catalog_inventory(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<catalog::Inventory> {
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let (id, cancel) = state.begin_job("catalog-inventory")?;
    let _guard = JobGuard {
        app: app.clone(),
        id,
    };
    emit(
        &app,
        "library:catalog",
        id,
        serde_json::json!({"phase":"inventory","done":0,"total":0,"code":""}),
    );
    tauri::async_runtime::spawn_blocking(move || {
        catalog::inventory(&root, &cancel).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn catalog_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    dirs: Vec<PathBuf>,
) -> CmdResult<CatalogEnvelope> {
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let settings = state.settings();
    let (id, cancel) = state.begin_job("catalog-preview")?;
    let _guard = JobGuard {
        app: app.clone(),
        id,
    };
    *state.catalog.lock().unwrap_or_else(|e| e.into_inner()) = None;
    emit(
        &app,
        "library:catalog",
        id,
        serde_json::json!({"phase":"preview","done":0,"total":dirs.len(),"code":""}),
    );
    let rules = settings.name_rules.clone();
    let preview = tauri::async_runtime::spawn_blocking(move || {
        catalog::preview(&root, &dirs, &rules, &cancel).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    let _writer = state
        .settings_writer
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if state.settings() != settings {
        return Err("设置已变化，请重新预览".into());
    }
    let token = id.to_string();
    *state.catalog.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((token.clone(), preview.clone(), settings));
    Ok(CatalogEnvelope { token, preview })
}
#[tauri::command]
pub async fn catalog_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
) -> CmdResult<CatalogResult> {
    let (id, cancel) = state.begin_job("catalog-apply")?;
    let _guard = JobGuard {
        app: app.clone(),
        id,
    };
    emit(
        &app,
        "library:catalog",
        id,
        serde_json::json!({"phase":"apply","done":0,"total":0,"code":""}),
    );
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let _writer = state
            .settings_writer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut pending = state.catalog.lock().unwrap_or_else(|e| e.into_inner());
        let p = pending.as_ref().ok_or("名称预览已过期")?;
        if p.0 != token {
            return Err("名称预览已过期".into());
        }
        let (_, preview, settings) = pending.take().unwrap();
        drop(pending);
        if settings != state.settings() {
            return Err("设置已变化，请重新预览".into());
        }
        let outcome = catalog::apply(&preview, &settings.name_rules, &cancel, &|p| {
            emit(
                &app,
                "library:catalog",
                id,
                serde_json::json!({"phase":"apply","done":p.done,"total":p.total,"code":p.code}),
            )
        })
        .map_err(|e| e.to_string())?;
        let dirs = outcome
            .items
            .iter()
            .filter(|i| i.journal_id.is_some())
            .map(|i| i.dir.clone())
            .collect::<Vec<_>>();
        let index_error = if dirs.is_empty() {
            None
        } else {
            reindex(&app, &dirs).err()
        };
        Ok(CatalogResult {
            job_id: id,
            outcome,
            index_error,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
