use super::*;
use crate::state::LibraryReadSnapshot;
use kiroku_core::supplement::{self, Entry, Kind};

pub struct Pending {
    token: String,
    preview: supplement::Preview,
    snapshot: LibraryReadSnapshot,
    settings: Settings,
}
#[derive(Serialize)]
pub struct Review {
    token: String,
    code: String,
    dir: PathBuf,
    entries: Vec<Entry>,
}

#[tauri::command]
pub async fn supplement_preview(
    app: AppHandle,
    code: String,
    item_id: Option<u32>,
    request_id: Option<String>,
) -> CmdResult<Review> {
    let state = app.state::<AppState>();
    let (id, cancel) =
        reliability::begin_reported_job(&state, "supplement-preview", request_id, |id, r| {
            emit(&app, "job:started", id, serde_json::json!({"requestId":r}))
        })?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _job = guard;
        let state = app.state::<AppState>();
        let snapshot = state.read_snapshot()?;
        let reader = snapshot.handle.open().map_err(|e| e.to_string())?;
        let detail = reader
            .get(&code)
            .map_err(|e| e.to_string())?
            .ok_or("媒体库中没有匹配编号的影片。请先整理入库，或核对影片编号。")?;
        let video = detail
            .files
            .iter()
            .find(|f| Path::new(&f.path).parent() == Some(Path::new(&detail.dir)))
            .ok_or("该影片的库内目录没有可匹配的原视频")?;
        let (source, local) = if let Some(item_id) = item_id {
            let (item, siblings, mut meta) = {
                let s = state.session();
                let item = s
                    .scan
                    .iter()
                    .find(|s| s.id == item_id && s.code.as_ref().is_some_and(|c| c.id == code))
                    .ok_or("导入列表已变化，请重新预览")?;
                (
                    item.clone(),
                    s.scan
                        .iter()
                        .filter(|s| s.path.parent() == item.path.parent())
                        .cloned()
                        .collect::<Vec<_>>(),
                    s.metas.get(&code).cloned(),
                )
            };
            // Same-code scan rows may come from distinct folders. Only the chosen row owns its local companions.
            if let Some(meta) = &mut meta {
                meta.import_local = None;
            }
            (
                meta,
                pipeline::LocalImport::beside(&item, &siblings, &cancel),
            )
        } else {
            (None, None)
        };
        let settings = state.settings();
        let preview = supplement::preview(
            snapshot.handle.root(),
            Path::new(&detail.dir),
            &code,
            Path::new(&video.path),
            source.as_ref(),
            local,
            &settings,
        )
        .map_err(|e| e.to_string())?;
        if cancel.is_cancelled() {
            return Err("资料预览已停止".into());
        }
        state.validate_read(&snapshot)?;
        let token = id.to_string();
        let review = Review {
            token: token.clone(),
            code,
            dir: preview.dir.clone(),
            entries: preview.entries.clone(),
        };
        *state.supplement.lock().unwrap_or_else(|e| e.into_inner()) = Some(Pending {
            token,
            preview,
            snapshot,
            settings,
        });
        Ok(review)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn supplement_apply(
    app: AppHandle,
    token: String,
    kinds: Vec<Kind>,
    request_id: Option<String>,
) -> CmdResult<supplement::Outcome> {
    let state = app.state::<AppState>();
    let (id, cancel) =
        reliability::begin_reported_job(&state, "supplement-apply", request_id, |id, r| {
            emit(&app, "job:started", id, serde_json::json!({"requestId":r}))
        })?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    tauri::async_runtime::spawn_blocking(move || {
        let _job=guard;
        let state=app.state::<AppState>();
        let p={
            let mut slot=state.supplement.lock().unwrap_or_else(|e|e.into_inner());
            if !slot.as_ref().is_some_and(|p|p.token==token){return Err("只补资料预览已过期，请重新检查".into())}
            slot.take().unwrap()
        };
        let _writer=state.settings_writer.lock().unwrap_or_else(|e|e.into_inner());
        state.validate_read(&p.snapshot)?;
        if state.settings()!=p.settings{return Err("设置已变化，请重新预览".into())}
        let dir=p.preview.dir.clone();
        let http=p.settings.build_http().map_err(|e|e.to_string())?.for_images();
        let mut out=supplement::apply(p.preview,&kinds,&p.settings,&cancel,&mut |url,avatar|{
            let mut http=http.clone();
            if let Ok(site)=p.settings.source_for_url(url){
                http=http.with_proxy(p.settings.proxy_for(site)).map_err(std::io::Error::other)?;
                http.seed_cookies(&site.base,&site.cookie);
            }
            tauri::async_runtime::block_on(async {
                tokio::select! {
                    _ = cancel.cancelled() => Err(std::io::Error::new(std::io::ErrorKind::Interrupted,"已停止")),
                    result = async {
                        if avatar {return kiroku_core::actor_image::fetch(&http,url).await}
                        let (status,data)=http.get_image(url).await?;
                        if !(200..300).contains(&status) {return Err(std::io::Error::other(format!("图片来源返回 HTTP {status}")))}
                        Ok(data)
                    }=>result
                }
            })
        }).map_err(|e|e.to_string())?;
        if out.journal_id.is_some(){if let Err(e)=reindex(&app,&[dir]){out.skipped.push(format!("索引更新失败：{e}"))}}
        Ok(out)
    }).await.map_err(|e|e.to_string())?
}
