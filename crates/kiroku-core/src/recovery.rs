//! Read-only intake of interrupted/corrupt journals; only rollback may change files.
use crate::{
    append,
    journal::{Journal, JournalStatus},
    pipeline::RootGuard,
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
            .and_then(|_| Journal::load(&item.path()))
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
                });
            }
            Err(e) => report.entries.push(Entry {
                id: id.into(),
                kind: "corrupt".into(),
                codes: vec![],
                pending_operations: 0,
                message: format!("日志无法安全读取：{e}。请保留日志和文件"),
                can_undo: false,
            }),
        }
    }
    guard.verify()?;
    report.entries.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(report)
}
