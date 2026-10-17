use super::*;
use kiroku_core::append;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendEnvelope {
    token: String,
    #[serde(flatten)]
    preview: append::AppendPreview,
}
#[tauri::command]
pub async fn pick_media_files(app: AppHandle) -> CmdResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("选择追加的视频或字幕")
            .add_filter(
                "视频和字幕",
                &[
                    "mp4", "mkv", "avi", "wmv", "mov", "ts", "m2ts", "flv", "rmvb", "srt", "ass",
                    "ssa", "vtt", "sub", "idx", "sup",
                ],
            )
            .blocking_pick_files()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.into_path().ok().map(|p| p.display().to_string()))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn append_targets(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
) -> CmdResult<Vec<PathBuf>> {
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let (id, cancel) = state.begin_job("append-preview")?;
    let _guard = JobGuard { app, id };
    tauri::async_runtime::spawn_blocking(move || {
        append::targets_cancellable(&root, &code, &cancel).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn append_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
    inputs: Vec<PathBuf>,
    target: Option<PathBuf>,
    video: Option<PathBuf>,
) -> CmdResult<AppendEnvelope> {
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let settings = state.settings();
    let (id, cancel) = state.begin_job("append-preview")?;
    let _guard = JobGuard {
        app: app.clone(),
        id,
    };
    *state.append.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let p = tauri::async_runtime::spawn_blocking(move || {
        append::preview_with_video_cancellable(
            &root,
            &code,
            &inputs,
            target.as_deref(),
            video.as_deref(),
            &cancel,
        )
        .and_then(|mut p|{p.set_mode(settings.organize_mode)?;Ok(p)})
        .map_err(|e| e.to_string())
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
    *state.append.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((token.clone(), p.clone(), settings));
    Ok(AppendEnvelope { token, preview: p })
}
#[tauri::command]
pub async fn append_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
) -> CmdResult<append::AppendResult> {
    let (id, cancel) = state.begin_job("append")?;
    let _guard = JobGuard {
        app: app.clone(),
        id,
    };
    let candidate = state
        .append
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or("请先生成追加预览")?;
    if candidate.0 != token {
        return Err("追加预览已过期".into());
    }
    let app2 = app.clone();
    let settings = candidate.2.clone();
    let dir = candidate.1.target_dir.clone();
    let root = candidate.1.root.to_string_lossy().into_owned();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        let _writer = state
            .settings_writer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.settings() != candidate.2 {
            return Err("设置已变化，请重新预览".into());
        }
        append::apply(&candidate.1, &cancel).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    emit(&app, "library:changed", id, serde_json::json!({}));
    notify_plex(&app, &settings.plex, id, root, vec![dir]).await;
    Ok(result)
}

#[tauri::command]
pub async fn sources_check(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Option<Vec<String>>,
    draft: Option<kiroku_core::settings::Settings>,
) -> CmdResult<Vec<kiroku_core::connectivity::SourceCheck>> {
    let settings = draft.unwrap_or_else(|| state.settings());
    // The probe only reads sources/network settings. Unfinished templates or
    // scan rules in other draft categories must not block authentication.
    let (id, cancel) = state.begin_job("sources-check")?;
    let _guard = JobGuard { app, id };
    Ok(kiroku_core::connectivity::check(&settings, ids.as_deref(), &cancel).await)
}
#[tauri::command]
pub async fn library_health(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<kiroku_core::health::HealthReport> {
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let settings = state.settings();
    let (id, cancel) = state.begin_job("library-health")?;
    let _guard = JobGuard {
        app: app.clone(),
        id,
    };
    let ev = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        kiroku_core::health::scan(&root, &cancel, &|done| {
            emit(
                &ev,
                "library:health-progress",
                id,
                serde_json::json!({"done":done}),
            )
        })
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    if state.settings() != settings {
        return Err("媒体库或设置已变化，请重新检查".into());
    }
    Ok(r)
}
#[tauri::command]
pub async fn reveal_library_path(
    app: AppHandle,
    state: State<'_, AppState>,
    path: PathBuf,
) -> CmdResult<()> {
    let root = state.library_root().ok_or("请先设置媒体库")?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = if std::fs::symlink_metadata(&path).is_ok_and(|m|m.file_type().is_symlink()) {
            append::confined_path(&root,path.parent().ok_or("路径无父目录")?)
        }else{append::confined_path(&root, &path)}.map_err(|e| e.to_string())?;
        let target = if path.exists() {
            path
        } else {
            path.parent().ok_or("路径无效")?.to_path_buf()
        };
        app.opener()
            .reveal_item_in_dir(target)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceEnvelope {
    token: String,
    #[serde(flatten)]
    preview: kiroku_core::maintenance::MaintenancePreview,
}
#[tauri::command]
pub async fn maintenance_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    include_recent: Option<bool>,
) -> CmdResult<MaintenanceEnvelope> {
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let settings = state.settings();
    let policy = if include_recent.unwrap_or(false) {
        kiroku_core::maintenance::RetentionPolicy { history_days: 0, log_days: 0 }
    } else { settings.retention.clone() };
    let (id, cancel) = state.begin_job("maintenance-preview")?;
    let _guard = JobGuard { app, id };
    let p = tauri::async_runtime::spawn_blocking(move || {
        kiroku_core::maintenance::preview_cancellable(
            &root,
            &kiroku_core::logging::log_dir(&Settings::default_path()),
            &policy,
            &cancel,
        )
        .map_err(|e| e.to_string())
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
    *state.maintenance.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((token.clone(), p.clone(), settings));
    Ok(MaintenanceEnvelope { token, preview: p })
}
#[tauri::command]
pub async fn maintenance_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    selected: Vec<String>,
) -> CmdResult<kiroku_core::maintenance::MaintenanceResult> {
    let (id, cancel) = state.begin_job("maintenance")?;
    let _guard = JobGuard {
        app: app.clone(),
        id,
    };
    let candidate = state
        .maintenance
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .ok_or("请先预览清理")?;
    if candidate.0 != token {
        return Err("预览已过期".into());
    }
    let app2 = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        let _writer = state
            .settings_writer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.settings() != candidate.2 {
            return Err("设置已变化，请重新预览".into());
        }
        kiroku_core::maintenance::apply(
            &candidate.1,
            &selected,
            &pipeline::trash_to_system,
            &cancel,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    emit(&app, "library:changed", id, serde_json::json!({}));
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn library_switch_invalidates_both_preview_tokens() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("ABC-123");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("kiroku.json"),
            serde_json::to_vec(&kiroku_core::Metadata {
                code: "ABC-123".into(),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::write(dir.join("ABC-123.mp4"), b"video").unwrap();
        let settings = Settings {
            library_root: Some(t.path().to_string_lossy().into()),
            ..Default::default()
        };
        let state = AppState::new(settings.clone());
        let p = append::preview(t.path(), "ABC-123", &[], None).unwrap();
        *state.append.lock().unwrap() = Some(("1".into(), p, settings.clone()));
        let p = kiroku_core::maintenance::preview(
            t.path(),
            &t.path().join("logs"),
            &Default::default(),
        )
        .unwrap();
        *state.maintenance.lock().unwrap() = Some(("2".into(), p, settings));
        assert!(library_change(&state, Some("/other-library")).unwrap());
        assert!(state.append.lock().unwrap().is_none());
        assert!(state.maintenance.lock().unwrap().is_none());
    }
}

#[tauri::command]
pub async fn append_target_videos(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
    target: PathBuf,
) -> CmdResult<Vec<PathBuf>> {
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let (id, cancel) = state.begin_job("append-videos")?;
    let _guard = JobGuard { app, id };
    tauri::async_runtime::spawn_blocking(move || {
        append::target_videos_cancellable(&root, &code, &target, &cancel).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
#[cfg(test)]
mod cancel_scope_tests {
    use super::*;
    #[test]
    fn stale_task_id_cannot_cancel_the_next_task() {
        let state = AppState::new(Default::default());
        let (id, token) = state.begin_job("repair").unwrap();
        assert!(!state.cancel_job_for(Some(id + 1)));
        assert!(!token.is_cancelled());
        assert!(state.cancel_job_for(Some(id)));
        assert!(token.is_cancelled());
    }
}
