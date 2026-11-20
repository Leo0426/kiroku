//! Inspect interrupted journals and explicitly preserve completed imports before rescanning.
use crate::{
    append,
    journal::{Journal, JournalStatus},
    pipeline::RootGuard,
    library::{Library, LibraryError},
};
use serde::Serialize;
use std::{fs, io, path::Path};
use tokio_util::sync::CancellationToken;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub kind: String,
    pub codes: Vec<String>,
    pub pending_operations: usize,
    pub message: String,
    pub can_undo: bool,
    pub can_preserve: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub root: std::path::PathBuf,
    pub entries: Vec<Entry>,
}
pub fn inspect(root: &Path, cancel: &CancellationToken) -> io::Result<Report> {
    let guard = RootGuard::capture(root, false)?;
    let normalized = root.canonicalize()?;
    let dir = crate::journal::dir(&normalized);
    append::confined_path(&normalized, &dir)?;
    let mut report = Report {
        root: root.to_path_buf(),
        entries: vec![],
    };
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            guard.verify()?;
            return Ok(report);
        }
        Err(e) => return Err(e),
    };
    for item in entries {
        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "已取消检查"));
        }
        guard.verify()?;
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        let Some(id) = name
            .strip_prefix("journal-")
            .and_then(|s| s.strip_suffix(".json"))
        else {
            continue;
        };
        let parsed = append::confined_path(&normalized, &item.path())
            .and_then(|_| append::regular(&item.path()))
            .and_then(|_| Journal::load_cancellable(&item.path(), cancel))
            .and_then(|j| {
                j.validate_recorded_targets()?;
                Ok(j)
            });
        let numeric = !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit());
        match parsed {
            Ok(j) => {
                let pending = usize::from(j.pending.is_some());
                let replacement_pending = if j.status == JournalStatus::Partial {
                    j.replacements.iter().filter(|r| !r.undone).count()
                } else {
                    0
                };
                if j.status != JournalStatus::Partial && pending == 0 {
                    continue;
                }
                let kind = if pending > 0 { "pending" } else { "partial" };
                report.entries.push(Entry {
                    id: id.into(),
                    kind: kind.into(),
                    codes: j.items.iter().take(6).map(|i| i.code.clone()).collect(),
                    pending_operations: pending + replacement_pending,
                    message: if pending > 0 {
                        "有中断操作待核对，撤销前将检查磁盘状态"
                    } else {
                        "上次操作部分完成，已完成项保留在磁盘与日志中"
                    }
                    .into(),
                    can_undo: numeric,
                    can_preserve: numeric && preservable(&j),
                });
            }
            Err(e) if cancel.is_cancelled() => return Err(e),
            Err(e) => report.entries.push(Entry {
                id: id.into(),
                kind: "corrupt".into(),
                codes: vec![],
                pending_operations: 0,
                message: format!("日志无法安全读取：{e}。请保留日志和文件"),
                can_undo: false,
                can_preserve: false,
            }),
        }
    }
    guard.verify()?;
    report.entries.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(report)
}

fn preservable(journal: &Journal) -> bool {
    matches!(journal.version, 2 | 4)
        && journal.replacements.is_empty()
        && journal.rolled_back_ops.is_empty()
        && journal.status != JournalStatus::RolledBack
        && (journal.status == JournalStatus::Partial || journal.pending.is_some())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreserveResult {
    pub root: std::path::PathBuf,
    pub journal_id: String,
    pub recorded_operations: usize,
    pub recovery_error: Option<String>,
    pub index_errors: Vec<String>,
    pub index_total: usize,
    pub index_completed: usize,
    pub index_pending: usize,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub stage: &'static str,
    pub done: usize,
    pub total: usize,
    pub elapsed_ms: u64,
}

/// Reconcile files first, then rebuild the affected cache. Partial history stays retryable.
pub fn preserve(root: &Path, id: &str, cancel: &CancellationToken) -> io::Result<PreserveResult> {
    preserve_with_index(root, id, cancel, None, &mut |_| {})
}

/// Desktop callers reuse their single writer; opening another writer on SMB can
/// conflict even when the existing connection is idle. CLI callers pass None.
pub fn preserve_with_index(
    root: &Path, id: &str, cancel: &CancellationToken, index: Option<&Library>,
    progress: &mut dyn FnMut(Progress),
) -> io::Result<PreserveResult> {
    let started = std::time::Instant::now();
    let mut report = |stage, done, total| progress(Progress {
        stage, done, total, elapsed_ms: started.elapsed().as_millis() as u64,
    });
    report("inspect", 0, 0);
    append::check_stopped(&|| cancel.is_cancelled())?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(io::Error::other("无效的日志编号"));
    }
    let guard = RootGuard::capture(root, false)?;
    let normalized = root.canonicalize()?;
    if let Some(library) = index {
        if library.root().canonicalize()? != normalized {
            return Err(io::Error::other("媒体库索引与恢复目录不匹配"));
        }
    }
    let path = crate::journal::dir(&normalized).join(format!("journal-{id}.json"));
    append::confined_path(&normalized, &path)?;
    append::regular(&path)?;
    let mut journal = Journal::load_cancellable(&path, cancel)?;
    journal.validate_recorded_targets()?;
    if !preservable(&journal) {
        return Err(io::Error::other(
            "此日志不支持保留成果后继续；修订或已开始撤销的记录请在历史中处理",
        ));
    }
    let mut owners = std::collections::BTreeSet::new();
    for item in &journal.items {
        append::check_cancel(cancel)?;
        let dir = crate::journal::replacement::recorded_path(&normalized, &journal.root, &item.dir)?;
        append::confined_path(&normalized, &dir)?;
        owners.insert(dir);
    }
    let mut dirs = owners.clone();
    for dir in journal.ops.iter().chain(journal.pending.iter().map(|p| &p.op))
        .filter_map(|op| op.target().parent()) {
        append::check_cancel(cancel)?;
        let dir = crate::journal::replacement::recorded_path(&normalized, &journal.root, dir)?;
        append::confined_path(&normalized, &dir)?;
        // An artwork subdirectory is not a movie. Unrecorded pending targets
        // still need reconciliation; do not restrict recovery to journal.items.
        if !owners.iter().any(|owner| dir.starts_with(owner)) { dirs.insert(dir); }
    }
    guard.verify()?;
    append::check_stopped(&|| cancel.is_cancelled())?;
    // Never abandon a file operation mid-reconciliation. Once it is durable,
    // the disposable index can stop at directory transaction boundaries.
    report("recover", 0, dirs.len());
    let recovery_error = journal.recover_pending().err().map(|e| e.to_string());
    let mut result = PreserveResult {
        root: root.to_path_buf(), journal_id: id.into(),
        recorded_operations: journal.ops.len(), recovery_error,
        index_errors: vec![], index_total: dirs.len(), index_completed: 0,
        index_pending: dirs.len(), cancelled: cancel.is_cancelled(),
    };
    report("index", 0, dirs.len());
    if !result.cancelled && !dirs.is_empty() {
        let owned;
        let library = match index {
            Some(library) => Ok(library),
            None => {
                owned = Library::open(&normalized);
                owned.as_ref().map_err(|e| e.to_string())
            }
        };
        match library {
            Ok(library) => {
                if library.root().canonicalize()? != normalized {
                    return Err(io::Error::other("媒体库索引与恢复目录不匹配"));
                }
                for (attempt, dir) in dirs.iter().enumerate() {
                    if cancel.is_cancelled() { break; }
                    guard.verify()?;
                    let index_dir = library.root().join(dir.strip_prefix(&normalized).expect("confined recovery directory"));
                    let update = library.reindex_dir_bounded(&index_dir, cancel);
                    let stop = match update {
                        Ok(_) => { result.index_completed += 1; false }
                        Err(LibraryError::Io(e)) if e.kind() == io::ErrorKind::Interrupted => true,
                        Err(error) => {
                            let database_error = matches!(error, LibraryError::Db(_));
                            result.index_errors.push(if database_error {
                                format!("索引暂不可写，已停止批量重试：{error}。文件成果已保留，可稍后重试核对。")
                            } else { format!("{}: {error}", dir.display()) });
                            database_error
                        }
                    };
                    result.index_pending = result.index_total - result.index_completed;
                    report("index", attempt + 1, result.index_total);
                    if stop { break; }
                }
            }
            Err(error) => result.index_errors.push(format!("索引暂不可用：{error}。文件成果已保留，可稍后重试核对。")),
        }
    }
    result.cancelled = cancel.is_cancelled();
    result.index_pending = result.index_total - result.index_completed;
    report("finished", result.index_completed, result.index_total);
    log::info!(target: "kiroku::recovery", "恢复收尾：索引 {}/{}，待更新 {}，取消 {}，用时 {} ms", result.index_completed, result.index_total, result.index_pending, result.cancelled, started.elapsed().as_millis());
    Ok(result)
}
