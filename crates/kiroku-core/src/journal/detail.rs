use super::{Journal, JournalStatus};
use crate::{append, pipeline::RootGuard};
use serde::Serialize;
use std::{io, path::Path};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryOperation {
    pub index: usize,
    pub description: String,
    pub state: &'static str,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDetail {
    pub id: String,
    pub status: JournalStatus,
    pub operations: Vec<HistoryOperation>,
    pub total: usize,
    pub next: Option<usize>,
}
impl HistoryDetail {
    /// Display-only projection. Never serialize embedded NFO/sidecar content,
    /// download credentials or recovery proofs to the view.
    pub fn read(
        root: &Path,
        id: &str,
        offset: usize,
        cancel: &CancellationToken,
    ) -> io::Result<Self> {
        if id.is_empty() || id.len() > 100 || !id.bytes().all(|c| c.is_ascii_digit() || c == b'-') {
            return Err(io::Error::other("日志编号无效"));
        }
        append::check_cancel(cancel)?;
        let guard = RootGuard::capture(root, false)?;
        let path = super::dir(root).join(format!("journal-{id}.json"));
        append::confined_path(root, &path)?;
        append::regular(&path)?;
        let before = super::history::file_stamp(&path)?;
        let journal = Journal::load_cancellable(&path, cancel)?;
        let total = journal.ops.len()
            + journal.replacements.len()
            + usize::from(journal.pending.is_some())
            + journal.trashed.len();
        let end = offset.saturating_add(100).min(total);
        let operations = journal
            .ops
            .iter()
            .enumerate()
            .map(|(i, op)| {
                (
                    op.describe(),
                    if journal.rolled_back_ops.contains(&i)
                        || journal.status == JournalStatus::RolledBack
                    {
                        "undone"
                    } else {
                        "recorded"
                    },
                )
            })
            .chain(journal.replacements.iter().map(|r| {
                (
                    format!("修订 {}", r.path.display()),
                    if r.undone { "undone" } else { "recorded" },
                )
            }))
            .chain(journal.pending.iter().map(|p| (p.op.describe(), "pending")))
            .chain(
                journal
                    .trashed
                    .iter()
                    .map(|p| (format!("移到废纸篓 {}", p.display()), "trashed")),
            )
            .enumerate()
            .skip(offset)
            .take(100)
            .map(|(index, (description, state))| HistoryOperation {
                index,
                description,
                state,
            })
            .collect();
        guard.verify()?;
        append::check_cancel(cancel)?;
        if before != super::history::file_stamp(&path)? {
            return Err(io::Error::other("读取期间日志已变化，请重试"));
        }
        Ok(Self {
            id: id.into(),
            status: journal.status,
            operations,
            total,
            next: (end < total).then_some(end),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detail_is_bounded_redacts_content_rejects_paths_and_cancels() {
        let root = tempfile::tempdir().unwrap();
        let mut j = Journal::new(root.path());
        j.push_ops((0..205).map(|i| crate::organize::Op::WriteFile {
            path: root.path().join(format!("{i}.nfo")),
            content: "private contents".into(),
        }));
        j.rolled_back_ops.push(0);
        j.save().unwrap();
        let cancel = CancellationToken::new();
        let first = HistoryDetail::read(root.path(), &j.id, 0, &cancel).unwrap();
        assert_eq!(first.operations.len(), 100);
        assert_eq!(first.total, 205);
        assert_eq!(first.next, Some(100));
        assert_eq!(first.operations[0].state, "undone");
        assert!(!serde_json::to_string(&first)
            .unwrap()
            .contains("private contents"));
        let last = HistoryDetail::read(root.path(), &j.id, 200, &cancel).unwrap();
        assert_eq!(last.operations.len(), 5);
        assert_eq!(last.next, None);
        assert!(HistoryDetail::read(root.path(), "../other", 0, &cancel).is_err());
        cancel.cancel();
        assert_eq!(
            HistoryDetail::read(root.path(), &j.id, 0, &cancel)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
    }
}
