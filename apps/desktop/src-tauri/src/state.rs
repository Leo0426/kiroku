//! 应用状态:设置、当前任务、媒体库、导入会话。

use kiroku_core::{
    library::{Library, LibraryReadHandle},
    metadata::Metadata,
    pipeline::{PlanItem, ScanItem},
    settings::Settings,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
};
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

pub struct JobHandle {
    pub id: u64,
    pub kind: &'static str,
    pub cancel: CancellationToken,
}

/// 一次导入流程里跨命令共享的数据:扫描结果 → 刮削结果 → 计划。
#[derive(Default)]
pub struct Session {
    /// 本次扫描的根目录(整理后清理原文件夹只在这些目录之内进行)
    pub roots: Vec<PathBuf>,
    pub scan: Vec<ScanItem>,
    pub metas: HashMap<String, Metadata>,
    pub plans: Vec<PlanItem>,
    pub plan_settings: Option<Settings>,
}

pub struct PendingRevision {
    pub token: String,
    pub preview: kiroku_core::revision::Preview,
    pub settings: Option<Settings>,
}

pub struct PendingArtwork {
 pub token:String, pub original:kiroku_core::artwork::Original, pub settings:Settings,
}
pub struct PendingLanguageExport {
 pub token:String, pub root:std::path::PathBuf, pub settings:Settings, pub previews:Vec<kiroku_core::revision::Preview>, pub guard:kiroku_core::pipeline::RootGuard,
}
#[derive(Clone)]
pub struct LibraryReadSnapshot { pub handle: LibraryReadHandle, generation: u64 }
#[derive(Default)]
struct LibraryReadSlot { handle: Option<LibraryReadHandle>, generation: u64 }
pub struct HistoryReadSnapshot {pub root:PathBuf,generation:u64}
pub struct AppState {
    pub history:crate::history::HistoryState,
    library_read: Mutex<LibraryReadSlot>,
    pub pending: Mutex<crate::pending::PendingCache>,
    pub catalog: Mutex<Option<(String,kiroku_core::catalog::Preview,Settings)>>,
    pub language_export: Mutex<Option<PendingLanguageExport>>,
    pub artwork: Mutex<Option<PendingArtwork>>,
    pub maintenance: Mutex<Option<(String, kiroku_core::maintenance::MaintenancePreview, Settings)>>,
    pub append: Mutex<Option<(String, kiroku_core::append::AppendPreview, Settings)>>,
    pub settings: Mutex<Settings>,
    pub settings_writer: Mutex<()>,
    pub job: Mutex<Option<JobHandle>>,
    pub lib: Mutex<Option<Library>>,
    pub session: Mutex<Session>,
    pub revision: Mutex<Option<PendingRevision>>,
    next_job: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    // 持锁线程 panic 不应让整个应用永久不可用
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        Self {
            history:Default::default(),
            library_read: Mutex::new(LibraryReadSlot::default()),
            pending: Mutex::new(crate::pending::PendingCache::default()),
            catalog: Mutex::new(None),
            language_export: Mutex::new(None),
            artwork: Mutex::new(None),
            maintenance: Mutex::new(None),
            append: Mutex::new(None),
            settings: Mutex::new(settings),
            settings_writer: Mutex::new(()),
            job: Mutex::new(None),
            lib: Mutex::new(None),
            session: Mutex::new(Session::default()),
            revision: Mutex::new(None),
            next_job: AtomicU64::new(1),
        }
    }

    pub fn settings(&self) -> Settings {
        lock(&self.settings).clone()
    }

    pub fn lib(&self) -> MutexGuard<'_, Option<Library>> {
        lock(&self.lib)
    }

    /// Publish a new root only after settings have committed. No I/O under the read slot lock.
    pub fn replace_library(&self, candidate: Option<Library>) {
        let reader = candidate.as_ref().map(Library::reader_handle);
        let previous = {
            let mut writer = self.lib();
            let mut read = lock(&self.library_read);
            read.generation += 1;
            read.handle = reader;
            std::mem::replace(&mut *writer, candidate)
        };
        drop(previous);
        self.history.cancel_all();
    }
    pub fn history_snapshot(&self)->Result<HistoryReadSnapshot,String>{
        let generation=lock(&self.library_read).generation;let root=self.library_root().ok_or("请先设置媒体库目录")?;
        Ok(HistoryReadSnapshot{root,generation})
    }
    pub fn validate_history(&self,snapshot:&HistoryReadSnapshot)->Result<(),String>{
        if lock(&self.library_read).generation!=snapshot.generation || self.library_root().as_deref()!=Some(snapshot.root.as_path()){return Err("媒体库已变化，请重新读取".into())}Ok(())
    }
    pub fn read_snapshot(&self) -> Result<LibraryReadSnapshot, String> {
        let read = lock(&self.library_read);
        let handle = read.handle.clone().ok_or("媒体库索引尚未打开")?;
        Ok(LibraryReadSnapshot { handle, generation: read.generation })
    }
    pub fn validate_read(&self, snapshot: &LibraryReadSnapshot) -> Result<(), String> {
        if lock(&self.library_read).generation != snapshot.generation || self.library_root().as_deref() != Some(snapshot.handle.root()) {
            return Err("媒体库已变化，请重新读取".into());
        }
        Ok(())
    }

    pub fn session(&self) -> MutexGuard<'_, Session> {
        lock(&self.session)
    }

    pub fn library_root(&self) -> Option<PathBuf> {
        lock(&self.settings).library_root.as_deref().filter(|s| !s.is_empty()).map(PathBuf::from)
    }

    /// 同一时间只允许一个任务。
    pub fn begin_job(&self, kind: &'static str) -> Result<(u64, CancellationToken), String> {
        let mut g = lock(&self.job);
        if let Some(j) = g.as_ref() {
            return Err(format!("已有任务在运行:{}", j.kind));
        }
        let id = self.next_job.fetch_add(1, Ordering::SeqCst);
        let cancel = CancellationToken::new();
        *g = Some(JobHandle { id, kind, cancel: cancel.clone() });
        Ok((id, cancel))
    }

    pub fn cancel_job(&self) -> bool {self.cancel_job_for(None)}
    pub fn cancel_job_for(&self,id:Option<u64>)->bool{lock(&self.job).as_ref().filter(|j|id.is_none_or(|id|j.id==id)).map(|j|j.cancel.cancel()).is_some()}

    /// 正在运行的任务编号(日志事件用它关联到任务)。
    pub fn current_job_id(&self) -> Option<u64> {
        lock(&self.job).as_ref().map(|j| j.id)
    }

    pub fn job_running(&self) -> bool {
        lock(&self.job).is_some()
    }

    fn end_job(&self, id: u64) {
        let mut g = lock(&self.job);
        if g.as_ref().is_some_and(|j| j.id == id) {
            *g = None;
        }
    }
}

/// 任务结束时(含 panic)释放任务槽。
pub struct JobGuard {
    pub app: AppHandle,
    pub id: u64,
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        self.app.state::<AppState>().end_job(self.id);
    }
}

/// 所有事件都带 jobId,前端据此忽略过期任务的事件。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Tagged<T: Serialize> {
    pub job_id: u64,
    #[serde(flatten)]
    pub data: T,
}

pub fn emit<T: Serialize + Clone>(app: &AppHandle, event: &str, job_id: u64, data: T) {
    if matches!(event, "library:changed" | "scan:done") {
        app.state::<AppState>().pending.lock().unwrap_or_else(|e| e.into_inner()).invalidate();
    }
    let _ = app.emit(event, Tagged { job_id, data });
}

/// 允许前端用 asset 协议读取库目录下的文件(海报)。
pub fn allow_library_assets(app: &AppHandle, root: &std::path::Path) {
    let _ = app.asset_protocol_scope().allow_directory(root, true);
}

#[cfg(test)]
mod history_scope_tests {
    use super::*;
    #[test] fn old_history_snapshot_fails_after_library_a_b_a_without_an_index(){
        let mut settings=Settings::default();settings.library_root=Some("/library-a".into());let state=AppState::new(settings);let old=state.history_snapshot().unwrap();
        lock(&state.settings).library_root=Some("/library-b".into());state.replace_library(None);lock(&state.settings).library_root=Some("/library-a".into());state.replace_library(None);
        assert!(state.validate_history(&old).is_err());let current=state.history_snapshot().unwrap();assert!(state.validate_history(&current).is_ok());
    }
}
