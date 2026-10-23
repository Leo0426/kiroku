use crate::state::{allow_library_assets, emit, AppState, JobGuard, PendingRevision};
use kiroku_core::{
    journal::{Journal, JournalStatus},
    library::{Facets, Library, MovieDetail, Page, Query},
    organize::Op,
    pipeline::{self, PlanItem, ScanItem, Scraper},
    settings::Settings,
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

type CmdResult<T> = Result<T, String>;

fn library_change(state: &AppState, root: Option<&str>) -> CmdResult<bool> {
    let changed = state.settings().library_root.as_deref() != root;
    if changed && state.job_running() {
        return Err("有任务正在运行,请等待完成后再更换媒体库".into());
    }
    if changed {*state.catalog.lock().unwrap_or_else(|e|e.into_inner())=None; *state.append.lock().unwrap_or_else(|e|e.into_inner())=None; *state.maintenance.lock().unwrap_or_else(|e|e.into_inner())=None; *state.revision.lock().unwrap_or_else(|e|e.into_inner())=None; *state.artwork.lock().unwrap_or_else(|e|e.into_inner())=None;}
    Ok(changed)
}

/// 保留预览中的目标路径;首片未选时把共享元数据交给仍被选中的分片。
fn selected_plans(plans: &[PlanItem], item_ids: &[u32], root: &Path) -> CmdResult<Vec<PlanItem>> {
    let mut selected: Vec<_> = plans.iter().filter(|p| item_ids.contains(&p.item_id) && p.conflict.is_none()).cloned().collect();
    let mut carriers: std::collections::HashSet<String> = selected.iter().filter(|p| !p.extras.is_empty()).map(|p| p.code.clone()).collect();
    for p in &mut selected {
        if carriers.insert(p.code.clone()) {
            if let Some(donor) = plans.iter().find(|other| other.code == p.code && other.to.parent() == p.to.parent() && !other.extras.is_empty()) {
                p.ops.extend(donor.ops.iter().filter(|op| op.source().is_none()).cloned());
                p.extras = donor.extras.clone();
                p.image_candidates.extend(donor.image_candidates.clone());
            }
        }
        if p.root != root || p.ops.iter().any(|op| !op.target().starts_with(root) && !root.canonicalize().is_ok_and(|r|op.target().starts_with(r))) {
            return Err("媒体库位置已更改,请重新生成整理计划".into());
        }
    }
    Ok(selected)
}

// ---------------------------------------------------------------------------
// 设置与媒体库根目录
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn default_settings() -> Settings {
    Settings::default()
}

#[tauri::command]
pub fn settings_path() -> String {
    Settings::default_path().display().to_string()
}

fn persist(state: &AppState, settings: Settings) -> CmdResult<()> {
    persist_to(state, settings, &Settings::default_path(), None)
}

fn persist_to(state: &AppState, settings: Settings, path: &Path, root_owner: Option<u64>) -> CmdResult<()> {
    let _writer = state.settings_writer.lock().unwrap_or_else(|e| e.into_inner());
    let changed = state.settings().library_root != settings.library_root;
    if changed {
        let job = state.job.lock().unwrap_or_else(|e| e.into_inner());
        if !job.as_ref().is_some_and(|job| Some(job.id) == root_owner && job.kind == "settings") {
            return Err("媒体库位置已变化,请重新保存或使用更换媒体库入口".into());
        }
    }
    let config_changed=state.settings()!=settings;
    let scan_changed=crate::pending::key(&state.settings())!=crate::pending::key(&settings);
    settings.save(path).map_err(|e| e.to_string())?;
    kiroku_core::logging::configure(&settings.log);
    *state.settings.lock().unwrap_or_else(|e| e.into_inner()) = settings;
    if scan_changed { state.pending.lock().unwrap_or_else(|e| e.into_inner()).invalidate(); }
    if config_changed {let mut session=state.session();session.plans.clear();session.plan_settings=None;*state.append.lock().unwrap_or_else(|e|e.into_inner())=None;*state.maintenance.lock().unwrap_or_else(|e|e.into_inner())=None;}
    Ok(())
}

fn commit_library(state: &AppState, settings: Settings, candidate: Option<Library>, path: &Path, owner: u64) -> CmdResult<()> {
    persist_to(state, settings, path, Some(owner))?;
    state.replace_library(candidate);
    Ok(())
}

/// 打开(必要时创建)媒体库索引并放开海报读取范围。
pub fn open_library(app: &AppHandle, root: &Path) -> CmdResult<()> {
    let lib = Library::open(root).map_err(|e| e.to_string())?;
    allow_library_assets(app, root);
    app.state::<AppState>().replace_library(Some(lib));
    Ok(())
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, settings: Settings) -> CmdResult<()> {
    settings.validate()?;
    let before=app.state::<AppState>().settings();
    if settings.dir_template!=before.dir_template {kiroku_core::template::validate_template(&settings.dir_template,false)?;}
    if settings.file_template!=before.file_template {kiroku_core::template::validate_file_template(&settings.file_template)?;}
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let changed = library_change(&state, settings.library_root.as_deref())?;
        if changed {
            let (id, _) = state.begin_job("settings")?;
            let _guard = JobGuard { app: app.clone(), id };
            let root = settings.library_root.clone().filter(|root| !root.is_empty());
            let candidate = root.as_ref().map(|root| Library::open(Path::new(root)).map_err(|e| e.to_string())).transpose()?;
            commit_library(&state, settings, candidate, &Settings::default_path(), id)?;
            if let Some(root) = root { allow_library_assets(&app, Path::new(&root)); }
            Ok(())
        } else { persist(&state, settings) }
    }).await.map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDirs {
    /// 系统影片目录(macOS ~/Movies,Windows「视频」)
    videos: Option<String>,
    downloads: Option<String>,
    /// 建议的媒体库位置:影片目录下的 kiroku
    suggested_library: Option<String>,
}

/// 系统的媒体相关目录,用作默认位置和选文件夹对话框的起点。
#[tauri::command]
pub fn media_dirs(app: AppHandle) -> MediaDirs {
    let p = app.path();
    let videos = p.video_dir().ok();
    MediaDirs {
        suggested_library: videos.as_ref().map(|v| v.join("kiroku").display().to_string()),
        videos: videos.map(|v| v.display().to_string()),
        downloads: p.download_dir().ok().map(|d| d.display().to_string()),
    }
}

/// 选定媒体库目录:校验可写,保存设置,打开索引。`create` 为真时目录不存在就创建(用于默认位置)。
#[tauri::command]
pub async fn set_library_root(app: AppHandle, path: String, create: Option<bool>) -> CmdResult<Settings> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (id, _) = state.begin_job("settings")?;
        let _guard = JobGuard { app: app.clone(), id };
        let root = PathBuf::from(&path);
        if create.unwrap_or(false) && !root.exists() { std::fs::create_dir_all(&root).map_err(|e| e.to_string())?; }
        if !root.is_dir() { return Err(format!("目录不存在:{path}")); }
        let candidate = Library::open(&root).map_err(|e| format!("无法创建媒体库索引(需要可写):{e}"))?;
        let mut settings = state.settings(); settings.library_root = Some(path);
        commit_library(&state, settings.clone(), Some(candidate), &Settings::default_path(), id)?;
        allow_library_assets(&app, &root);
        Ok(settings)
    }).await.map_err(|e| e.to_string())?
}

/// 快速体检一个文件夹(视频数、NFO 数),供前端在选目录时给出提醒。
#[tauri::command]
pub async fn inspect_directory(path: String) -> CmdResult<pipeline::DirInfo> {
    tauri::async_runtime::spawn_blocking(move || pipeline::inspect_dir(Path::new(&path))).await.map_err(|e| e.to_string())
}

/// 记住的待整理文件夹:单个视频文件只用于本次导入,不记住。
fn remembered_sources(roots: &[String]) -> Vec<String> {
    roots.iter().filter(|r| !Path::new(r.as_str()).is_file()).cloned().collect()
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum DroppedKind { Dir, Video, File, Missing }

#[derive(Serialize, Debug, PartialEq)]
pub struct DroppedPath { path: String, kind: DroppedKind }

/// 拖入窗口的路径按跟随软链接后的类型归类;只读,只供界面决定是否加入待整理文件夹,扫描仍独立校验。
fn classify_dropped(paths: Vec<String>) -> Vec<DroppedPath> {
    paths.into_iter().map(|path| {
        let kind = match std::fs::metadata(&path) {
            Ok(m) if m.is_dir() => DroppedKind::Dir,
            Ok(_) if Path::new(&path).extension().and_then(|e| e.to_str()).is_some_and(|e| pipeline::VIDEO_EXT.contains(&e.to_ascii_lowercase().as_str())) => DroppedKind::Video,
            Ok(_) => DroppedKind::File,
            Err(_) => DroppedKind::Missing,
        };
        DroppedPath { path, kind }
    }).collect()
}

#[tauri::command]
pub async fn classify_paths(paths: Vec<String>) -> CmdResult<Vec<DroppedPath>> {
    tauri::async_runtime::spawn_blocking(move || classify_dropped(paths)).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod dropped_path_tests {
    use super::*;
    #[test]
    fn dropped_paths_are_classified_after_following_links() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("新片");
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("ABC-001.mp4");
        std::fs::write(&file, b"x").unwrap();
        let text = dir.join("notes.txt");
        std::fs::write(&text, b"x").unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        let mut input = vec![s(&dir), s(&file), s(&text), s(&t.path().join("gone"))];
        #[cfg(unix)]
        {
            let link = t.path().join("link");
            std::os::unix::fs::symlink(&dir, &link).unwrap();
            input.push(s(&link));
        }
        let kinds: Vec<DroppedKind> = classify_dropped(input.clone()).into_iter().map(|d| d.kind).collect();
        let mut expected = vec![DroppedKind::Dir, DroppedKind::Video, DroppedKind::File, DroppedKind::Missing];
        if cfg!(unix) { expected.push(DroppedKind::Dir) }
        assert_eq!(kinds, expected);
        assert_eq!(classify_dropped(input.clone())[0].path, input[0]);
        assert_eq!(remembered_sources(&input[..3]), vec![input[0].clone()], "single files are not remembered as source folders");
    }
}

/// 系统原生的选文件夹对话框;取消返回 None。`start` 为对话框初始打开的目录(存在时生效)。
#[tauri::command]
pub async fn pick_directory(app: AppHandle, title: Option<String>, start: Option<String>) -> CmdResult<Option<String>> {
    let r = tauri::async_runtime::spawn_blocking(move || {
        let mut d = app.dialog().file().set_title(title.unwrap_or_else(|| "选择文件夹".into()));
        if let Some(dir) = start.map(PathBuf::from).filter(|p| p.is_dir()) {
            d = d.set_directory(dir);
        }
        d.blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(r.and_then(|p| p.into_path().ok()).map(|p| p.display().to_string()))
}

// ---------------------------------------------------------------------------
// 任务:扫描 → 刮削 → 计划 → 执行
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ItemsPayload {
    items: Vec<ScanItem>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScanDone {
    total: usize,
    codes: usize,
    unrecognized: usize,
    /// 不算正片而跳过的视频;最多回传 SKIPPED_MAX 条,`skipped_total` 为实际总数
    skipped: Vec<pipeline::SkippedFile>,
    skipped_total: usize,
    cancelled: bool,
    errors: Vec<pipeline::ScanIssue>,
    errors_total: usize,
    complete: bool,
}

const SKIPPED_MAX: usize = 500;

#[tauri::command]
pub async fn scan_start(app: AppHandle, state: State<'_, AppState>, dirs: Vec<String>) -> CmdResult<u64> {
    if dirs.is_empty() {
        return Err("请选择要整理的文件夹".into());
    }
    // 库内的文件不会被扫描,选在库里面的待整理目录只会得到空结果,直接拦下并说明
    if let Some(root) = state.library_root() {
        if let Some(bad) = dirs.iter().find(|d| Path::new(d.as_str()).starts_with(&root)) {
            return Err(format!("「{bad}」在媒体库目录里面。请选择媒体库之外的文件夹(库内的影片已经整理过了)。"));
        }
    }
    let (id, cancel) = state.begin_job("scan")?;
    let mut s = state.settings();
    s.source_dirs = remembered_sources(&dirs);
    let exclude = state.library_root();
    let filter = s.video_filter();
    let _ = persist(&state, s);
    {
        let mut session = state.session();
        session.roots = dirs.iter().map(PathBuf::from).collect();
        session.scan.clear(); session.metas.clear(); session.plans.clear(); session.plan_settings = None;
    }

    tauri::async_runtime::spawn(async move {
        let guard = JobGuard { app: app.clone(), id };
        let (app2, cancel2) = (app.clone(), cancel.clone());
        let res = tauri::async_runtime::spawn_blocking(move || {
            let roots: Vec<PathBuf> = dirs.into_iter().map(PathBuf::from).collect();
            let mut batch: Vec<ScanItem> = vec![];
            let r = pipeline::scan(&roots, exclude.as_deref(), &filter, &cancel2, &mut |it| {
                batch.push(it.clone());
                if batch.len() >= 50 {
                    emit(&app2, "scan:items", id, ItemsPayload { items: std::mem::take(&mut batch) });
                }
            });
            if !batch.is_empty() {
                emit(&app2, "scan:items", id, ItemsPayload { items: batch });
            }
            r
        })
        .await;
        let done = match res {
            Ok(r) => {
                log::info!(target: "kiroku::scan", "扫描{}:视频 {} 个,番号 {} 个,未识别 {} 个,跳过 {} 个,读取错误 {} 个", if r.summary.complete { "完成" } else { "未完成" }, r.items.len(), r.codes, r.unrecognized, r.skipped.len(), r.summary.errors_total);
                for issue in &r.summary.errors { log::warn!(target: "kiroku::scan", "无法读取 {}: {}", issue.path.display(), issue.message); }
                let skipped_total = r.skipped.len();
                let skipped = r.skipped.into_iter().take(SKIPPED_MAX).collect();
                let d = ScanDone { total: r.items.len(), codes: r.codes, unrecognized: r.unrecognized, skipped, skipped_total, cancelled: r.summary.cancelled, errors: r.summary.errors, errors_total: r.summary.errors_total, complete: r.summary.complete };
                let st = app.state::<AppState>();
                let mut sess = st.session();
                sess.scan = r.items;
                sess.metas.clear();
                sess.plans.clear();
                d
            }
            Err(error) => ScanDone { total: 0, codes: 0, unrecognized: 0, skipped: vec![], skipped_total: 0, cancelled: cancel.is_cancelled(), errors: vec![pipeline::ScanIssue { path: PathBuf::new(), message: format!("扫描任务失败: {error}") }], errors_total: 1, complete: false },
        };
        drop(guard);
        emit(&app, "scan:done", id, done);
    });
    Ok(id)
}

/// 保存待整理文件夹列表(不能在媒体库里面)。
#[tauri::command]
pub async fn set_source_dirs(app: AppHandle, dirs: Vec<String>) -> CmdResult<Settings> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if let Some(root) = state.library_root() {
            if let Some(bad) = dirs.iter().find(|dir| Path::new(dir.as_str()).starts_with(&root)) { return Err(format!("「{bad}」在媒体库目录里面")); }
        }
        let mut settings = state.settings(); settings.source_dirs = dirs;
        persist(&state, settings.clone())?;
        Ok(settings)
    }).await.map_err(|e| e.to_string())?
}

/// 手动指定(或修正)某个扫描项的番号,返回更新后的条目。
#[tauri::command(async)]
pub fn set_item_code(state: State<'_, AppState>, item_id: u32, code: String) -> CmdResult<ScanItem> {
    if state.job_running() {
        return Err("有任务正在运行,请稍后再试".into());
    }
    let mut sess = state.session();
    let item = sess.scan.get_mut(item_id as usize).ok_or("找不到该文件")?;
    pipeline::override_code(item, &code)?;
    let updated = item.clone();
    sess.metas.clear();
    sess.plans.clear();
    Ok(updated)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScrapeDone {
    found: usize,
    not_found: usize,
    failed: usize,
    cancelled: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Eta {
    remaining_secs: u64,
}

/// 对给定的扫描项刮削(按番号去重)。
#[tauri::command]
pub async fn scrape_start(app: AppHandle, state: State<'_, AppState>, item_ids: Vec<u32>) -> CmdResult<u64> {
    let codes: Vec<_> = {
        let sess = state.session();
        item_ids.iter().filter_map(|i| sess.scan.get(*i as usize)).filter_map(|it| it.code.clone()).collect()
    };
    if codes.is_empty() {
        return Err("没有可刮削的文件".into());
    }
    let (id, cancel) = state.begin_job("scrape")?;
    let guard=JobGuard{app:app.clone(),id};
    let mut previous = state.session().metas.clone();
    if let Some(root) = state.library_root() {
        let ids: Vec<_> = codes.iter().map(|c| c.id.clone()).collect();
        let discovery_cancel=cancel.clone();
        let existing = tauri::async_runtime::spawn_blocking(move || kiroku_core::append::existing_metadata_cancellable(&root, &ids,&discovery_cancel)).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
        previous.extend(existing);
        state.session().metas.extend(previous.clone());
    }
    let scraper = scraper(state.settings())?;
    tauri::async_runtime::spawn(async move {
        let guard = guard;
        let started = Instant::now();
        let app_ev = app.clone();
        let results = pipeline::scrape_missing(&scraper, &codes, &previous, &cancel, &move |ev| {
            // 用已完成项的平均耗时估算剩余时间
            if ev.done > 0 && ev.done < ev.total {
                let per = started.elapsed().as_secs_f64() / ev.done as f64;
                emit(&app_ev, "job:eta", id, Eta { remaining_secs: (per * (ev.total - ev.done) as f64).round() as u64 });
            }
            emit(&app_ev, "scrape:event", id, ev);
        })
        .await;
        let reused=codes.iter().filter(|c|previous.contains_key(&c.id)).map(|c|&c.id).collect::<std::collections::HashSet<_>>().len();
        let mut done = ScrapeDone { found: reused, not_found: 0, failed: 0, cancelled: cancel.is_cancelled() };
        {
            let st = app.state::<AppState>();
            let mut sess = st.session();
            for (code, r) in results {
                match r {
                    Ok(m) => {
                        done.found += 1;
                        sess.metas.insert(code, m);
                    }
                    Err(pipeline::ScrapeError::NotFound) => done.not_found += 1,
                    Err(pipeline::ScrapeError::Cancelled) => {}
                    Err(_) => done.failed += 1,
                }
            }
            sess.plans.clear();
        }
        drop(guard);
        emit(&app, "scrape:done", id, done);
    });
    Ok(id)
}

/// 刮削器;AI 汉化使用设置目录下的译文缓存。
fn scraper(settings: Settings) -> CmdResult<Scraper> {
    Ok(Scraper::new(settings)?.with_translation_cache(kiroku_core::translate::cache_path(&Settings::default_path())))
}

/// 清空 AI 汉化译文缓存。
#[tauri::command]
pub async fn translation_cache_clear() -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(|| {
        kiroku_core::translate::TranslationCache::load(kiroku_core::translate::cache_path(&Settings::default_path()), 0).clear()
    }).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

/// 把内置标签译名并入(尚未保存的)名称规则;不改动已有规则。
#[tauri::command]
pub fn tag_preset_merge(rules: kiroku_core::names::NameRules) -> kiroku_core::names::PresetMerge {
    rules.merge_tags(&kiroku_core::names::tag_preset())
}

/// 长任务结束时发系统通知:设置关闭或主窗口在前台时不发。返回是否发出。
#[tauri::command]
pub fn notify_finish(app: AppHandle, state: State<'_, AppState>, title: String, body: String) -> CmdResult<bool> {
    use tauri_plugin_notification::NotificationExt;
    let focused = app.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
    if !state.settings().notify_on_finish || focused {
        return Ok(false);
    }
    app.notification().builder().title(title).body(body).show().map_err(|e| e.to_string())?;
    Ok(true)
}

/// 用用户指定的某站详情页重刮导入中的一部影片;事件与 scrape_start 相同。
#[tauri::command]
pub async fn scrape_url_start(app: AppHandle, state: State<'_, AppState>, item_ids: Vec<u32>, url: String) -> CmdResult<u64> {
    let mut codes: Vec<_> = {
        let sess = state.session();
        item_ids.iter().filter_map(|i| sess.scan.get(*i as usize)).filter_map(|it| it.code.clone()).collect()
    };
    codes.sort_by(|a, b| a.id.cmp(&b.id));
    codes.dedup_by(|a, b| a.id == b.id);
    let [code] = codes.as_slice() else { return Err("请为一部影片指定详情页".into()) };
    let code = code.clone();
    let (id, cancel) = state.begin_job("scrape")?;
    let guard = JobGuard { app: app.clone(), id };
    let scraper = scraper(state.settings())?;
    tauri::async_runtime::spawn(async move {
        let guard = guard;
        let app_ev = app.clone();
        let r = pipeline::scrape_url_one(&scraper, &code, url.trim(), &cancel, &move |ev| emit(&app_ev, "scrape:event", id, ev)).await;
        let mut done = ScrapeDone { found: 0, not_found: 0, failed: 0, cancelled: cancel.is_cancelled() };
        {
            let st = app.state::<AppState>();
            let mut sess = st.session();
            match r {
                Ok(m) => { done.found = 1; sess.metas.insert(code.id.clone(), m); }
                Err(pipeline::ScrapeError::NotFound) => done.not_found = 1,
                Err(pipeline::ScrapeError::Cancelled) => {}
                Err(_) => done.failed = 1,
            }
            sess.plans.clear();
        }
        drop(guard);
        emit(&app, "scrape:done", id, done);
    });
    Ok(id)
}

/// 为已刮削到元数据的文件生成整理计划(不触碰磁盘),返回给前端预览。
fn publish_plans(state: &AppState, id: u64, settings: Settings, root: &Path, plans: Vec<PlanItem>) -> CmdResult<Vec<PlanItem>> {
    let _writer = state.settings_writer.lock().unwrap_or_else(|e| e.into_inner());
    let job = state.job.lock().unwrap_or_else(|e| e.into_inner());
    if !job.as_ref().is_some_and(|job| job.id == id && job.kind == "plan" && !job.cancel.is_cancelled()) {
        return Err("计划已取消".into());
    }
    if state.settings() != settings || state.library_root().as_deref() != Some(root) {
        return Err("设置或媒体库已变化，请重新生成整理计划".into());
    }
    let mut session = state.session();
    session.plans = plans.clone();
    session.plan_settings = Some(settings);
    Ok(plans)
}

#[tauri::command]
pub async fn plan_build(app: AppHandle, state: State<'_, AppState>, item_ids: Vec<u32>, request_id: Option<String>) -> CmdResult<Vec<PlanItem>> {
    #[derive(Serialize, Clone)]
    #[serde(rename_all = "camelCase")]
    struct PlanStarted { request_id: Option<String> }
    let (id, cancel) = state.begin_job("plan")?;
    let _guard = JobGuard { app: app.clone(), id };
    let settings = state.settings();
    let root = state.library_root().ok_or("请先设置媒体库目录")?;
    emit(&app, "plan:started", id, PlanStarted { request_id });
    let plan_root = root.clone();
    let plan_settings=settings.clone();
    let (scan, metas) = { let session = state.session(); (session.scan.clone(), session.metas.clone()) };
    let metas = metas.into_iter().map(|(code, meta)| kiroku_core::names::project(&meta, &settings.name_rules).map(|p|(code,p.metadata))).collect::<Result<std::collections::HashMap<_,_>,_>>()?;
    log::info!(target: "kiroku::organize", "生成整理计划:{} 项", item_ids.len());
    let started = std::time::Instant::now();
    let plans = tauri::async_runtime::spawn_blocking(move || {
        let pairs: Vec<_> = item_ids.iter().filter_map(|id| scan.get(*id as usize)).filter_map(|it| it.code.as_ref().map(|code| (it,code))).collect();
        pipeline::build_plans_cancellable(&pairs, &metas, &root, &settings.dir_template, settings.plan_opts(), &cancel)
    }).await.map_err(|e| {
        log::error!(target: "kiroku::organize", "生成整理计划失败: {e}");
        e.to_string()
    })?.map_err(|e| e.to_string())?;
    let conflicts = plans.iter().filter(|p| p.conflict.is_some()).count();
    log::info!(target: "kiroku::organize", "整理计划已生成:{} 项,{} 项有冲突({} ms)", plans.len(), conflicts, started.elapsed().as_millis());
    publish_plans(&state, id, plan_settings, &plan_root, plans)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ApplyDone {
    applied: usize,
    failed: Vec<(String, String)>,
    cancelled: bool,
    journal_id: String,
    /// 移到废纸篓的原文件夹
    cleaned: Vec<String>,
    index_error: Option<String>,
}

/// 执行选中的计划项(有冲突的项会被忽略)。
#[tauri::command]
pub async fn apply_start(app: AppHandle, state: State<'_, AppState>, item_ids: Vec<u32>) -> CmdResult<u64> {
    let (id, cancel) = state.begin_job("apply")?;
    let guard = JobGuard { app: app.clone(), id };
    let root = state.library_root().ok_or("请先设置媒体库目录")?;
    let plans: Vec<PlanItem> = {
        let sess = state.session();
        if sess.plan_settings.as_ref()!=Some(&state.settings()){return Err("设置已变化，请重新生成整理计划".into())}
        selected_plans(&sess.plans, &item_ids, &root)?
    };
    if plans.is_empty() {
        return Err("没有可执行的项目,请重新生成整理计划".into());
    }
    let settings = state.settings();
    let http = settings.build_http().map_err(|e| e.to_string())?;
    let roots = if settings.cleanup_source { state.session().roots.clone() } else { vec![] };
    let filter = settings.video_filter();
    tauri::async_runtime::spawn(async move {
        let guard = guard;
        let app_ev = app.clone();
        let app_progress = app.clone();
        let cleanup = (!roots.is_empty()).then_some(pipeline::Cleanup { roots: &roots, trash: pipeline::trash_to_system, filter: &filter });
        let out = pipeline::apply_with_progress(plans, &http, &root, cleanup, &cancel,
            &move |ev| emit(&app_ev, "apply:event", id, ev),
            &move |progress| emit(&app_progress, "apply:progress", id, progress)).await;
        // 整理落盘后,按磁盘现状更新索引
        let dirs: Vec<PathBuf> = out.journal.items.iter().map(|item| item.dir.clone()).collect();
        emit(&app, "apply:progress", id, pipeline::ApplyProgress { phase: pipeline::ApplyPhase::Index,
            done: 0, total: dirs.len(), failed: 0, code: None });
        let (app2, dirs2) = (app.clone(), dirs.clone());
        let index_error = match tauri::async_runtime::spawn_blocking(move || reindex(&app2, &dirs2)).await {
            Ok(Ok(())) => None, Ok(Err(e)) => Some(e), Err(e) => Some(e.to_string()),
        };
        match &index_error {
            None => log::info!(target: "kiroku::organize", "媒体库索引已更新({} 个目录)", dirs.len()),
            Some(e) => log::error!(target: "kiroku::organize", "媒体库索引更新失败: {e}"),
        }
        drop(guard);
        emit(
            &app,
            "apply:done",
            id,
            ApplyDone {
                applied: out.applied.len(),
                failed: out.failed,
                cancelled: out.cancelled,
                journal_id: out.journal.id.clone(),
                cleaned: out.cleaned.iter().map(|p| p.display().to_string()).collect(),
                index_error,
            },
        );
        let root = root.display().to_string();
        tauri::async_runtime::spawn(async move { notify_servers(&app, &settings, id, root, dirs).await });
    });
    Ok(id)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PlexRefreshed {
    root: String,
    ok: bool,
    message: String,
}

/// 整理/撤销后通知已启用的媒体服务器。失败只提示,不影响整理结果。
async fn notify_servers(app: &AppHandle, settings: &kiroku_core::settings::Settings, id: u64, root: String, dirs: Vec<PathBuf>) {
    notify_plex(app, &settings.plex, id, root.clone(), dirs.clone()).await;
    notify_media_server(app, &settings.media_server, id, root, dirs).await;
}

/// Jellyfin / Emby(ADR-0010):按路径通知变化。
async fn notify_media_server(app: &AppHandle, cfg: &kiroku_core::media_server::MediaServerCfg, id: u64, root: String, dirs: Vec<PathBuf>) {
    if !cfg.active() || dirs.is_empty() {
        return;
    }
    let name = match cfg.kind { kiroku_core::media_server::ServerKind::Jellyfin => "Jellyfin", kiroku_core::media_server::ServerKind::Emby => "Emby" };
    let payload = match kiroku_core::media_server::notify(cfg, &dirs).await {
        Ok(n) => PlexRefreshed { root, ok: true, message: format!("已通知 {name} 刷新 {n} 个文件夹") },
        Err(e) => PlexRefreshed { root, ok: false, message: e },
    };
    emit(app, "server:refreshed", id, payload);
}

/// 设置里开启了 Plex 联动时,让 Plex 局部刷新这些目录。失败只提示,不影响整理结果。
async fn notify_plex(app: &AppHandle, cfg: &kiroku_core::settings::PlexCfg, id: u64, root: String, dirs: Vec<PathBuf>) {
    if !cfg.enabled || cfg.token.trim().is_empty() || dirs.is_empty() {
        return;
    }
    let r = kiroku_core::plex::refresh_dirs(&cfg.url, cfg.token.trim(), &dirs).await;
    let payload = match r {
        Ok(n) => PlexRefreshed { root, ok: true, message: format!("已通知 Plex 刷新 {n} 个文件夹") },
        Err(e) => PlexRefreshed { root, ok: false, message: e },
    };
    emit(app, "plex:refreshed", id, payload);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexTest {
    sections: Vec<kiroku_core::plex::PlexSection>,
    /// 包含当前媒体库目录的分区标题
    matched: Option<String>,
}

/// 测试 Plex 连接:列出分区,并找出包含媒体库目录的那个。
#[tauri::command]
pub async fn plex_test(state: State<'_, AppState>, url: String, token: String) -> CmdResult<PlexTest> {
    let sections = kiroku_core::plex::sections(&url, token.trim()).await?;
    let matched = state
        .library_root()
        .and_then(|root| kiroku_core::plex::match_section(&sections, &root).map(|s| s.title.clone()));
    Ok(PlexTest { sections, matched })
}

/// 用当前(未保存的)设置测试 Jellyfin / Emby 连接,并找出包含媒体库目录的服务器媒体库。
#[tauri::command]
pub async fn media_server_test(state: State<'_, AppState>, cfg: kiroku_core::media_server::MediaServerCfg) -> CmdResult<kiroku_core::media_server::ServerTest> {
    let root = state.library_root();
    kiroku_core::media_server::test(&cfg, root.as_deref()).await
}

/// 用当前(未保存的)AI 设置试译一句,验证 Key、模型和网络。
#[tauri::command]
pub async fn ai_test(cfg: kiroku_core::translate::AiCfg) -> CmdResult<kiroku_core::translate::Translated> {
    let t = kiroku_core::translate::Translator::new(cfg)?;
    t.translate(Some("海辺の町で、古い友人と過ごした3日間。"), None).await
}

fn reindex(app: &AppHandle, dirs: &[PathBuf]) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let library = state.lib();
    let lib = library.as_ref().ok_or("索引未打开,请重建媒体库索引")?;
    let mut errors = vec![];
    for dir in dirs { let normalized=lib.root().canonicalize().ok().and_then(|r|dir.strip_prefix(r).ok().map(|p|lib.root().join(p))).unwrap_or_else(||dir.clone()); if let Err(e) = lib.reindex_dir(&normalized) { errors.push(format!("{}: {e}", dir.display())); } }
    emit(app, "library:changed", state.current_job_id().unwrap_or(0), serde_json::json!({}));
    if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
}

#[tauri::command]
pub fn job_cancel(state: State<'_, AppState>,job_id:Option<u64>) -> bool {
    state.cancel_job_for(job_id)
}

// ---------------------------------------------------------------------------
// 历史与回滚
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn reveal_journal(app: AppHandle, state: State<'_, AppState>, journal_id: String) -> CmdResult<()> {
    if journal_id.is_empty() || !journal_id.bytes().all(|b| b.is_ascii_digit()) { return Err("无效的日志编号".into()); }
    let root = state.library_root().ok_or("媒体库未设置")?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = kiroku_core::journal::dir(&root).join(format!("journal-{journal_id}.json"));
        if !inside(&root, &path) { return Err("日志不存在或不在当前媒体库内".into()); }
        app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackResult {
    index_error: Option<String>,
    failures: Vec<String>,
    status: JournalStatus,
}

#[tauri::command]
pub async fn rollback(app: AppHandle, state: State<'_, AppState>, journal_id: String) -> CmdResult<RollbackResult> {
    if state.job_running() {
        return Err("有任务正在运行,请稍后再试".into());
    }
    // id 是时间戳;拒绝其他形态,避免拼出库外路径
    if journal_id.is_empty() || !journal_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("无效的日志编号".into());
    }
    let root = state.library_root().ok_or("请先设置媒体库目录")?;
    let settings = state.settings();
    let root_label = root.display().to_string();
    let (id, _cancel) = state.begin_job("rollback")?;
    let guard = JobGuard { app: app.clone(), id };
    let app_bg = app.clone();
    let (result, dirs) = tauri::async_runtime::spawn_blocking(move || {
        let path = kiroku_core::journal::dir(&root).join(format!("journal-{journal_id}.json"));
        let mut j = Journal::load(&path).map_err(|e| e.to_string())?;
        let mut dirs: Vec<PathBuf> = j.items.iter().map(|i| i.dir.clone()).collect();
        // 旧格式日志没有 items:从移动操作推断影片目录
        for op in &j.ops {
            if let kiroku_core::organize::Op::MoveFile { to, .. } = op {
                dirs.extend(to.parent().map(Path::to_path_buf));
            }
        }
        let failures = j.rollback();
        let index_error = reindex(&app, &dirs).err();
        Ok::<_, String>((RollbackResult { failures, index_error, status: j.status }, dirs))
    })
    .await
    .map_err(|e| e.to_string())??;
    drop(guard);
    tauri::async_runtime::spawn(async move { notify_servers(&app_bg, &settings, id, root_label, dirs).await });
    Ok(result)
}

// ---------------------------------------------------------------------------
// 媒体库浏览
// ---------------------------------------------------------------------------

fn with_read_snapshot<T>(state: &AppState, snapshot: &crate::state::LibraryReadSnapshot,
    f: impl FnOnce(&kiroku_core::library::LibraryReader) -> Result<T, kiroku_core::library::LibraryError>,
) -> CmdResult<T> {
    state.validate_read(snapshot)?;
    let reader = snapshot.handle.open().map_err(|error| error.to_string())?;
    let result = f(&reader).map_err(|error| error.to_string())?;
    state.validate_read(snapshot)?;
    Ok(result)
}

fn with_lib<T>(state: &AppState, f: impl FnOnce(&kiroku_core::library::LibraryReader) -> Result<T, kiroku_core::library::LibraryError>) -> CmdResult<T> {
    with_read_snapshot(state, &state.read_snapshot()?, f)
}

/// Capture the root generation before dispatch, and open a readonly WAL
/// connection on the blocking pool. Browsing never acquires the writer mutex.
async fn query_lib<T: Send + 'static>(app: AppHandle, f: impl FnOnce(&kiroku_core::library::LibraryReader) -> Result<T, kiroku_core::library::LibraryError> + Send + 'static) -> CmdResult<T> {
    let snapshot = app.state::<AppState>().read_snapshot()?;
    tauri::async_runtime::spawn_blocking(move || with_read_snapshot(&app.state::<AppState>(), &snapshot, f)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn library_list(app: AppHandle, query: Query) -> CmdResult<Page> {
    query_lib(app, move |l| l.list(&query)).await
}

#[tauri::command]
pub async fn library_get(app: AppHandle, code: String) -> CmdResult<Option<MovieDetail>> {
    query_lib(app, move |l| l.get(&code)).await
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RepairDone {
    root: String, code: String, written: usize, failed: Vec<(String, String)>,
    cancelled: bool, journal_id: Option<String>, index_error: Option<String>, warnings: Vec<String>,
}

/// Read current disk metadata, then execute only missing artwork through the same durable journal.
#[tauri::command]
pub async fn repair_images_start(app: AppHandle, state: State<'_, AppState>, code: String) -> CmdResult<u64> {
    let (id, cancel) = state.begin_job("repair")?;
    let guard = JobGuard { app: app.clone(), id };
    let root = state.library_root().ok_or("请先设置媒体库目录")?;
    let settings = state.settings();
    let http = settings.build_http().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn(async move {
        let (app_read, root_read, code_read) = (app.clone(), root.clone(), code.clone());
        let prepared = tauri::async_runtime::spawn_blocking(move || -> CmdResult<PlanItem> {
            let state = app_read.state::<AppState>();
            let detail = with_lib(&state, |lib| lib.get(&code_read))?.ok_or("影片不在媒体库中")?;
            let dir = PathBuf::from(detail.dir);
            if !inside(&root_read, &dir) { return Err("影片目录不在媒体库内".into()); }
            // SQLite is a cache. Artwork candidates always come from disk.
            let meta = kiroku_core::revision::capture(&root_read, &dir, &code_read)
                .map_err(|e|e.to_string())?.before;
            if meta.code != code_read { return Err("磁盘番号已变化，请重建索引后重试".into()); }
            pipeline::build_image_repair(&kiroku_core::artwork::repair_metadata(&meta, &state.settings()), &dir, &root_read).map_err(|e| e.to_string())
        }).await.unwrap_or_else(|e| Err(e.to_string()));
        let mut done = RepairDone { root: root.display().to_string(), code: code.clone(), written: 0, failed: vec![], cancelled: false, journal_id: None, index_error: None, warnings: vec![] };
        match prepared {
            Err(error) => done.failed.push((code, error)),
            Ok(plan) if plan.ops.is_empty() => done.warnings = plan.notices,
            Ok(plan) => {
                let warnings = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
                let (warnings_ev, app_ev) = (warnings.clone(), app.clone());
                let out = pipeline::apply(vec![plan], &http, &root, None, &cancel, &move |ev| {
                    warnings_ev.lock().unwrap().extend(ev.warnings.clone());
                    emit(&app_ev, "repair:event", id, ev);
                }).await;
                done.written = out.journal.ops.iter().filter(|op| matches!(op, Op::Download { .. })).count();
                done.failed = out.failed;
                done.cancelled = out.cancelled;
                done.journal_id = Some(out.journal.id.clone());
                done.warnings = warnings.lock().unwrap().clone();
                let dirs = out.journal.items.iter().map(|item| item.dir.clone()).collect::<Vec<_>>();
                let app_index = app.clone();
                done.index_error = tauri::async_runtime::spawn_blocking(move || reindex(&app_index, &dirs)).await.unwrap_or_else(|e| Err(e.to_string())).err();
            }
        }
        done.cancelled |= cancel.is_cancelled();
        drop(guard);
        emit(&app, "repair:done", id, done);
    });
    Ok(id)
}

#[tauri::command]
pub async fn library_facets(app: AppHandle) -> CmdResult<Facets> {
    query_lib(app, |l| l.facets()).await
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    root: String,
    done: usize,
    total: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Rebuilt {
    root: String,
    count: usize,
    cancelled: bool,
    error: Option<String>,
}

fn rebuild_result(result: CmdResult<usize>, cancelled: bool, root: String) -> Rebuilt {
    match result {
        Ok(count) => Rebuilt { root, count, cancelled, error: None },
        Err(error) => Rebuilt { root, count: 0, cancelled, error: Some(error) },
    }
}

/// 清空并从磁盘重建索引(用于接入已有的媒体库)。
#[tauri::command]
pub async fn library_rebuild(app: AppHandle, state: State<'_, AppState>) -> CmdResult<u64> {
    let snapshot = state.read_snapshot()?;
    state.validate_read(&snapshot)?;
    let root_label = snapshot.handle.root().display().to_string();
    let (id, cancel) = state.begin_job("rebuild")?;
    tauri::async_runtime::spawn(async move {
        let guard = JobGuard { app: app.clone(), id };
        let app2 = app.clone();
        let c2 = cancel.clone();
        let progress_root = root_label.clone();
        let res = tauri::async_runtime::spawn_blocking(move || {
            let st = app2.state::<AppState>();
            st.validate_read(&snapshot)?;
            let g = st.lib();
            st.validate_read(&snapshot)?;
            let lib = g.as_ref().ok_or("媒体库索引已关闭".to_string())?;
            let mut last = 0;
            lib.rebuild(&c2, &mut |done, total| {
                // 限频:最多约每 1% 发一次
                if done == total || done >= last + (total / 100).max(1) {
                    last = done;
                    emit(&app2, "library:progress", id, Progress { root: progress_root.clone(), done, total });
                }
            })
            .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        drop(guard);
        emit(&app, "library:rebuilt", id, rebuild_result(res, cancel.is_cancelled(), root_label));
            emit(&app, "library:changed", id, serde_json::json!({}));
    });
    Ok(id)
}

// ---------------------------------------------------------------------------
// 播放与定位:用系统默认程序打开,且只允许库目录内的文件
// ---------------------------------------------------------------------------

fn inside(root: &Path, p: &Path) -> bool {
    match (root.canonicalize(), p.canonicalize()) {
        (Ok(r), Ok(p)) => p.starts_with(&r),
        _ => false,
    }
}

fn movie_path(state: &AppState, code: &str, part: Option<u8>,cancel:&tokio_util::sync::CancellationToken,progress:&dyn Fn(u64,u64)) -> CmdResult<PathBuf> {
    let root = state.library_root().ok_or("请先设置媒体库目录")?;
    let detail = with_lib(state, |l| l.get(code))?.ok_or("影片不在媒体库中")?;
    let file = detail
        .files
        .iter()
        .find(|f| part.is_none() || f.part == part)
        .or(detail.files.first())
        .ok_or("没有可播放的文件")?;
    let p = PathBuf::from(&file.path);
    let dir=p.parent().ok_or("媒体文件路径无效")?;
    let meta=kiroku_core::append::read_meta(dir).map_err(|e|e.to_string())?;
    kiroku_core::placement::resolve_media_observed(&root,dir,&p,&meta,cancel,progress).map(|m|m.path).map_err(|e|e.to_string())
}

// IPC runs this blocking validation off the UI thread; the existing media API reports job progress.
fn open_media(app:AppHandle,state:&AppState,code:String,part:Option<u8>,kind:&'static str,request_id:Option<String>)->CmdResult<()> {
    let (id,cancel)=reliability::begin_reported_job(state,kind,request_id,|id,request_id|{
        emit(&app,"job:started",id,serde_json::json!({"requestId":request_id}));
    })?;
    let _guard=JobGuard{app:app.clone(),id};
    if cancel.is_cancelled(){return Err("媒体校验已取消".into())}
    let root=state.library_root().ok_or("请先设置媒体库目录")?.display().to_string();
    let report=|done:u64,total:u64|emit(&app,"media:progress",id,serde_json::json!({"root":root,"code":code,"kind":kind,"doneBytes":done,"totalBytes":total}));
    report(0,0);
    let last=std::cell::Cell::new(Instant::now());
    let path=movie_path(state,&code,part,&cancel,&|done,total|{if done==total||last.get().elapsed()>=std::time::Duration::from_millis(100){last.set(Instant::now());report(done,total);}})?;
    if cancel.is_cancelled(){return Err("媒体校验已取消".into())}
    if kind=="play"{app.opener().open_path(path.to_string_lossy(),None::<&str>).map_err(|e|e.to_string())}
    else{app.opener().reveal_item_in_dir(path).map_err(|e|e.to_string())}
}
#[tauri::command(async)]
pub fn play(app:AppHandle,state:State<'_,AppState>,code:String,part:Option<u8>,request_id:Option<String>)->CmdResult<()>{open_media(app,&state,code,part,"play",request_id)}

/// 打开日志文件夹(不存在时先创建)。
#[tauri::command(async)]
pub fn open_log_dir(app: AppHandle) -> CmdResult<()> {
    let dir = kiroku_core::logging::log_dir(&Settings::default_path());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| e.to_string())
}

/// 在 Finder / 资源管理器中显示。
#[tauri::command(async)]
pub fn reveal(app: AppHandle, state: State<'_, AppState>, code: String, part: Option<u8>, request_id: Option<String>) -> CmdResult<()> {
    open_media(app,&state,code,part,"reveal",request_id)
}

// ---------------------------------------------------------------------------
// 删除:移到系统废纸篓(可在废纸篓里放回),不做永久删除。
// 「只删资料」移走 NFO / sidecar / 图片等,视频和字幕留在原处;「删除影片和文件」移走整个目录。
// ---------------------------------------------------------------------------

/// 子目录里还有影片元数据时拒绝,避免连带删掉别的影片。
fn holds_other_movie(dir: &Path) -> std::io::Result<bool> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() { continue; }
        let sub = entry.path();
        for inner in std::fs::read_dir(&sub)? {
            let name = inner?.file_name().to_string_lossy().to_ascii_lowercase();
            if name == "kiroku.json" || name.ends_with(".nfo") { return Ok(true); }
        }
        if holds_other_movie(&sub)? { return Ok(true); }
    }
    Ok(false)
}

/// 视频和字幕算「文件」,其余(NFO、sidecar、图片、extrafanart 等)算「资料」。
fn is_media_entry(entry: &std::fs::DirEntry) -> std::io::Result<bool> {
    if entry.file_type()?.is_dir() { return Ok(false); }
    let name = entry.file_name().to_string_lossy().into_owned();
    if name.starts_with("._") { return Ok(false); }
    let ext = Path::new(&name).extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    Ok(pipeline::VIDEO_EXT.contains(&ext.as_str()) || kiroku_core::organize::SUB_EXT.contains(&ext.as_str()))
}

/// 删除结果:目录在删除前就已不存在时只清索引。
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TrashResult { missing: bool, trashed: usize }

/// 校验目录确实是库内、仍属于这个番号的单部影片目录,再交给 `trash`。
fn trash_movie_dir(root: &Path, dir: &Path, code: &str, keep_media: bool, trash: pipeline::TrashFn) -> CmdResult<TrashResult> {
    let lexical_inside = dir.starts_with(root) && dir != root && dir.components().all(|c| !matches!(c, std::path::Component::ParentDir));
    if !lexical_inside { return Err("影片目录不在媒体库内".into()); }
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TrashResult { missing: true, trashed: 0 }),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    let (root_abs, dir_abs) = (root.canonicalize().map_err(|e| e.to_string())?, dir.canonicalize().map_err(|e| e.to_string())?);
    if dir_abs == root_abs || !dir_abs.starts_with(&root_abs) { return Err("影片目录不在媒体库内".into()); }
    let meta = kiroku_core::append::read_meta(&dir_abs).map_err(|e| e.to_string())?;
    if meta.code != code { return Err("磁盘番号已变化，请重建索引后重试".into()); }
    if holds_other_movie(&dir_abs).map_err(|e| e.to_string())? { return Err("影片目录里还有其他影片，已拒绝删除".into()); }
    if !keep_media {
        trash(&dir_abs).map_err(|e| format!("移到废纸篓失败：{e}"))?;
        return Ok(TrashResult { missing: false, trashed: 1 });
    }
    let mut entries = vec![];
    for entry in std::fs::read_dir(&dir_abs).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        // 链接方式整理的视频要靠 sidecar 解析,资料删掉后链接就成了索引不认的孤儿
        if is_media_entry(&entry).map_err(|e| e.to_string())? {
            if entry.file_type().map_err(|e| e.to_string())?.is_symlink() { return Err("这部影片的视频是链接，只删资料会留下失效链接；请改用「删除影片和文件」（只移走链接，不动源文件）".into()); }
        } else { entries.push(entry.path()); }
    }
    let mut trashed = 0;
    for path in &entries {
        trash(path).map_err(|e| format!("已移走 {trashed} 项资料，{} 移到废纸篓失败：{e}", path.display()))?;
        trashed += 1;
    }
    Ok(TrashResult { missing: false, trashed })
}

#[tauri::command]
pub async fn library_trash(app: AppHandle, state: State<'_, AppState>, code: String, keep_media: bool) -> CmdResult<TrashResult> {
    let (id, _) = state.begin_job("trash")?;
    let guard = JobGuard { app: app.clone(), id };
    let root = state.library_root().ok_or("请先设置媒体库目录")?;
    let settings = state.settings();
    let app_bg = app.clone();
    let (result, root, dir) = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let detail = with_lib(&state, |lib| lib.get(&code))?.ok_or("影片不在媒体库中")?;
        let dir = PathBuf::from(detail.dir);
        let result = trash_movie_dir(&root, &dir, &code, keep_media, pipeline::trash_to_system);
        // 部分资料已移走时也要刷新索引
        let index = reindex(&app, std::slice::from_ref(&dir));
        let result = result?;
        index.map_err(|e| format!("已移到废纸篓，但索引更新失败：{e}"))?;
        Ok::<_, String>((result, root, dir))
    })
    .await
    .map_err(|e| e.to_string())??;
    drop(guard);
    tauri::async_runtime::spawn(async move { notify_servers(&app_bg, &settings, id, root.display().to_string(), vec![dir]).await });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiroku_core::organize::Op;

    fn fake_trash(path: &Path) -> std::io::Result<()> { if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) } }

    fn movie(dir: &Path, code: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let metadata = kiroku_core::Metadata { code: code.into(), ..Default::default() };
        std::fs::write(dir.join("kiroku.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
        std::fs::write(dir.join(format!("{code}.mp4")), b"video").unwrap();
    }

    #[test]
    fn trashing_a_movie_only_accepts_its_own_directory_inside_the_library() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("library");
        let target = root.join("Actor").join("ABC-123");
        movie(&target, "ABC-123");
        for keep in [false, true] {
            assert!(trash_movie_dir(&root, &root, "ABC-123", keep, fake_trash).is_err(), "库根目录不能删");
            assert!(trash_movie_dir(&root, &target, "XYZ-001", keep, fake_trash).unwrap_err().contains("番号"));
        }
        let outside = temporary.path().join("outside");
        movie(&outside, "ABC-123");
        assert!(trash_movie_dir(&root, &outside, "ABC-123", false, fake_trash).is_err(), "库外目录不能删");
        assert!(trash_movie_dir(&root, &root.join("Actor/../../outside"), "ABC-123", false, fake_trash).is_err(), "越界路径不能删");
        let parent = root.join("Actor");
        std::fs::write(parent.join("kiroku.json"), serde_json::to_vec(&kiroku_core::Metadata { code: "PAR-001".into(), ..Default::default() }).unwrap()).unwrap();
        assert!(trash_movie_dir(&root, &parent, "PAR-001", false, fake_trash).unwrap_err().contains("其他影片"));
        trash_movie_dir(&root, &target, "ABC-123", false, fake_trash).unwrap();
        assert!(!target.exists() && parent.exists() && outside.exists());
        // 目录已在库外被删掉:只清索引
        assert!(trash_movie_dir(&root, &target, "ABC-123", false, fake_trash).unwrap().missing);
    }

    #[test]
    fn trashing_only_metadata_keeps_videos_and_subtitles() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("library");
        let target = root.join("ABC-123");
        movie(&target, "ABC-123");
        for name in ["ABC-123.nfo", "ABC-123-poster.jpg", "ABC-123.zh.srt"] { std::fs::write(target.join(name), b"x").unwrap(); }
        std::fs::create_dir(target.join("extrafanart")).unwrap();
        let library = Library::open(&root).unwrap();
        library.reindex_dir(&target).unwrap();
        let result = trash_movie_dir(&root, &target, "ABC-123", true, fake_trash).unwrap();
        assert_eq!(result.trashed, 4);
        let mut left: Vec<_> = std::fs::read_dir(&target).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        left.sort();
        assert_eq!(left, ["ABC-123.mp4", "ABC-123.zh.srt"]);
        assert_eq!(library.reindex_dir(&target).unwrap(), None, "资料删掉后移出索引");
    }

    #[test]
    fn browsing_finishes_while_rebuild_holds_the_writer_lock() {
        use std::{sync::{Arc, mpsc}, time::{Duration, Instant}};
        use tokio_util::sync::CancellationToken;
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("library");
        let library = Library::open(&root).unwrap();
        let movie = root.join("ABC-123"); std::fs::create_dir_all(&movie).unwrap();
        let metadata = kiroku_core::Metadata { code: "ABC-123".into(), ..Default::default() };
        std::fs::write(movie.join("kiroku.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
        std::fs::write(movie.join("ABC-123.mp4"), b"video").unwrap();
        library.reindex_dir(&movie).unwrap();
        let settings = Settings { library_root: Some(root.display().to_string()), ..Default::default() };
        let state = Arc::new(AppState::new(settings.clone()));
        let (owner, _) = state.begin_job("settings").unwrap();
        commit_library(&state, settings, Some(library), &temporary.path().join("settings.json"), owner).unwrap();
        state.job.lock().unwrap().take();
        let (started_tx, started_rx) = mpsc::channel(); let (release_tx, release_rx) = mpsc::channel();
        let writer_state = state.clone();
        let writer = std::thread::spawn(move || {
            let slot = writer_state.lib();
            slot.as_ref().unwrap().rebuild(&CancellationToken::new(), &mut |_, _| {
                started_tx.send(()).unwrap(); release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (read_tx, read_rx) = mpsc::channel(); let reader_state = state.clone();
        let reader = std::thread::spawn(move || {
            let start = Instant::now();
            let result = with_lib(&reader_state, |library| library.list(&Query::default()));
            read_tx.send((result, start.elapsed())).unwrap();
        });
        let result = read_rx.recv_timeout(Duration::from_millis(500));
        release_tx.send(()).unwrap(); writer.join().unwrap(); reader.join().unwrap();
        let (page, elapsed) = result.expect("browsing must finish before releasing the writer");
        assert_eq!(page.unwrap().total, 1);
        println!("list query during held rebuild: {elapsed:?}");
    }

    #[test]
    fn queued_and_completed_reads_are_rejected_after_library_switch_or_close() {
        let temporary = tempfile::tempdir().unwrap();
        let a = temporary.path().join("a"); let b = temporary.path().join("b");
        let fixture = |root: &Path, title: &str| {
            let library = Library::open(root).unwrap();
            let dir = root.join("movie"); std::fs::create_dir(&dir).unwrap();
            let metadata = kiroku_core::Metadata { code: "ABC-123".into(), fields: kiroku_core::metadata::PartialMetadata { title: Some(title.into()), ..Default::default() }, ..Default::default() };
            std::fs::write(dir.join("kiroku.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
            std::fs::write(dir.join("ABC-123.mp4"), b"v").unwrap(); library.reindex_dir(&dir).unwrap(); library
        };
        let library_a = fixture(&a, "A"); let library_b = fixture(&b, "B");
        let settings = Settings { library_root: Some(a.display().to_string()), ..Default::default() };
        let state = AppState::new(settings.clone()); state.replace_library(Some(library_a));
        let snapshot = state.read_snapshot().unwrap();
        let result = with_read_snapshot(&state, &snapshot, |reader| {
            let old = reader.get("ABC-123")?;
            let (owner, _) = state.begin_job("settings").unwrap();
            let next = Settings { library_root: Some(b.display().to_string()), ..settings };
            commit_library(&state, next, Some(library_b), &temporary.path().join("settings.json"), owner).unwrap();
            state.job.lock().unwrap().take();
            Ok(old)
        });
        assert!(result.unwrap_err().contains("媒体库已变化"), "already executed reads must not publish after a root switch");
        assert!(with_read_snapshot::<()>(&state, &snapshot, |_| panic!("a queued stale query must not execute")).is_err());
        assert_eq!(with_lib(&state, |reader| reader.get("ABC-123")).unwrap().unwrap().meta.fields.title.as_deref(), Some("B"));
        let current = state.read_snapshot().unwrap();
        state.replace_library(None);
        assert!(state.read_snapshot().is_err()); assert!(state.validate_read(&current).is_err());
    }

    #[test]
    fn failed_library_configuration_save_preserves_the_old_index() {
        let d = tempfile::tempdir().unwrap();
        let old_root = d.path().join("old");
        let new_root = d.path().join("new");
        let old = Library::open(&old_root).unwrap();
        let movie = old_root.join("ABC-123"); std::fs::create_dir_all(&movie).unwrap();
        let meta = kiroku_core::Metadata { code: "ABC-123".into(), ..Default::default() };
        std::fs::write(movie.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        std::fs::write(movie.join("ABC-123.mp4"), b"video").unwrap();
        old.reindex_dir(&movie).unwrap();
        let state = AppState::new(Settings { library_root: Some(old_root.display().to_string()), ..Settings::default() });
        state.replace_library(Some(old));
        let snapshot = state.read_snapshot().unwrap();
        let (id, _) = state.begin_job("settings").unwrap();
        let candidate = Library::open(&new_root).unwrap();
        let settings = Settings { library_root: Some(new_root.display().to_string()), ..state.settings() };
        let blocked = d.path().join("settings.json"); std::fs::create_dir(&blocked).unwrap();
        assert!(commit_library(&state, settings, Some(candidate), &blocked, id).is_err());
        assert_eq!(state.settings().library_root.as_deref(), old_root.to_str());
        assert_eq!(state.lib().as_ref().unwrap().list(&Query::default()).unwrap().total, 1);
        assert!(state.validate_read(&snapshot).is_ok());
        assert_eq!(with_read_snapshot(&state, &snapshot, |reader| reader.list(&Query::default())).unwrap().total, 1);
    }

    #[test]
    fn stale_source_configuration_cannot_change_the_library_root() {
        let d = tempfile::tempdir().unwrap();
        let state = AppState::new(Settings { library_root: Some("/current-library".into()), ..Settings::default() });
        let stale = Settings { library_root: Some("/old-library".into()), ..state.settings() };
        assert!(persist_to(&state, stale, &d.path().join("settings.json"), None).is_err());
        assert_eq!(state.settings().library_root.as_deref(), Some("/current-library"));
        assert!(!d.path().join("settings.json").exists());
    }

    fn part(id: u32, metadata: bool) -> PlanItem {
        let from = PathBuf::from(format!("/source/ABC-123-CD{id}.mp4"));
        let to = PathBuf::from(format!("/library-a/ABC-123/ABC-123-CD{id}.mp4"));
        let mut ops = vec![Op::MoveFile { from: from.clone(), to: to.clone() }];
        let mut extras = vec![];
        if metadata {
            for name in ["movie.nfo", "kiroku.json"] {
                ops.push(Op::WriteFile { path: Path::new("/library-a/ABC-123").join(name), content: name.into() });
                extras.push(name.into());
            }
            ops.push(Op::Download { path: Path::new("/library-a/ABC-123/poster.jpg").into(), url: "https://example.invalid/poster.jpg".into(), crop: None, stamp: None });
            extras.push("poster.jpg".into());
        }
        PlanItem { item_id: id, code: "ABC-123".into(), title: None, from, to, extras, conflict: None, conflicts: vec![], selected: true, new_version: false, ops, image_candidates: Default::default(), notices: vec![], append_preview: None, root_guard:None, root: "/library-a".into() }
    }

    #[test]
    fn cancelled_planning_does_not_replace_a_previous_plan() {
        let settings = Settings { library_root: Some("/library-a".into()), ..Settings::default() };
        let state = AppState::new(settings.clone());
        state.session().plans = vec![part(1, true)];
        let (id, cancel) = state.begin_job("plan").unwrap();
        cancel.cancel();
        assert!(publish_plans(&state, id, settings, Path::new("/library-a"), vec![part(2, true)]).is_err());
        assert_eq!(state.session().plans[0].item_id, 1);
    }

    #[test]
    fn planning_publication_rejects_a_different_job_or_library() {
        let settings = Settings { library_root: Some("/library-a".into()), ..Settings::default() };
        let state = AppState::new(settings.clone());
        state.session().plans = vec![part(1, true)];
        let (id, _) = state.begin_job("plan").unwrap();
        assert!(publish_plans(&state, id + 1, settings.clone(), Path::new("/library-a"), vec![part(2, true)]).is_err());
        assert!(publish_plans(&state, id, settings, Path::new("/library-b"), vec![part(2, true)]).is_err());
        assert_eq!(state.session().plans[0].item_id, 1);
    }

    #[test]
    fn planning_cannot_publish_after_settings_commit() {
        let settings = Settings { library_root: Some("/library-a".into()), ..Settings::default() };
        let state = AppState::new(settings.clone());
        let (id, _) = state.begin_job("plan").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let changed = Settings { dir_template: "{code}".into(), file_template: "{code}.{ext}".into(), ..settings.clone() };
        assert_ne!(changed, settings);
        persist_to(&state, changed, &temp.path().join("settings.json"), None).unwrap();
        assert!(publish_plans(&state, id, settings, Path::new("/library-a"), vec![part(2, true)]).is_err());
        assert!(state.session().plans.is_empty());
    }

    #[test]
    fn failed_settings_save_keeps_the_previous_plan_and_allows_current_planning() {
        let settings = Settings { library_root: Some("/library-a".into()), ..Settings::default() };
        let state = AppState::new(settings.clone());
        state.session().plans = vec![part(1, true)];
        let (id, _) = state.begin_job("plan").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let blocked = temp.path().join("settings.json");
        std::fs::create_dir(&blocked).unwrap();
        let changed = Settings { file_template: "{code}.{ext}".into(), ..settings.clone() };
        assert!(persist_to(&state, changed, &blocked, None).is_err());
        assert_eq!(state.session().plans[0].item_id, 1);
        assert_eq!(state.settings(), settings);
        assert_eq!(publish_plans(&state, id, settings, Path::new("/library-a"), vec![part(2, true)]).unwrap()[0].item_id, 2);
    }

    #[test]
    fn selected_part_keeps_metadata_when_carrier_is_unchecked() {
        let mut second = part(2, false);
        second.ops.push(Op::MoveFile { from: "/source/ABC-123-CD2.srt".into(), to: "/library-a/ABC-123/ABC-123-CD2.srt".into() });
        let plans = [part(1, true), second];
        let selected = selected_plans(&plans, &[2], Path::new("/library-a")).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].item_id, 2);
        assert_eq!(selected[0].ops.len(), 5, "remaining part must carry its metadata and retain subtitles");
        assert_eq!(selected[0].extras, vec!["movie.nfo", "kiroku.json", "poster.jpg"]);
        assert!(matches!(&selected[0].ops[0], Op::MoveFile { from, .. } if from.ends_with("ABC-123-CD2.mp4")));
        assert!(matches!(&selected[0].ops[1], Op::MoveFile { from, .. } if from.ends_with("ABC-123-CD2.srt")));
    }

    #[test]
    fn selected_parts_write_metadata_once() {
        let plans = [part(1, true), part(2, false)];
        let selected = selected_plans(&plans, &[1, 2], Path::new("/library-a")).unwrap();
        assert_eq!(selected.iter().flat_map(|p| &p.ops).filter(|op| matches!(op, Op::WriteFile { .. })).count(), 2);
    }

    #[test]
    fn selected_plans_reject_a_different_library() {
        assert!(selected_plans(&[part(1, true)], &[1], Path::new("/library-b")).is_err());
    }

    #[test]
    fn selected_plans_reject_switching_to_an_ancestor_library() {
        assert!(selected_plans(&[part(1, true)], &[1], Path::new("/")).is_err());
    }

    #[test]
    fn library_change_invalidates_saved_revision_preview() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("movie.nfo"), "<movie><num>ABC-123</num><title>old</title></movie>").unwrap();
        let preview = kiroku_core::revision::preview(&root, &root, "ABC-123", serde_json::json!({"title":"new"})).unwrap();
        let root_label = root.display().to_string();
        let state = AppState::new(Settings { library_root: Some(root_label.clone()), ..Settings::default() });
        *state.revision.lock().unwrap() = Some(PendingRevision{token:"preview-token".into(),preview,settings:None});
        assert!(!library_change(&state, Some(&root_label)).unwrap());
        assert!(state.revision.lock().unwrap().is_some());
        assert!(library_change(&state, Some("/other-library")).unwrap());
        assert!(state.revision.lock().unwrap().is_none());
    }

    fn source_revision_fixture() -> (tempfile::TempDir, AppState, String) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let dir = root.join("ABC-001");
        std::fs::create_dir(&dir).unwrap();
        let settings = Settings {
            library_root: Some(root.display().to_string()),
            ..Settings::default()
        };
        let code = kiroku_core::code::parse_code("ABC-001").unwrap();
        let mut meta = kiroku_core::Metadata {
            code: code.id.clone(),
            ..Default::default()
        };
        meta.fields.title = Some("old".into());
        meta.raw_sources.insert(
            "javdb".into(),
            kiroku_core::metadata::SourceSnapshot::configured(
                &code,
                &settings.sources[0],
                None,
                &kiroku_core::metadata::PartialMetadata {
                    title: Some("source title".into()),
                    ..Default::default()
                },
            ),
        );
        std::fs::write(dir.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        std::fs::write(dir.join("movie.nfo"), kiroku_core::nfo::to_nfo(&meta)).unwrap();
        std::fs::write(dir.join("ABC-001.mp4"), b"video").unwrap();
        let lib = Library::open(&root).unwrap();
        lib.reindex_dir(&dir).unwrap();
        let state = AppState::new(settings.clone());
        state.replace_library(Some(lib));
        let after = kiroku_core::refresh::select_source(&meta, "title", "javdb").unwrap();
        let preview = kiroku_core::revision::prepare(&root, &dir, "ABC-001", after).unwrap();
        *state.revision.lock().unwrap() = Some(PendingRevision {
            token: "source-token".into(),
            preview,
            settings: Some(settings),
        });
        (temp, state, root.display().to_string())
    }
    #[test]
    fn source_revision_save_rejects_same_library_changed_settings_without_writes() {
        for changed_field in ["base", "cookie", "proxy", "route", "enabled"] {
            let (temp, state, root) = source_revision_fixture();
            let side = temp.path().join("ABC-001/kiroku.json");
            let nfo = temp.path().join("ABC-001/movie.nfo");
            let old_side = std::fs::read(&side).unwrap();
            let old_nfo = std::fs::read(&nfo).unwrap();
            let mut settings = state.settings();
            match changed_field {
                "base" => settings.sources[0].base = "https://changed.example".into(),
                "cookie" => settings.sources[0].cookie = "changed".into(),
                "proxy" => settings.proxy = Some("http://localhost:9999".into()),
                "route" => {
                    settings
                        .routes
                        .insert(kiroku_core::code::ContentType::Censored, vec![]);
                }
                _ => settings.sources[0].enabled = false,
            }
            persist_to(&state, settings, &temp.path().join("settings.json"), None).unwrap();
            assert!(
                commit_revision(
                    &state,
                    "source-token",
                    &root,
                    "ABC-001",
                    kiroku_core::revision::apply
                )
                .is_err(),
                "stale {changed_field} source preview saved"
            );
            assert_eq!(std::fs::read(&side).unwrap(), old_side);
            assert_eq!(std::fs::read(&nfo).unwrap(), old_nfo);
            assert_eq!(
                std::fs::read(temp.path().join("ABC-001/ABC-001.mp4")).unwrap(),
                b"video"
            );
        }
    }
    #[test]
    fn source_revision_commit_serializes_settings_change_across_consumption_and_apply() {
        use std::sync::mpsc;
        let (temp, state, root) = source_revision_fixture();
        let (mutated_tx, mutated_rx) = mpsc::channel();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (attempted_tx, attempted_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let state_ref = &state;
            let root_ref = &root;
            let save = scope.spawn(move || {
                commit_revision(state_ref, "source-token", root_ref, "ABC-001", |preview| {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    kiroku_core::revision::apply(preview)
                })
            });
            entered_rx.recv().unwrap();
            let mut changed = state.settings();
            changed.sources[0].cookie = "changed concurrently".into();
            let settings_path = temp.path().join("settings.json");
            let update = scope.spawn(move || {
                attempted_tx.send(()).unwrap();
                persist_to(state_ref, changed, &settings_path, None).unwrap();
                mutated_tx.send(()).unwrap();
            });
            attempted_rx.recv().unwrap();
            let blocked = mutated_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err();
            release_tx.send(()).unwrap();
            assert!(save.join().unwrap().is_ok());
            update.join().unwrap();
            assert!(
                blocked,
                "settings changed between preview consumption and metadata apply"
            );
        });
    }
    #[test]
    fn manual_revision_remains_valid_after_same_root_source_settings_change() {
        let (temp, state, root) = source_revision_fixture();
        {
            let mut pending = state.revision.lock().unwrap();
            let pending = pending.as_mut().unwrap();
            pending.settings = None;
            pending.preview = kiroku_core::revision::preview(
                Path::new(&root),
                &temp.path().join("ABC-001"),
                "ABC-001",
                serde_json::json!({"title":"manual"}),
            )
            .unwrap();
        }
        let mut settings = state.settings();
        settings.sources[0].cookie = "changed".into();
        persist_to(&state, settings, &temp.path().join("settings.json"), None).unwrap();
        assert!(commit_revision(
            &state,
            "source-token",
            &root,
            "ABC-001",
            kiroku_core::revision::apply
        )
        .is_ok());
    }

    #[test]
    fn library_change_is_blocked_while_a_job_is_running() {
        let state = AppState::new(Settings { library_root: Some("/library-a".into()), ..Settings::default() });
        state.begin_job("apply").unwrap();
        assert!(library_change(&state, Some("/library-b")).is_err());
        assert!(!library_change(&state, Some("/library-a")).unwrap());
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionPreview {
    token: String,
    #[serde(flatten)]
    preview: kiroku_core::revision::Preview,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionResult {
    job_id: u64,
    root: String,
    code: String,
    journal_id: String,
    error: Option<String>,
    index_error: Option<String>,
}

#[tauri::command]
pub async fn revision_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
    patch: serde_json::Value,
) -> CmdResult<RevisionPreview> {
    let (id, _) = state.begin_job("revision-preview")?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let detail = with_lib(&state, |lib| lib.get(&code))?.ok_or("影片不在媒体库中")?;
        let preview = kiroku_core::revision::preview(&root, Path::new(&detail.dir), &code, patch)
            .map_err(|e| e.to_string())?;
        let token = id.to_string();
        *state.revision.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(PendingRevision{token:token.clone(),preview:preview.clone(),settings:None});
        Ok(RevisionPreview { token, preview })
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(guard);
    result
}
fn commit_revision<T>(
    state: &AppState,
    token: &str,
    root: &str,
    code: &str,
    apply: impl FnOnce(&kiroku_core::revision::Preview) -> std::io::Result<T>,
) -> CmdResult<(kiroku_core::revision::Preview, T)> {
    // Keep settings persistence outside the final identity-check / file-commit interval.
    let _writer = state
        .settings_writer
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let pending = state
        .revision
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
        .filter(|p| p.token == token)
        .ok_or("预览已过期，请重新预览")?;
    if pending
        .settings
        .as_ref()
        .is_some_and(|settings| *settings != state.settings())
    {
        return Err("设置已变化，预览已过期，请重新预览".into());
    }
    let preview = pending.preview;
    let current_root = state.library_root().ok_or("请先设置媒体库")?;
    if current_root.canonicalize().map_err(|e| e.to_string())? != preview.root
        || Path::new(root) != preview.root
        || code != preview.code
    {
        return Err("预览的媒体库或番号不匹配".into());
    }
    let detail = with_lib(state, |lib| lib.get(code))?.ok_or("影片已不在媒体库中")?;
    if Path::new(&detail.dir)
        .canonicalize()
        .map_err(|e| e.to_string())?
        != preview.dir
    {
        return Err("影片目录已变化，请重新预览".into());
    }
    let outcome = apply(&preview).map_err(|e| e.to_string())?;
    Ok((preview, outcome))
}

#[tauri::command]
pub async fn revision_save(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    root: String,
    code: String,
) -> CmdResult<RevisionResult> {
    let (id, _) = state.begin_job("revision-save")?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (preview, out) = commit_revision(&state, &token, &root, &code, kiroku_core::revision::apply)?;
        let index_error = reindex(&app, &[preview.dir]).err();
        Ok(RevisionResult {
            job_id: id,
            root,
            code,
            journal_id: out.journal.id,
            error: out.error,
            index_error,
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(guard);
    result
}
#[tauri::command]
pub fn open_source_url(app: AppHandle, url: String) -> CmdResult<()> {
    let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("来源链接仅支持 HTTP/HTTPS".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MetadataUpdate {
    RevalidateLinks,
    TextVersion {language:String,text:kiroku_core::localization::TextVersion,select:bool},
    NfoLanguage { language: kiroku_core::localization::NfoLanguage },
    Lock {
        field: String,
        locked: bool,
    },
    Source {
        field: String,
        source: String,
    },
    /// 收藏只改 sidecar,走同样的可撤销修订
    Favorite { favorite: bool },
    Refresh {
        fields: Vec<String>,
        replace: bool,
        force: bool,
        /// 用户指定的某站详情页;给出时候选来自这一页
        #[serde(default)]
        url: Option<String>,
    },
}

#[tauri::command]
pub async fn metadata_update_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
    update: MetadataUpdate,
    request_id: Option<String>,
) -> CmdResult<RevisionPreview> {
    let (id, cancel) = reliability::begin_reported_job(&state, "metadata-update", request_id, |id, request_id| {
        emit(&app, "job:started", id, serde_json::json!({"requestId":request_id}));
    })?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let settings = state.settings();
    let capture_app = app.clone();
    let capture_root = root.clone();
    let capture_code = code.clone();
    let captured = tauri::async_runtime::spawn_blocking(move || -> CmdResult<_> {
        let state = capture_app.state::<AppState>();
        let detail = with_lib(&state, |lib| lib.get(&capture_code))?.ok_or("影片不在媒体库中")?;
        // Read the authoritative disk data through revision, never the cached index.
        kiroku_core::revision::capture(&capture_root, Path::new(&detail.dir), &capture_code)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    let before = captured.before.clone();
    let is_language_export=matches!(&update,MetadataUpdate::NfoLanguage{..});
    let after = match update {
        MetadataUpdate::RevalidateLinks => {let root=root.clone();let dir=captured.dir.clone();let meta=before.clone();tauri::async_runtime::spawn_blocking(move||kiroku_core::placement::revalidate(&root,&dir,&meta)).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?},
        MetadataUpdate::TextVersion{language,text,select} => kiroku_core::localization::set_localized(&before,&language,text,select).map_err(|e|e.to_string())?,
        MetadataUpdate::NfoLanguage{language} => {let mut after=before.clone();kiroku_core::localization::recover_original(&mut after);after.nfo_language=language;after},
        MetadataUpdate::Favorite { favorite } => { let mut after = before.clone(); after.favorite = favorite; after }
        MetadataUpdate::Lock { field, locked } => {
            kiroku_core::refresh::set_lock(&before, &field, locked).map_err(|e| e.to_string())?
        }
        MetadataUpdate::Source { field, source } => {
            if !kiroku_core::refresh::source_candidates(&before, &settings).get(&field).is_some_and(|choices|choices.iter().any(|candidate| candidate.source == source)) {
                return Err("该字段没有这个来源的有效非空候选，请更新缓存".into());
            }
            kiroku_core::refresh::select_source_named(&before, &field, &source, &settings.name_rules)
                .map_err(|e| e.to_string())?
        }
        MetadataUpdate::Refresh {
            fields,
            replace,
            force,
            url,
        } => {
            if fields
                .iter()
                .any(|f| !kiroku_core::revision::EDITABLE_FIELDS.contains(&f.as_str()))
            {
                return Err("不可更新字段".into());
            }
            let parsed = kiroku_core::code::parse_code(&code).ok_or("无法识别番号类型")?;
            let scraper = scraper(settings.clone())?;
            match url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
                Some(url) => scraper.refresh_item_url(&parsed, &before, &fields, replace, url, &cancel).await?,
                None => scraper.refresh_item(&parsed, &before, &fields, replace, force, &cancel).await?,
            }
        }
    };
    if cancel.is_cancelled() {
        return Err("已取消".into());
    }
    let result = tauri::async_runtime::spawn_blocking(move || -> CmdResult<_> {
        let state = app.state::<AppState>();
        let _writer = state.settings_writer.lock().unwrap_or_else(|e|e.into_inner());
        if state.library_root() != Some(root) || state.settings() != settings {
            return Err("媒体库或设置已变化，请重新预览".into());
        }
        let preview = captured.prepare_after(after).map_err(|e| e.to_string())?;
        let preview=if is_language_export {preview.for_language_export().map_err(|e|e.to_string())?}else{preview};
        if cancel.is_cancelled() {
            return Err("已取消".into());
        }
        let token = id.to_string();
        *state.revision.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(PendingRevision{token:token.clone(),preview:preview.clone(),settings:Some(settings)});
        Ok(RevisionPreview { token, preview })
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(guard);
    result
}

#[tauri::command]
pub async fn metadata_source_candidates(
    app: AppHandle,
    code: String,
) -> CmdResult<std::collections::BTreeMap<String, Vec<kiroku_core::refresh::SourceCandidate>>> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let settings = state.settings();
        let root = state.library_root().ok_or("请先设置媒体库")?;
        let detail = with_lib(&state, |lib| lib.get(&code))?.ok_or("影片不在媒体库中")?;
        let captured = kiroku_core::revision::capture(&root, Path::new(&detail.dir), &code)
            .map_err(|e| e.to_string())?;
        Ok(kiroku_core::refresh::source_candidates(
            &captured.before,
            &settings,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtworkOriginal {
    token: String,
    root: String,
    code: String,
    width: u32,
    height: u32,
    image: Vec<u8>,
    crop: Option<kiroku_core::artwork::CropBox>,
    source: Option<String>,
}
#[derive(Serialize)]
pub struct ArtworkPreview {
    revision: RevisionPreview,
    image: Vec<u8>,
}
#[tauri::command]
pub async fn artwork_open(
    app: AppHandle,
    state: State<'_, AppState>,
    code: String,
    request_id: Option<String>,
) -> CmdResult<ArtworkOriginal> {
    use kiroku_core::artwork;
    let (id, cancel) = reliability::begin_reported_job(&state, "artwork-open", request_id, |id, request_id| {
        emit(&app, "job:started", id, serde_json::json!({"requestId":request_id}));
    })?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    let settings = state.settings();
    let root = state.library_root().ok_or("请先设置媒体库")?;
    let (a, r, c) = (app.clone(), root.clone(), code.clone());
    let captured = tauri::async_runtime::spawn_blocking(move || -> CmdResult<_> {
        let state = a.state::<AppState>();
        let detail = with_lib(&state, |lib| lib.get(&c))?.ok_or("影片不在媒体库中")?;
        kiroku_core::revision::capture(&r, Path::new(&detail.dir), &c).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;
    let snapshot = captured.clone();
    let initial_poster = tauri::async_runtime::spawn_blocking(move || {
        kiroku_core::assets::Paths::for_movie(&snapshot.dir, &snapshot.before)
            .and_then(|paths| kiroku_core::revision::snapshot_file(&snapshot.root, &paths.poster))
            .map(|(digest, _)| digest)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let snapshot = captured.clone();
    let existing = tauri::async_runtime::spawn_blocking(move || artwork::Original::load(snapshot))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let original = if let Some(original) = existing {
        original
    } else {
        let snapshot = captured.clone();
        let (legacy, phase, local) =
            tauri::async_runtime::spawn_blocking(move || -> std::io::Result<_> {
                let (url, phase) = artwork::legacy_source(&snapshot.root, &snapshot.dir)?;
                let local = match &url {
                    Some(url) => {
                        artwork::legacy_local_original(&snapshot.root, &snapshot.dir, url)?
                    }
                    None => None,
                };
                Ok((url, phase, local))
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let candidates = artwork::repair_metadata(&captured.before, &settings);
        let mut found = if let Some(bytes) = local {
            let snap = captured.clone();
            let url = legacy.clone();
            let original_phase = captured
                .before
                .artwork_phase
                .clone()
                .unwrap_or_else(|| phase.clone());
            Some(
                tauri::async_runtime::spawn_blocking(move || {
                    artwork::Original::from_bytes(snap, bytes, url, original_phase)
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        let urls = legacy
            .into_iter()
            .chain(candidates.fields.poster_urls)
            .chain(candidates.fields.thumb_urls)
            .collect::<Vec<_>>();
        let http = settings.build_http().map_err(|e| e.to_string())?.for_images();
        let mut errors = vec![];
        for url in urls {
            if found.is_some() {
                break;
            }
            if cancel.is_cancelled() {
                return Err("已取消".into());
            }
            let response = tokio::select! {_=cancel.cancelled()=>return Err("已取消".into()),r=http.get_image(&url)=>r};
            match response {
                Ok((200, bytes)) => {
                    let snap = captured.clone();
                    let source = url.clone();
                    let phase = captured
                        .before
                        .artwork_phase
                        .clone()
                        .unwrap_or_else(|| phase.clone());
                    match tauri::async_runtime::spawn_blocking(move || {
                        artwork::Original::from_download_bytes(snap, bytes, Some(source), phase)
                    })
                    .await
                    .map_err(|e| e.to_string())?
                    {
                        Ok(original) => {
                            found = Some(original);
                            break;
                        }
                        Err(e) => errors.push(e.to_string()),
                    }
                }
                Ok((status, _)) => errors.push(format!("HTTP {status}")),
                Err(e) => errors.push(e.to_string()),
            }
        }
        found.ok_or_else(|| {
            format!(
                "无法取得未裁剪原图，请更新来源缓存后重试。{}",
                errors.join("；")
            )
        })?
    };
    let result = tauri::async_runtime::spawn_blocking(move || -> CmdResult<_> {
        let state = app.state::<AppState>();
        let _writer = state
            .settings_writer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if cancel.is_cancelled() {
            return Err("已取消".into());
        }
        if state.settings() != settings || state.library_root() != Some(root) {
            return Err("媒体库或设置已变化".into());
        }
        if kiroku_core::revision::snapshot_file(
            &original.captured.root,
            &kiroku_core::assets::Paths::for_movie(&original.captured.dir, &original.captured.before).map_err(|e| e.to_string())?.poster,
        )
        .map_err(|e| e.to_string())?
        .0 != initial_poster
        {
            return Err("读取原图期间海报已变化，请重新打开".into());
        }
        original
            .captured
            .prepare_after(original.captured.before.clone())
            .map_err(|e| e.to_string())?;
        let (width, height) = artwork::dimensions(&original.bytes).map_err(|e| e.to_string())?;
        let token = id.to_string();
        let out = ArtworkOriginal {
            token: token.clone(),
            root: original.captured.root.display().to_string(),
            code: original.captured.code.clone(),
            width,
            height,
            image: original.bytes.clone(),
            crop: original.artwork.crop,
            source: original.artwork.source_url.clone(),
        };
        *state.artwork.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(crate::state::PendingArtwork {
                token,
                original,
                settings,
            });
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(guard);
    result
}
#[tauri::command]
pub async fn artwork_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    token: String,
    root: String,
    code: String,
    crop: kiroku_core::artwork::CropBox,
    request_id: Option<String>,
) -> CmdResult<ArtworkPreview> {
    let (id, cancel) = reliability::begin_reported_job(&state, "artwork-preview", request_id, |id, request_id| {
        emit(&app, "job:started", id, serde_json::json!({"requestId":request_id}));
    })?;
    let guard = JobGuard {
        app: app.clone(),
        id,
    };
    let result = tauri::async_runtime::spawn_blocking(move || -> CmdResult<_> {
        let state = app.state::<AppState>();
        let _writer = state
            .settings_writer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let pending = state.artwork.lock().unwrap_or_else(|e| e.into_inner());
        let p = pending
            .as_ref()
            .filter(|p| p.token == token)
            .ok_or("原图已过期，请重新打开裁剪")?;
        if p.settings != state.settings()
            || Path::new(&root) != p.original.captured.root
            || code != p.original.captured.code
            || state
                .library_root()
                .and_then(|p| p.canonicalize().ok())
                .as_ref()
                != Some(&p.original.captured.root)
        {
            return Err("原图媒体库或设置已变化".into());
        }
        let stamp = p
            .settings
            .badges
            .stamping(
                &p.original.artwork.phase,
                Some(&kiroku_core::badge::user_dir(
                    &kiroku_core::settings::Settings::default_path(),
                )),
            )
            .filter(|_| p.settings.badges.poster);
        let (preview, image) = p
            .original
            .preview(crop, stamp.as_ref())
            .map_err(|e| e.to_string())?;
        if cancel.is_cancelled() {
            return Err("已取消".into());
        }
        let token = id.to_string();
        *state.revision.lock().unwrap_or_else(|e| e.into_inner()) = Some(PendingRevision {
            token: token.clone(),
            preview: preview.clone(),
            settings: Some(p.settings.clone()),
        });
        Ok(ArtworkPreview {
            revision: RevisionPreview { token, preview },
            image,
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    drop(guard);
    result
}

mod reliability;
pub use reliability::*;

mod catalog;
pub use catalog::*;

#[cfg(test)]
mod catalog_scope_tests {
    use super::*;
    #[test]
    fn switching_library_invalidates_catalog_preview() {
        let t=tempfile::tempdir().unwrap();let root=t.path();let movie=root.join("ABC-001");std::fs::create_dir(&movie).unwrap();
        let metadata=kiroku_core::Metadata{code:"ABC-001".into(),..Default::default()};
        std::fs::write(movie.join("kiroku.json"),serde_json::to_vec(&metadata).unwrap()).unwrap();
        let mut settings=Settings::default();settings.library_root=Some(root.to_string_lossy().into_owned());
        let preview=kiroku_core::catalog::preview(root,&[movie],&settings.name_rules,&tokio_util::sync::CancellationToken::new()).unwrap();
        let state=AppState::new(settings.clone());*state.catalog.lock().unwrap()=Some(("catalog-token".into(),preview,settings));
        assert!(library_change(&state,Some("/different-library")).unwrap());
        assert!(state.catalog.lock().unwrap().is_none());
    }
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct TemplatePreview { directory: String, file: String, warnings: Vec<String>, error: Option<String> }
#[tauri::command]
pub fn template_preview(directory: String, file: String) -> TemplatePreview {
    if let Err(error)=kiroku_core::template::validate_template(&directory,false).and_then(|_|kiroku_core::template::validate_template(&file,true)){return TemplatePreview{directory:String::new(),file:String::new(),warnings:vec![],error:Some(error)};}
    let mut m=kiroku_core::Metadata::default();m.code="SMPL-001".into();m.fields.title=Some("示例标题".into());m.fields.original_title=Some("サンプル".into());m.fields.release_date=Some("2024-01-02".into());m.fields.runtime_min=Some(120);m.fields.actors=vec![kiroku_core::metadata::Actor::new("示例演员",kiroku_core::metadata::Gender::Female)];
    let code=kiroku_core::code::parse_code("SMPL-001-CD2-C.mp4").unwrap();let source=Path::new("/待整理/示例文件夹/SMPL-001-CD2-C.mp4");
    match kiroku_core::template::preview_paths(&directory,&file,&kiroku_core::template::Ctx::with_source(&m,Some(&code),source)) {Ok((directory,file,warnings))=>TemplatePreview{directory,file:format!("{file}.mp4"),warnings,error:None},Err(error)=>TemplatePreview{directory:String::new(),file:String::new(),warnings:vec![],error:Some(error)}}
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct LanguageBatchPreview { token:String, root:String, items:Vec<RevisionPreview> }
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct LanguageBatchResult { job_id:u64, results:Vec<RevisionResult>, failed:Vec<String>, cancelled:bool }
#[tauri::command]
pub async fn nfo_export_preview(app:AppHandle,state:State<'_,AppState>,codes:Vec<String>,language:kiroku_core::localization::NfoLanguage,request_id:Option<String>)->CmdResult<LanguageBatchPreview> {
 let (id,cancel)=reliability::begin_reported_job(&state, "nfo-export-preview", request_id, |id, request_id| {
        emit(&app, "job:started", id, serde_json::json!({"requestId":request_id}));
    })?;let job_guard=JobGuard{app:app.clone(),id};
 let root=state.library_root().ok_or("请先设置媒体库")?;let settings=state.settings();
 let result=tauri::async_runtime::spawn_blocking(move||->CmdResult<_>{
  let state=app.state::<AppState>();let guard=kiroku_core::pipeline::RootGuard::capture(&root,false).map_err(|e|e.to_string())?;
  let mut previews=vec![];let mut seen=std::collections::BTreeSet::new();
  if codes.is_empty(){return Err("请选择影片".into());}
  for code in codes {if cancel.is_cancelled(){return Err("已取消".into());}if !seen.insert(code.clone()){continue;}
   let detail=with_lib(&state,|lib|lib.get(&code))?.ok_or("影片不在媒体库中")?;
   previews.push(kiroku_core::localization::export_preview(&root,Path::new(&detail.dir),&code,language).map_err(|e|e.to_string())?);
  }
  let _writer=state.settings_writer.lock().unwrap_or_else(|e|e.into_inner());
  if state.settings()!=settings||state.library_root()!=Some(root.clone()){return Err("媒体库或设置已变化，请重新预览".into());}guard.verify().map_err(|e|e.to_string())?;
  if cancel.is_cancelled(){return Err("已取消".into());}
  let token=id.to_string();let items=previews.iter().map(|p|RevisionPreview{token:token.clone(),preview:p.clone()}).collect();
  *state.language_export.lock().unwrap_or_else(|e|e.into_inner())=Some(crate::state::PendingLanguageExport{token:token.clone(),root:root.clone(),settings,previews,guard});
  Ok(LanguageBatchPreview{token,root:root.to_string_lossy().into_owned(),items})
 }).await.map_err(|e|e.to_string())?;drop(job_guard);result
}
#[tauri::command]
pub async fn nfo_export_apply(app:AppHandle,state:State<'_,AppState>,token:String,root:String,request_id:Option<String>)->CmdResult<LanguageBatchResult> {
 let (id,cancel)=reliability::begin_reported_job(&state, "nfo-export-apply", request_id, |id, request_id| {
        emit(&app, "job:started", id, serde_json::json!({"requestId":request_id}));
    })?;let job_guard=JobGuard{app:app.clone(),id};
 let result=tauri::async_runtime::spawn_blocking(move||->CmdResult<_>{
  let state=app.state::<AppState>();let _writer=state.settings_writer.lock().unwrap_or_else(|e|e.into_inner());
  let pending=state.language_export.lock().unwrap_or_else(|e|e.into_inner()).take().filter(|p|p.token==token).ok_or("预览已过期，请重新预览")?;
  if pending.settings!=state.settings()||Path::new(&root)!=pending.root||state.library_root()!=Some(pending.root.clone()){return Err("媒体库或设置已变化，请重新预览".into());}
  pending.guard.verify().map_err(|e|e.to_string())?;
  let mut results=vec![];let mut failed=vec![];
  for preview in pending.previews {
   if cancel.is_cancelled(){break;}
   if let Err(error)=pending.guard.verify(){failed.push(error.to_string());break;}
   let detail=match with_lib(&state,|lib|lib.get(&preview.code)){Ok(Some(detail))=>detail,_=>{failed.push(format!("{}：影片已不在媒体库中",preview.code));continue;}};
   if Path::new(&detail.dir).canonicalize().ok().as_ref()!=Some(&preview.dir){failed.push(format!("{}：影片目录已变化，请重新预览",preview.code));continue;}
   match kiroku_core::revision::apply(&preview){Ok(out)=>{let index_error=reindex(&app,&[preview.dir]).err();results.push(RevisionResult{job_id:id,root:root.clone(),code:preview.code,journal_id:out.journal.id,error:out.error,index_error});},Err(error)=>failed.push(format!("{}：{error}",preview.code))}
   emit(&app,"nfo:export",id,serde_json::json!({"completed":results.len(),"failed":failed.len()}));
  }
  Ok(LanguageBatchResult{job_id:id,results,failed,cancelled:cancel.is_cancelled()})
 }).await.map_err(|e|e.to_string())?;drop(job_guard);result
}

#[tauri::command]
pub async fn recovery_inspect(app:AppHandle,state:State<'_,AppState>)->CmdResult<kiroku_core::recovery::Report>{
 let root=state.library_root().ok_or("请先设置媒体库")?;
 let(id,cancel)=state.begin_job("recovery-check")?;let _guard=JobGuard{app,id};
 tauri::async_runtime::spawn_blocking(move||kiroku_core::recovery::inspect(&root,&cancel).map_err(|e|e.to_string())).await.map_err(|e|e.to_string())?
}
