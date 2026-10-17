//! Read-only pending counts/previews, short-lived cache and scoped cancellation.
use crate::state::{AppState, JobGuard};
use kiroku_core::{
    pipeline::{self, RootGuard, ScanItem, ScanSummary},
    settings::Settings,
};
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

const CACHE_TTL: Duration = Duration::from_secs(30);
const PREVIEW_LIMIT: usize = 2000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingSummary {
    pub dirs: Vec<String>,
    #[serde(flatten)]
    pub summary: ScanSummary,
    pub checked_at: u64,
    pub cached: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingList {
    pub dirs: Vec<String>,
    pub items: Vec<ScanItem>,
    pub codes: usize,
    pub unrecognized: usize,
    #[serde(flatten)]
    pub summary: ScanSummary,
    pub checked_at: u64,
    pub cached: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Summary,
    Preview,
}
#[derive(Clone)]
enum Value {
    Summary(PendingSummary),
    Preview(PendingList),
}
impl Value {
    fn summary(&self) -> &ScanSummary {
        match self {
            Self::Summary(s) => &s.summary,
            Self::Preview(p) => &p.summary,
        }
    }
    fn cached(mut self) -> Self {
        match &mut self {
            Self::Summary(s) => s.cached = true,
            Self::Preview(p) => p.cached = true,
        };
        self
    }
    fn cancel(&mut self) {
        let summary = match self {
            Self::Summary(s) => &mut s.summary,
            Self::Preview(p) => &mut p.summary,
        };
        summary.cancelled = true;
        summary.complete = false;
    }
    fn error(&mut self, path: &std::path::Path, message: &str) {
        let summary = match self {
            Self::Summary(s) => &mut s.summary,
            Self::Preview(p) => &mut p.summary,
        };
        summary.complete = false;
        if !summary.errors.iter().any(|issue| issue.path == path) {
            summary.errors_total += 1;
            if summary.errors.len() < 100 {
                summary.errors.push(pipeline::ScanIssue {
                    path: path.into(),
                    message: message.into(),
                });
            }
        }
    }
}

struct Sources(Vec<(PathBuf, Result<RootGuard, String>)>);
impl Sources {
    fn capture(roots: &[PathBuf], cancel: &CancellationToken) -> Self {
        Self(
            roots
                .iter()
                .take_while(|_| !cancel.is_cancelled())
                .map(|root| {
                    (
                        root.clone(),
                        RootGuard::capture(root, false).map_err(|error| error.to_string()),
                    )
                })
                .collect(),
        )
    }
    fn verify(self, value: &mut Value) -> Option<Vec<RootGuard>> {
        let mut guards = vec![];
        let mut valid = true;
        for (root, captured) in self.0 {
            let verified = captured.and_then(|guard| {
                guard
                    .verify()
                    .map(|_| guard)
                    .map_err(|error| error.to_string())
            });
            match verified {
                Ok(guard) => guards.push(guard),
                Err(error) => {
                    valid = false;
                    let reason = error
                        .replace("媒体库", "来源")
                        .replace("库目录", "来源目录")
                        .replace("库路径", "来源路径")
                        .replace("；新建库需明确选择创建", "");
                    value.error(&root, &format!("来源检查失败：{reason}"));
                }
            }
        }
        valid.then_some(guards)
    }
    fn available_roots(&self) -> Vec<PathBuf> {
        self.0
            .iter()
            .filter(|(_, guard)| guard.is_ok())
            .map(|(root, _)| root.clone())
            .collect()
    }
}
#[derive(Clone)]
struct Cached {
    key: String,
    at: Instant,
    guards: Vec<RootGuard>,
    value: Value,
}
impl Cached {
    fn get(&self, key: &str, now: Instant) -> Option<Value> {
        (self.key == key
            && now.duration_since(self.at) < CACHE_TTL
            && self.guards.iter().all(|guard| guard.verify().is_ok()))
        .then(|| self.value.clone().cached())
    }
}
struct Active {
    request_id: String,
    job_id: u64,
    cancel: CancellationToken,
}
#[derive(Default)]
pub struct PendingCache {
    generation: u64,
    entries: HashMap<Kind, Cached>,
    active: Option<Active>,
    cancelled_requests: VecDeque<String>,
}
impl PendingCache {
    pub fn invalidate(&mut self) {
        self.generation += 1;
        self.entries.clear();
        if let Some(active) = &self.active {
            active.cancel.cancel();
        }
    }
    #[cfg(test)]
    fn get(&self, kind: Kind, key: &str, now: Instant) -> Option<Value> {
        self.entries
            .get(&kind)
            .and_then(|entry| entry.get(key, now))
    }
    pub fn cancel(&mut self, request: &str) -> bool {
        if request.is_empty() || request.len() > 100 {
            return false;
        }
        // Cancellation can arrive before the worker registers its active job.
        if !self.cancelled_requests.iter().any(|id| id == request) {
            self.cancelled_requests.push_back(request.into());
            if self.cancelled_requests.len() > 128 {
                self.cancelled_requests.pop_front();
            }
        }
        self.active
            .as_ref()
            .filter(|active| active.request_id == request)
            .is_some_and(|active| {
                active.cancel.cancel();
                true
            })
    }
    fn register(
        &mut self,
        request_id: String,
        job_id: u64,
        cancel: CancellationToken,
        generation: u64,
    ) {
        if self
            .cancelled_requests
            .iter()
            .any(|request| request == &request_id)
            || self.generation != generation
        {
            cancel.cancel();
        }
        self.cancelled_requests
            .retain(|request| request != &request_id);
        self.active = Some(Active {
            request_id,
            job_id,
            cancel,
        });
    }
}

pub fn key(settings: &Settings) -> String {
    serde_json::to_string(&(
        &settings.source_dirs,
        &settings.library_root,
        &settings.scan,
    ))
    .expect("scan settings serialize")
}

struct PendingGuard {
    job: JobGuard,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        let state = self.job.app.state::<AppState>();
        let mut cache = state.pending.lock().unwrap_or_else(|e| e.into_inner());
        if cache
            .active
            .as_ref()
            .is_some_and(|active| active.job_id == self.job.id)
        {
            cache.active = None;
        }
    }
}

async fn read(
    app: AppHandle,
    kind: Kind,
    request_id: String,
    force: bool,
) -> Result<Value, String> {
    if request_id.is_empty() || request_id.len() > 100 {
        return Err("无效的扫描请求".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let settings = state.settings();
        if settings.source_dirs.is_empty() {
            let checked_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let summary = ScanSummary {
                complete: true,
                ..Default::default()
            };
            return Ok(match kind {
                Kind::Summary => Value::Summary(PendingSummary {
                    dirs: vec![],
                    summary,
                    checked_at,
                    cached: false,
                }),
                Kind::Preview => Value::Preview(PendingList {
                    dirs: vec![],
                    items: vec![],
                    codes: 0,
                    unrecognized: 0,
                    summary,
                    checked_at,
                    cached: false,
                }),
            });
        }
        let key = key(&settings);
        let (generation, candidate) = {
            let cache = state.pending.lock().unwrap_or_else(|e| e.into_inner());
            (cache.generation, cache.entries.get(&kind).cloned())
        };
        // Root identity checks can block on network disks. Never hold the cache
        // mutex (also used by cancellation and UI events) across filesystem I/O.
        if !force {
            if let Some(mut value) = candidate.and_then(|entry| entry.get(&key, Instant::now())) {
                let cache = state.pending.lock().unwrap_or_else(|e| e.into_inner());
                if cache.generation == generation {
                    if cache
                        .cancelled_requests
                        .iter()
                        .any(|request| request == &request_id)
                    {
                        value.cancel();
                    }
                    return Ok(value);
                }
            }
        }
        let (id, cancel) = state.begin_job("pending-scan")?;
        let _guard = PendingGuard {
            job: JobGuard {
                app: app.clone(),
                id,
            },
        };
        {
            let mut cache = state.pending.lock().unwrap_or_else(|e| e.into_inner());
            cache.register(request_id, id, cancel.clone(), generation);
        }
        let dirs = settings.source_dirs.clone();
        let roots: Vec<PathBuf> = dirs.iter().map(PathBuf::from).collect();
        let exclude = settings.library_root.as_ref().map(PathBuf::from);
        let filter = settings.video_filter();
        let sources = Sources::capture(&roots, &cancel);
        // Failed root captures are reported once by Sources::verify. Do not
        // traverse them again and double-count errors beyond the detail cap.
        let scanned_roots = sources.available_roots();
        let checked_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut value = match kind {
            Kind::Summary => Value::Summary(PendingSummary {
                dirs,
                summary: pipeline::scan_summary(
                    &scanned_roots,
                    exclude.as_deref(),
                    &filter,
                    &cancel,
                ),
                checked_at,
                cached: false,
            }),
            Kind::Preview => {
                let result = pipeline::scan_preview(
                    &scanned_roots,
                    exclude.as_deref(),
                    &filter,
                    &cancel,
                    PREVIEW_LIMIT,
                );
                Value::Preview(PendingList {
                    dirs,
                    items: result.items,
                    codes: result.codes,
                    unrecognized: result.unrecognized,
                    summary: result.summary,
                    checked_at,
                    cached: false,
                })
            }
        };
        let guards = sources.verify(&mut value);
        let current_key = self::key(&state.settings());
        let mut cache = state.pending.lock().unwrap_or_else(|e| e.into_inner());
        if cancel.is_cancelled() {
            value.cancel();
        }
        if generation == cache.generation
            && key == current_key
            && !value.summary().cancelled
            && value.summary().errors_total == 0
        {
            if let Some(guards) = guards {
                cache.entries.insert(
                    kind,
                    Cached {
                        key,
                        at: Instant::now(),
                        guards,
                        value: value.clone(),
                    },
                );
            }
        }
        Ok(value)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn pending_summary(
    app: AppHandle,
    request_id: String,
    force: Option<bool>,
) -> Result<PendingSummary, String> {
    match read(app, Kind::Summary, request_id, force.unwrap_or(false)).await? {
        Value::Summary(value) => Ok(value),
        _ => unreachable!(),
    }
}
#[tauri::command]
pub async fn pending_list(
    app: AppHandle,
    request_id: String,
    force: Option<bool>,
) -> Result<PendingList, String> {
    match read(app, Kind::Preview, request_id, force.unwrap_or(false)).await? {
        Value::Preview(value) => Ok(value),
        _ => unreachable!(),
    }
}
#[tauri::command]
pub async fn pending_cancel(app: AppHandle, request_id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<AppState>()
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel(&request_id)
    })
    .await
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_reuses_only_matching_fresh_available_sources_and_scoped_cancel() {
        let root = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.source_dirs = vec![root.path().display().to_string()];
        let cache_key = key(&settings);
        let at = Instant::now();
        let value = Value::Summary(PendingSummary {
            dirs: settings.source_dirs.clone(),
            summary: ScanSummary {
                complete: true,
                ..Default::default()
            },
            checked_at: 1,
            cached: false,
        });
        let mut cache = PendingCache::default();
        cache.entries.insert(
            Kind::Summary,
            Cached {
                key: cache_key.clone(),
                at,
                guards: vec![RootGuard::capture(root.path(), false).unwrap()],
                value,
            },
        );
        assert!(cache
            .get(Kind::Summary, &cache_key, at + Duration::from_secs(29))
            .is_some());
        assert!(cache
            .get(Kind::Summary, &cache_key, at + CACHE_TTL)
            .is_none());
        settings.scan.min_video_mb = 100;
        assert!(cache.get(Kind::Summary, &key(&settings), at).is_none());
        let cancel = CancellationToken::new();
        cache.active = Some(Active {
            request_id: "owned".into(),
            job_id: 1,
            cancel: cancel.clone(),
        });
        assert!(!cache.cancel("different"));
        assert!(!cancel.is_cancelled());
        assert!(cache.cancel("owned"));
        assert!(cancel.is_cancelled());
        std::fs::remove_dir(root.path()).unwrap();
        assert!(cache.get(Kind::Summary, &cache_key, at).is_none());
        cache.invalidate();
        assert!(cache.entries.is_empty());
    }
    #[test]
    fn cancellation_before_registration_and_invalidation_cannot_be_lost() {
        let mut cache = PendingCache::default();
        assert!(!cache.cancel("early"));
        let early = CancellationToken::new();
        cache.register("early".into(), 1, early.clone(), 0);
        assert!(early.is_cancelled());
        let late = CancellationToken::new();
        cache.invalidate();
        cache.register("late".into(), 2, late.clone(), 0);
        assert!(late.is_cancelled());
        let other = CancellationToken::new();
        cache.register("current".into(), 3, other.clone(), cache.generation);
        assert!(!cache.cancel("early"));
        assert!(!other.is_cancelled());
    }
    #[test]
    fn source_guards_bind_before_scan_and_reject_a_replaced_root() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("source");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("ABC-123.mp4"), b"v").unwrap();
        let roots = [root.clone()];
        let cancel = CancellationToken::new();
        let guards = Sources::capture(&roots, &cancel);
        let mut value = Value::Summary(PendingSummary {
            dirs: vec![root.display().to_string()],
            summary: pipeline::scan_summary(&roots, None, &Default::default(), &cancel),
            checked_at: 1,
            cached: false,
        });
        std::fs::rename(&root, parent.path().join("disconnected")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(guards.verify(&mut value).is_none());
        assert!(!value.summary().complete);
        assert_eq!(value.summary().errors_total, 1);
        assert_eq!(value.summary().errors[0].path, root);
    }
    #[test]
    fn missing_sources_are_counted_once_beyond_the_error_detail_cap() {
        let parent = tempfile::tempdir().unwrap();
        let roots = (0..150)
            .map(|i| parent.path().join(i.to_string()))
            .collect::<Vec<_>>();
        let cancel = CancellationToken::new();
        let sources = Sources::capture(&roots, &cancel);
        assert!(sources.available_roots().is_empty());
        let mut value = Value::Summary(PendingSummary {
            dirs: vec![],
            summary: ScanSummary {
                complete: true,
                ..Default::default()
            },
            checked_at: 1,
            cached: false,
        });
        assert!(sources.verify(&mut value).is_none());
        assert_eq!(value.summary().errors_total, 150);
        assert_eq!(value.summary().errors.len(), 100);
        assert!(!value.summary().complete);
    }
}
