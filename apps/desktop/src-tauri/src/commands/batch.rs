use super::*;

#[derive(Serialize)]
pub struct BatchEditItem {
    code: String,
    changes: Vec<kiroku_core::revision::Change>,
    warnings: Vec<String>,
}
#[derive(Serialize)]
pub struct BatchEditPreview {
    token: String,
    root: String,
    items: Vec<BatchEditItem>,
}
impl BatchEditPreview {
    fn new(token: String, root: String, previews: &[kiroku_core::revision::Preview]) -> Self {
        Self { token, root, items: previews.iter().map(|p| BatchEditItem {
            code: p.code.clone(), changes: p.changes.clone(), warnings: p.warnings.clone(),
        }).collect() }
    }
}

fn take_pending(state: &AppState, token: &str) -> Option<crate::state::PendingLanguageExport> {
    let mut slot = state.batch_edit.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|p| p.token == token) { slot.take() } else { None }
}

/// A late response from a closed editor can release only its own preview.
#[tauri::command]
pub async fn batch_edit_discard(app: AppHandle, token: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || drop(take_pending(&app.state::<AppState>(), &token)))
        .await.map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEditResult {
    job_id: u64,
    results: Vec<RevisionResult>,
    failed: Vec<String>,
    skipped: Vec<String>,
    cancelled: bool,
}
#[tauri::command]
pub async fn batch_edit_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    codes: Vec<String>,
    edit: kiroku_core::batch_edit::BatchEdit,
    request_id: Option<String>,
) -> CmdResult<BatchEditPreview> {
    let (id, cancel) = reliability::begin_reported_job(
        &state,
        "batch-edit-preview",
        request_id,
        |id, request_id| {
            emit(
                &app,
                "job:started",
                id,
                serde_json::json!({"requestId":request_id}),
            );
        },
    )?;
    let job_guard = JobGuard {
        app: app.clone(),
        id,
    };
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let settings = state.settings();
    let result = tauri::async_runtime::spawn_blocking(move || -> CmdResult<_> {
        let state = app.state::<AppState>();
        // Release the previous batch before allocating another batch's metadata.
        state.batch_edit.lock().unwrap_or_else(|e| e.into_inner()).take();
        let guard =
            kiroku_core::pipeline::RootGuard::capture(&root, false).map_err(|e| e.to_string())?;
        let mut previews = vec![];
        let mut seen = std::collections::BTreeSet::new();
        if codes.is_empty() || codes.len() > 200 {
            return Err("请选择 1–200 部影片".into());
        }
        for code in codes {
            if cancel.is_cancelled() {
                return Err("已取消".into());
            }
            if !seen.insert(code.clone()) {
                continue;
            }
            let detail = with_lib(&state, |lib| lib.get(&code))?.ok_or("影片不在媒体库中")?;
            let captured = kiroku_core::revision::capture(&root, Path::new(&detail.dir), &code)
                .map_err(|e| e.to_string())?;
            let after = edit.apply_to(&captured.before).map_err(|e| e.to_string())?;
            previews.push(captured.prepare_after(after).map_err(|e| e.to_string())?);
        }
        let _writer = state
            .settings_writer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state.settings() != settings || state.library_root() != Some(root.clone()) {
            return Err("媒体库或设置已变化，请重新预览".into());
        }
        guard.verify().map_err(|e| e.to_string())?;
        if cancel.is_cancelled() {
            return Err("已取消".into());
        }
        let token = id.to_string();
        let response = BatchEditPreview::new(token.clone(), root.to_string_lossy().into_owned(), &previews);
        *state.batch_edit.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(crate::state::PendingLanguageExport {
                token: token.clone(),
                root: root.clone(),
                settings,
                previews,
                guard,
            });
        Ok(response)
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(job_guard);
    result
}
#[tauri::command]
pub async fn batch_edit_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    root: String,
    request_id: Option<String>,
) -> CmdResult<BatchEditResult> {
    let (id, cancel) = reliability::begin_reported_job(
        &state,
        "batch-edit-apply",
        request_id,
        |id, request_id| {
            emit(
                &app,
                "job:started",
                id,
                serde_json::json!({"requestId":request_id}),
            );
        },
    )?;
    let job_guard = JobGuard {
        app: app.clone(),
        id,
    };
    let result=tauri::async_runtime::spawn_blocking(move||->CmdResult<_>{
  let state=app.state::<AppState>();let _writer=state.settings_writer.lock().unwrap_or_else(|e|e.into_inner());
  let pending=take_pending(&state,&token).ok_or("预览已过期，请重新预览")?;
  if pending.settings!=state.settings()||Path::new(&root)!=pending.root||state.library_root()!=Some(pending.root.clone()){return Err("媒体库或设置已变化，请重新预览".into());}
  pending.guard.verify().map_err(|e|e.to_string())?;
  let mut results=vec![];let mut failed=vec![];let mut skipped=vec![];
  for preview in pending.previews {
   if cancel.is_cancelled(){break;}
   if preview.before==preview.after {skipped.push(preview.code);continue;}
   if let Err(error)=pending.guard.verify(){failed.push(error.to_string());break;}
   let detail=match with_lib(&state,|lib|lib.get(&preview.code)){Ok(Some(detail))=>detail,_=>{failed.push(format!("{}：影片已不在媒体库中",preview.code));continue;}};
   if Path::new(&detail.dir).canonicalize().ok().as_ref()!=Some(&preview.dir){failed.push(format!("{}：影片目录已变化，请重新预览",preview.code));continue;}
   match kiroku_core::revision::apply(&preview){Ok(out)=>{let index_error=reindex(&app,&[preview.dir]).err();results.push(RevisionResult{job_id:id,root:root.clone(),code:preview.code,journal_id:out.journal.id,error:out.error,index_error});},Err(error)=>failed.push(format!("{}：{error}",preview.code))}
   emit(&app,"batch:edit",id,serde_json::json!({"completed":results.iter().filter(|r|r.error.is_none()).count(),"failed":failed.len()+results.iter().filter(|r|r.error.is_some()).count()}));
  }
  Ok(BatchEditResult{skipped,job_id:id,results,failed,cancelled:cancel.is_cancelled()})
 }).await.map_err(|e|e.to_string())?;
    drop(job_guard);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiroku_core::{batch_edit::BatchEdit, revision, Metadata};

    fn previews(root: &Path, count: usize, bytes: usize) -> Vec<revision::Preview> {
        (0..count).map(|i| {
            let code = format!("SMPL-{i:06}");
            let dir = root.join(&code);
            std::fs::create_dir(&dir).unwrap();
            let mut meta = Metadata { code: code.clone(), ..Default::default() };
            meta.fields.plot = Some("x".repeat(bytes));
            std::fs::write(dir.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
            let before = revision::capture(root, &dir, &code).unwrap();
            before.prepare_after(BatchEdit::Favorite { favorite: true }.apply_to(&before.before).unwrap()).unwrap()
        }).collect()
    }

    #[test]
    fn batch_response_omits_full_metadata_and_discard_only_releases_matching_token() {
        let root = tempfile::tempdir().unwrap();
        let previews = previews(root.path(), 2, 256 * 1024);
        let response = BatchEditPreview::new("new".into(), root.path().display().to_string(), &previews);
        let json = serde_json::to_value(&response).unwrap();
        assert!(serde_json::to_vec(&response).unwrap().len() < 1024);
        assert_eq!(json["items"][0]["changes"][0]["field"], "favorite");
        assert!(json["items"][0].get("before").is_none());
        assert!(json["items"][0].get("after").is_none());
        let state = AppState::new(Settings::default());
        *state.batch_edit.lock().unwrap() = Some(crate::state::PendingLanguageExport {
            token: "new".into(), root: root.path().into(), settings: state.settings(), previews,
            guard: pipeline::RootGuard::capture(root.path(), false).unwrap(),
        });
        assert!(take_pending(&state, "old").is_none());
        assert!(state.batch_edit.lock().unwrap().is_some());
        drop(take_pending(&state, "new"));
        assert!(state.batch_edit.lock().unwrap().is_none());
        assert!(take_pending(&state, "new").is_none());
    }

    #[test]
    #[ignore = "isolated synthetic maximum-batch response measurement"]
    fn batch_response_size_measurement() {
        let root = tempfile::tempdir().unwrap();
        let descriptors = || std::fs::read_dir("/dev/fd").ok().map(|entries| entries.count());
        let before_fds = descriptors();
        let previews = previews(root.path(), 200, 64 * 1024);
        // Run each mode in its own process when measuring peak RSS.
        let mode = std::env::var("KIROKU_RESPONSE_MODE").unwrap_or_else(|_| "compact".into());
        let bytes = if mode == "legacy" {
            let old = LanguageBatchPreview {
                token: "batch".into(), root: root.path().display().to_string(),
                items: previews.iter().map(|p| RevisionPreview { token:"batch".into(), preview:p.clone() }).collect(),
            };
            serde_json::to_vec(&old).unwrap().len()
        } else {
            assert_eq!(mode, "compact");
            let compact = BatchEditPreview::new("batch".into(), root.path().display().to_string(), &previews);
            let bytes = serde_json::to_vec(&compact).unwrap().len();
            assert!(bytes < 32 * 1024);
            bytes
        };
        let state = AppState::new(Settings::default());
        *state.batch_edit.lock().unwrap() = Some(crate::state::PendingLanguageExport {
            token: "batch".into(), root: root.path().into(), settings: state.settings(), previews,
            guard: pipeline::RootGuard::capture(root.path(), false).unwrap(),
        });
        let retained_fds = descriptors();
        drop(take_pending(&state, "batch"));
        let after_fds = descriptors();
        println!("batch=200 plot_bytes=65536 mode={mode} response_bytes={bytes} fds_before={before_fds:?} fds_retained={retained_fds:?} fds_after_discard={after_fds:?}");
        assert_eq!(before_fds, after_fds);
    }
}
