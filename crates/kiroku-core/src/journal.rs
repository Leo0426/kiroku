//! 整理批次的操作日志:JSON 存于 `<库根>/.kiroku/journal-<id>.json`,用于历史与回滚。
//! 兼容 CLI 早期写出的「裸 `Vec<Op>`」格式。

mod durable;
mod storage;
mod history;
mod detail;
pub use detail::{HistoryDetail,HistoryOperation};
pub use history::{HistoryCache,HistoryPage,HistorySummary,HistoryFilter};
pub(crate) mod replacement;
pub use replacement::{Replacement, ReplacementInput, ReplacementCheckpoint};
pub use durable::Checkpoint;

use crate::organize::Op;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::{Path, PathBuf}};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JournalStatus {
    Applied,
    /// 被取消或部分失败,已完成的部分仍可回滚
    Partial,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalItem {
    pub code: String,
    /// 影片目录(库根下)
    pub dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    #[serde(skip)]
    storage: storage::Storage,
    /// 毫秒时间戳字符串
    pub id: String,
    pub ts: u64,
    pub root: PathBuf,
    #[serde(default = "legacy_version")]
    pub version: u32,
    pub status: JournalStatus,
    pub ops: Vec<Op>,
    /// 已成功撤销的操作索引;保留原 ops,重试只处理剩余项。
    #[serde(default)]
    pub rolled_back_ops: Vec<usize>,
    /// 新格式记录每项的提交依据;旧日志默认没有。
    #[serde(default)]
    pub proofs: std::collections::BTreeMap<usize, Proof>,
    #[serde(default)]
    pub pending: Option<Pending>,
    pub items: Vec<JournalItem>,
    /// 整理后移到废纸篓的原文件夹(回滚不会恢复,需要时从废纸篓找回)
    #[serde(default)]
    pub trashed: Vec<PathBuf>,
    #[serde(default)]
    pub replacements: Vec<Replacement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proof { pub stage: PathBuf, pub digest: String, #[serde(default)] pub link: Option<PathBuf>, #[serde(default)] pub source_proof: Option<String> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending { pub op: Op, pub proof: Proof }

#[derive(Debug)]
pub struct HistoryEntry { pub id: String, pub journal: Option<Journal>, pub error: Option<String> }

fn legacy_version() -> u32 { 1 }

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 日志目录。
pub fn dir(root: &Path) -> PathBuf {
    root.join(".kiroku")
}

impl Journal {
    fn is_original_asset(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(".kiroku-original-") && name.ends_with(".png"))
    }

    /// Check at the deletion boundary, after sidecar rollback was actually attempted.
    /// Preflight cannot predict restore-stage, permission, rename or removal failures.
    fn ensure_original_unreferenced(&self, path: &Path) -> io::Result<()> {
        if !Self::is_original_asset(path) {
            return Ok(());
        }
        let path = replacement::recorded_path(&self.root, &self.root, path)?;
        let side = path
            .parent()
            .ok_or_else(|| io::Error::other("原图路径无效"))?
            .join(crate::organize::SIDECAR);
        replacement::within(&self.root, &side)?;
        if let Some(bytes) = replacement::read(&side)? {
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| io::Error::other("sidecar 无法读取，保留原图"))?;
            if value["artwork"]["original"].as_str()
                == path.file_name().and_then(|name| name.to_str())
            {
                return Err(io::Error::other(
                    "sidecar 仍引用原图，恢复未完成；已保留原图，可修复冲突后重试撤销",
                ));
            }
        }
        Ok(())
    }
    /// Newly retained originals may outlive the revision/import that created them. Do not
    /// remove one while a newer sidecar still references it, or before its sidecar can undo.
    fn protect_original_references(&self) -> io::Result<()> {
        use replacement::{current, digest, read, recorded_path};
        let created = self
            .replacements
            .iter()
            .filter(|r| r.before.is_none() && !r.undone)
            .map(|r| r.path.as_path())
            .chain(
                self.ops
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !self.rolled_back_ops.contains(i))
                    .filter_map(|(_, op)| match op {
                        Op::Download { path, .. } => Some(path.as_path()),
                        _ => None,
                    }),
            );
        for path in created {
            let Some(name) = path
                .file_name()
                .and_then(|s| s.to_str())
                .filter(|n| n.starts_with(".kiroku-original-") && n.ends_with(".png"))
            else {
                continue;
            };
            let path = recorded_path(&self.root, &self.root, path)?;
            let side = path
                .parent()
                .ok_or_else(|| io::Error::other("原图路径无效"))?
                .join(crate::organize::SIDECAR);
            replacement::within(&self.root, &side)?;
            let Some(bytes) = read(&side)? else { continue };
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| io::Error::other("sidecar 无法读取，保留原图"))?;
            if value["artwork"]["original"].as_str() != Some(name) {
                continue;
            }
            let conflict = || {
                io::Error::other(
                    "原图仍由较新或无法恢复的 sidecar 引用，请先撤销后续修订；已保留全部文件",
                )
            };
            if let Some(r) = self
                .replacements
                .iter()
                .find(|r| !r.undone && recorded_path(&self.root, &self.root, &r.path).is_ok_and(|p| p == side))
            {
                if digest(&bytes) != r.after {
                    return Err(conflict());
                }
                if let Some(before) = &r.before {
                    let backup = r.backup.as_ref().ok_or_else(conflict)?;
                    let backup = recorded_path(&self.root, &self.root, backup)?;
                    if current(&backup)?.as_ref() != Some(before) {
                        return Err(conflict());
                    }
                }
            } else if let Some((i, Op::WriteFile { content, .. })) =
                self.ops.iter().enumerate().find(|(i, op)| {
                    !self.rolled_back_ops.contains(i)
                        && matches!(op,Op::WriteFile{path,..} if recorded_path(&self.root, &self.root, path).is_ok_and(|p| p == side))
                })
            {
                if let Some(proof) = self.proofs.get(&i) {
                    // v2 deliberately strips WriteFile content; its proof is the byte authority.
                    let stage = recorded_path(&self.root, &self.root, &proof.stage)?;
                    if digest(&bytes) != proof.digest
                        || current(&stage)?.as_ref() != Some(&proof.digest)
                        || !same_file::is_same_file(&side, &stage).unwrap_or(false)
                    {
                        return Err(conflict());
                    }
                } else if bytes != content.as_bytes() {
                    return Err(conflict());
                }
            } else {
                return Err(conflict());
            }
        }
        Ok(())
    }
    /// 损坏日志也保留在历史中,避免把无法恢复误报为没有记录。
    pub fn inspect(root: &Path) -> io::Result<Vec<HistoryEntry>> {
        let guard=crate::pipeline::RootGuard::capture(root,false)?;
        crate::append::confined_path(root,&dir(root))?;
        let entries = match fs::read_dir(dir(root)) { Ok(e) => e, Err(e) if e.kind() == io::ErrorKind::NotFound => {guard.verify()?;return Ok(vec![])}, Err(e) => return Err(e) };
        let mut out = vec![];
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_prefix("journal-").and_then(|n| n.strip_suffix(".json")) else { continue };
            let (journal, error) = match Self::load(&entry.path()) { Ok(j) => (Some(j),None), Err(e) => (None,Some(e.to_string())) };
            out.push(HistoryEntry { id: id.into(), journal, error });
        }
        guard.verify()?;
        out.sort_by(|a,b| b.id.cmp(&a.id));
        Ok(out)
    }
    pub fn new(root: &Path) -> Self {
        let ts = now_ms();
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = format!("{}{:06}{:06}", ts, std::process::id(), NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        Self { id, ts, root: std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf()), version: 2, status: JournalStatus::Applied, ops: vec![], rolled_back_ops: vec![], proofs: Default::default(), pending: None, items: vec![], trashed: vec![], replacements: vec![], storage:Default::default() }
    }

    pub fn path(&self) -> PathBuf {
        dir(&self.root).join(format!("journal-{}.json", self.id))
    }

    /// 追加已完成的操作。文本内容不入日志(回滚只需要路径),避免日志膨胀。
    pub fn push_ops(&mut self, done: impl IntoIterator<Item = Op>) {
        self.ops.extend(done.into_iter().map(|op| match op {
            Op::WriteFile { path, .. } => Op::WriteFile { path: std::path::absolute(&path).unwrap_or(path), content: String::new() },
            Op::MoveFile { from, to } => Op::MoveFile { from: std::path::absolute(&from).unwrap_or(from), to: std::path::absolute(&to).unwrap_or(to) },
            Op::CopyFile { from,to,expected } => Op::CopyFile {expected,from:std::path::absolute(&from).unwrap_or(from),to:std::path::absolute(&to).unwrap_or(to)},
            Op::HardlinkFile { from,to,expected } => Op::HardlinkFile {expected,from:std::path::absolute(&from).unwrap_or(from),to:std::path::absolute(&to).unwrap_or(to)},
            Op::SymlinkFile { from,to,expected } => Op::SymlinkFile {expected,from:std::path::absolute(&from).unwrap_or(from),to:std::path::absolute(&to).unwrap_or(to)},
            Op::Download { path, url, crop, stamp } => Op::Download { path: std::path::absolute(&path).unwrap_or(path), url, crop, stamp },
        }));
    }

    pub fn save(&self) -> io::Result<PathBuf> {
        self.check_storage()?;
        let p = self.path();
        self.prepare_storage_directory()?;
        let tmp = p.with_extension("json.tmp");
        use std::io::Write;
        crate::append::confined_path(&self.root,&p)?;
        let root=same_file::Handle::from_path(&self.root)?;
        let (file,owner)=storage::open_temporary(self,&root)?;
        // Serialize directly to the staged file: JSON grows with the complete
        // history, but saving it needs only this fixed buffer, not another Vec.
        let mut file = io::BufWriter::with_capacity(64 * 1024, file);
        serde_json::to_writer_pretty(&mut file, self).map_err(io::Error::other)?;
        file.flush()?;
        let file = file.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        #[cfg(test)] storage::metrics::record(file.metadata()?.len());
        drop(file);
        self.check_storage()?;
        storage::verify_temporary(self,&root,&owner)?;
        fs::rename(&tmp, &p)?;
        #[cfg(unix)]
        fs::File::open(p.parent().expect("journal dir"))?.sync_all()?;
        self.mark_snapshot()?;
        Ok(p)
    }

    /// 读取日志;兼容旧的裸 `Vec<Op>` 格式(此时 root 取日志所在库目录)。
    pub fn load(path: &Path) -> io::Result<Self> {Self::load_checked(path,&||false)}
    pub fn load_cancellable(path:&Path,cancel:&tokio_util::sync::CancellationToken)->io::Result<Self>{Self::load_checked(path,&||cancel.is_cancelled())}
    pub(crate) fn load_checked(path:&Path,cancel:&dyn Fn()->bool)->io::Result<Self>{
        let j=storage::read_checked(path,cancel)?;
        if ![1,2,3,4].contains(&j.version) { return Err(io::Error::other(format!("不支持的操作日志版本: {}",j.version))); }
        let owner = path.parent().and_then(Path::parent).ok_or_else(|| io::Error::other("日志不在媒体库目录中"))?;
        if j.root.canonicalize()? != owner.canonicalize()? { return Err(io::Error::other("日志的媒体库根目录不匹配,已拒绝恢复")); }
        if path.file_name() != j.path().file_name() { return Err(io::Error::other("日志编号与文件名不匹配，已拒绝恢复")); }
        if j.version >= 3 {
            if j.id.is_empty() || !j.id.bytes().all(|b|b.is_ascii_digit()) { return Err(io::Error::other("修订日志编号不匹配")); }
            j.validate_replacements_checked(cancel)?;
        }
        crate::append::check_stopped(cancel)?;j.verify_read_identity()?;
        Ok(j)
    }

    /// 列出库内全部日志,新的在前;读不了的文件跳过。
    pub fn list(root: &Path) -> Vec<Journal> {
        let mut out: Vec<Journal> = fs::read_dir(dir(root))
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("journal-") && e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| Journal::load(&e.path()).ok())
            .collect();
        out.sort_by(|a, b| b.ts.cmp(&a.ts));
        out
    }

    /// 逐操作回滚并保存进度;失败项可重试,日志不可写时立即停止。
    /// 文件操作与进度落盘之间仍有崩溃窗口,不会仅凭文件存在推断已撤销。
    pub fn rollback(&mut self) -> Vec<String> {
        if let Err(error)=self.check_storage(){return vec![error.to_string()]}
        if let Err(error) = self.protect_original_references() { return vec![error.to_string()]; }
        if !self.replacements.is_empty() {
            let errors=self.rollback_replacements();if !errors.is_empty() || self.ops.is_empty(){return errors;}
            self.status=JournalStatus::Partial;
        }
        if self.status == JournalStatus::RolledBack {
            return vec![];
        }
        if let Err(e) = self.validate_recorded_targets() { return vec![e.to_string()]; }
        self.status = JournalStatus::Partial;
        if let Err(e) = self.recover_pending() { return vec![e.to_string()]; }
        if let Err(e) = self.persist_progress() {
            return vec![format!("保存撤销日志失败：{e}")];
        }
        let mut failures = vec![];
        // Only newly introduced original assets change order; all other operations retain
        // their existing reverse order. A failed sidecar operation still blocks asset deletion.
        let mut order = (0..self.ops.len()).rev().collect::<Vec<_>>();
        order.sort_by_key(|&i| matches!(&self.ops[i], Op::Download { path, .. } if Self::is_original_asset(path)));
        for i in order {
            if self.rolled_back_ops.contains(&i) {
                continue;
            }
            let errors = self.undo_recorded(i);
            if errors.is_empty() {
                self.rolled_back_ops.push(i);
                if let Err(e) = self.persist_progress() {
                    failures.push(format!("保存撤销日志失败：{e}"));
                    return failures;
                }
                if let Some(proof) = self.proofs.get(&i) {
                    // 成功进度持久化后再清暂存链接,也清理其空目录。
                    let _ = crate::organize::rollback(&[Op::Download { path: proof.stage.clone(), url: String::new(), crop: None, stamp: None }], Some(&self.root));
                }
            } else {
                failures.extend(errors);
            }
        }
        if failures.is_empty() {
            self.status = JournalStatus::RolledBack;
            if let Err(e) = self.save() {
                self.status = JournalStatus::Partial;
                failures.push(format!("保存撤销日志失败：{e}"));
            }
        }
        failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rapid_batches_have_distinct_journal_ids() {
        let ids: std::collections::HashSet<_> = (0..100).map(|_| Journal::new(Path::new("library")).id).collect();
        assert_eq!(ids.len(), 100);
    }

    #[test]
    fn process_exit_at_each_checkpoint_leaves_a_recoverable_log() {
        if let Ok(root) = std::env::var("KIROKU_TEST_CRASH_ROOT") {
            let root = PathBuf::from(root);
            let point: usize = std::env::var("KIROKU_TEST_CRASH_POINT").unwrap().parse().unwrap();
            let op_index: usize = std::env::var("KIROKU_TEST_CRASH_OP").unwrap().parse().unwrap();
            let mut seen = 0;
            let points = [Checkpoint::Intent, Checkpoint::Staged, Checkpoint::Published, Checkpoint::SourceRemoved, Checkpoint::Committed];
            let from = root.join("source.mp4");
            fs::write(&from, b"video").unwrap();
            let mut j = Journal::new(&root.join("library"));
            let mut png = std::io::Cursor::new(vec![]);
            image::RgbImage::new(1,1).write_to(&mut png, image::ImageFormat::Png).unwrap();
            j.execute_observed(&crate::organize::Plan { ops: vec![
                Op::MoveFile { from, to: root.join("library/A/movie.mp4") },
                Op::WriteFile { path: root.join("library/A/movie.nfo"), content: "metadata".into() },
                Op::Download { path: root.join("library/A/poster.png"), url: "fixture".into(), crop: None, stamp: None },
            ] }, &mut |_| Ok(png.get_ref().clone()), &|| false, &mut |p| {
                if p == points[point] { seen += 1; if seen == op_index { std::process::exit(86); } }
                Ok(())
            });
            panic!("fault checkpoint was not reached");
        }
        for (point, op_index) in (0..5).flat_map(|point| (1..=3).map(move |index| (point,index))) {
            let d = tempfile::tempdir().unwrap();
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "journal::tests::process_exit_at_each_checkpoint_leaves_a_recoverable_log"])
                .env("KIROKU_TEST_CRASH_ROOT",d.path()).env("KIROKU_TEST_CRASH_POINT",point.to_string()).env("KIROKU_TEST_CRASH_OP",op_index.to_string()).output().unwrap();
            assert_eq!(output.status.code(), Some(86));
            let mut j = Journal::list(&d.path().join("library")).remove(0);
            // 提交随下一个意图帧落盘:在提交点退出时,该操作在日志里仍是待恢复,由暂存证明补记
            if point == 4 { assert!(j.pending.is_some(), "op {op_index} commit is deferred"); }
            assert!(j.rollback().is_empty(), "checkpoint {point}");
            assert_eq!(fs::read(d.path().join("source.mp4")).unwrap(), b"video");
            assert!(!d.path().join("library/A/movie.mp4").exists());
            assert!(!d.path().join("library/A/movie.nfo").exists());
            assert!(!d.path().join("library/A/poster.png").exists());
        }
    }

    #[test]
    fn one_movie_syncs_the_log_once_per_operation_plus_once_at_the_end() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("library");
        let mut j = Journal::new(&root);
        let mut png = std::io::Cursor::new(vec![]);
        image::RgbImage::new(1,1).write_to(&mut png, image::ImageFormat::Png).unwrap();
        for movie in ["A", "B"] {
            let from = d.path().join(format!("{movie}.mp4"));
            fs::write(&from, b"video").unwrap();
            super::storage::metrics::take_persists();
            let ex = j.execute(&crate::organize::Plan { ops: vec![
                Op::MoveFile { from, to: root.join(format!("{movie}/movie.mp4")) },
                Op::WriteFile { path: root.join(format!("{movie}/movie.nfo")), content: "metadata".into() },
                Op::Download { path: root.join(format!("{movie}/poster.png")), url: "fixture".into(), crop: None, stamp: None },
            ] }, &mut |_| Ok(png.get_ref().clone()), &|| false);
            assert!(ex.error.is_none(), "{:?}", ex.error);
            // 原来 7 次:项目开始 1 + 每个操作意图、提交各 1;现在 3 个意图 + 结束 1
            assert_eq!(super::storage::metrics::take_persists(), 4, "{movie}");
            let saved = Journal::load(&j.path()).unwrap();
            assert!(saved.pending.is_none() && saved.ops.len() == j.ops.len(), "结束时全部提交已落盘");
        }
        assert!(j.rollback().is_empty());
        assert!(d.path().join("A.mp4").exists() && d.path().join("B.mp4").exists());
    }

    #[test]
    fn every_forward_checkpoint_can_be_reloaded_and_rolled_back() {
        for point in [Checkpoint::Intent, Checkpoint::Staged, Checkpoint::Published, Checkpoint::SourceRemoved, Checkpoint::Committed] {
            let d = tempfile::tempdir().unwrap();
            let root = d.path().join("library");
            let from = d.path().join("source.mp4");
            let to = root.join("A/movie.mp4");
            fs::write(&from, b"original video").unwrap();
            let mut j = Journal::new(&root);
            let plan = crate::organize::Plan { ops: vec![Op::MoveFile { from: from.clone(), to: to.clone() }] };
            let ex = j.execute_observed(&plan, &mut |_| unreachable!(), &|| false, &mut |p| {
                if p == point { Err(io::Error::other("simulated process interruption")) } else { Ok(()) }
            });
            assert!(ex.error.is_some());
            let mut reloaded = Journal::load(&j.path()).unwrap();
            assert!(reloaded.rollback().is_empty(), "recovery at {point:?}");
            assert_eq!(fs::read(&from).unwrap(), b"original video");
            assert!(!to.exists());
            assert_eq!(Journal::load(&j.path()).unwrap().status, JournalStatus::RolledBack);
        }
    }

    #[test]
    fn recovery_preserves_a_target_replaced_by_the_user() {
        let d = tempfile::tempdir().unwrap();
        let from = d.path().join("source.mp4");
        let to = d.path().join("library/movie.mp4");
        fs::write(&from, b"original").unwrap();
        let mut j = Journal::new(&d.path().join("library"));
        let plan = crate::organize::Plan { ops: vec![Op::MoveFile { from: from.clone(), to: to.clone() }] };
        j.execute_observed(&plan, &mut |_| unreachable!(), &|| false, &mut |p| {
            if p == Checkpoint::Published { Err(io::Error::other("interrupted")) } else { Ok(()) }
        });
        fs::remove_file(&to).unwrap();
        fs::write(&to, b"new user file").unwrap();
        let mut reloaded = Journal::load(&j.path()).unwrap();
        assert!(!reloaded.rollback().is_empty());
        assert_eq!(fs::read(&from).unwrap(), b"original");
        assert_eq!(fs::read(&to).unwrap(), b"new user file");
    }

    #[test]
    fn failed_commit_save_stops_subsequent_operations_and_remains_recoverable() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("library");
        let from = d.path().join("source.mp4");
        let to = root.join("A/movie.mp4");
        fs::write(&from, b"video").unwrap();
        let mut j = Journal::new(&root);
        let path=j.path();let blocked=path.with_extension("blocked");
        let plan = crate::organize::Plan { ops: vec![
            Op::MoveFile { from: from.clone(), to: to.clone() },
            Op::WriteFile { path: root.join("A/never.nfo"), content: "x".into() },
        ] };
        let result = j.execute_observed(&plan, &mut |_| unreachable!(), &|| false, &mut |p| {
            if p == Checkpoint::Published {fs::rename(&path,&blocked)?;fs::create_dir(&path)?;}
            Ok(())
        });
        assert!(result.error.is_some());
        assert!(!root.join("A/never.nfo").exists());
        fs::remove_dir(&path).unwrap();fs::rename(&blocked,&path).unwrap();
        let mut reloaded = Journal::load(&j.path()).unwrap();
        assert!(reloaded.pending.is_some());
        assert!(reloaded.rollback().is_empty());
        assert_eq!(fs::read(&from).unwrap(), b"video");
        assert!(!to.exists());
    }

    #[test]
    fn interrupted_partial_copy_is_kept_for_manual_inspection() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("library");
        let from = d.path().join("source.mp4");
        fs::write(&from, b"complete video").unwrap();
        let mut j = Journal::new(&root);
        let stage = root.join(format!(".kiroku-{}-0.tmp", j.id));
        let plan = crate::organize::Plan { ops: vec![Op::MoveFile { from: from.clone(), to: root.join("movie.mp4") }] };
        j.execute_observed(&plan, &mut |_| unreachable!(), &|| false, &mut |point| {
            if point == Checkpoint::Intent { fs::write(&stage, b"partial")?; return Err(io::Error::other("copy interrupted")); }
            Ok(())
        });
        let mut reloaded = Journal::load(&j.path()).unwrap();
        assert!(!reloaded.rollback().is_empty());
        assert_eq!(fs::read(&from).unwrap(), b"complete video");
        assert_eq!(fs::read(stage).unwrap(), b"partial");
    }

    #[test]
    fn damaged_journal_is_retained_in_inspection_results() {
        let d = tempfile::tempdir().unwrap();
        Journal::new(d.path()).save().unwrap();
        fs::write(dir(d.path()).join("journal-123.json"), b"broken JSON").unwrap();
        let entries = Journal::inspect(d.path()).unwrap();
        assert_eq!(entries.len(), 2);
        let damaged = entries.iter().find(|e| e.id == "123").unwrap();
        assert!(damaged.journal.is_none() && damaged.error.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn organize_rejects_a_library_subdirectory_symlink_to_outside() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("library");
        let outside = d.path().join("outside");
        fs::create_dir_all(&root).unwrap(); fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("A")).unwrap();
        let from = d.path().join("source.mp4"); fs::write(&from, b"video").unwrap();
        let mut j = Journal::new(&root);
        let ex = j.execute(&crate::organize::Plan { ops: vec![Op::MoveFile { from: from.clone(), to: root.join("A/movie.mp4") }] }, &mut |_| unreachable!(), &|| false);
        assert!(ex.error.is_some() && ex.done.is_empty());
        assert_eq!(fs::read(from).unwrap(), b"video");
        assert!(!outside.join("movie.mp4").exists());
    }

    #[test]
    fn load_rejects_a_journal_that_claims_another_library_root() {
        let d = tempfile::tempdir().unwrap();
        let actual = d.path().join("library");
        let other = d.path().join("other");
        fs::create_dir_all(dir(&actual)).unwrap(); fs::create_dir_all(&other).unwrap();
        let j = Journal::new(&other);
        let path = dir(&actual).join(format!("journal-{}.json",j.id));
        fs::write(&path, serde_json::to_vec(&j).unwrap()).unwrap();
        assert!(Journal::load(&path).is_err());
    }

    #[test]
    fn load_rejects_unknown_journal_versions() {
        let d = tempfile::tempdir().unwrap();
        let j = Journal::new(d.path());
        let path = j.save().unwrap();
        let mut value = serde_json::to_value(j).unwrap();
        value["version"] = 99.into();
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(Journal::load(&path).is_err());
    }

    #[test]
    fn partial_rollback_resumes_after_reloading_journal() {
        let d = tempfile::tempdir().unwrap();
        let (source, library) = (d.path().join("source"), d.path().join("library"));
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&library).unwrap();
        let mut j = Journal::new(&library);
        for name in ["A.mp4", "B.mp4"] {
            let to = library.join(name);
            fs::write(&to, name.as_bytes()).unwrap();
            j.push_ops([Op::MoveFile { from: source.join(name), to }]);
        }
        let original_ops = j.ops.clone();
        let path = j.save().unwrap();
        fs::write(source.join("A.mp4"), b"new user file").unwrap();

        assert_eq!(j.rollback().len(), 1);
        assert_eq!(fs::read(source.join("B.mp4")).unwrap(), b"B.mp4");
        assert_eq!(fs::read(source.join("A.mp4")).unwrap(), b"new user file");
        assert_eq!(fs::read(library.join("A.mp4")).unwrap(), b"A.mp4");

        let mut reloaded = Journal::load(&path).unwrap();
        assert_eq!(reloaded.status, JournalStatus::Partial);
        assert_eq!(reloaded.rolled_back_ops, vec![1], "已完成进度随日志一起恢复");
        fs::remove_file(source.join("A.mp4")).unwrap();
        assert!(reloaded.rollback().is_empty(), "只重试尚未撤销的 A,不再次恢复已经完成的 B");
        assert_eq!(reloaded.status, JournalStatus::RolledBack);
        assert_eq!(reloaded.ops, original_ops, "原操作历史必须完整保留");
        assert_eq!(fs::read(source.join("A.mp4")).unwrap(), b"A.mp4");
        assert_eq!(fs::read(source.join("B.mp4")).unwrap(), b"B.mp4");
        assert_eq!(Journal::load(&path).unwrap().status, JournalStatus::RolledBack);
    }

    #[test]
    fn completed_legacy_rollback_does_not_touch_later_user_files() {
        let d = tempfile::tempdir().unwrap();
        let (from, to, nfo) = (d.path().join("source.mp4"), d.path().join("library.mp4"), d.path().join("movie.nfo"));
        fs::write(&from, b"new source").unwrap();
        fs::write(&to, b"new library video").unwrap();
        fs::write(&nfo, b"new metadata").unwrap();
        let mut j = Journal::new(d.path());
        j.push_ops([
            Op::MoveFile { from: from.clone(), to: to.clone() },
            Op::WriteFile { path: nfo.clone(), content: String::new() },
        ]);
        j.status = JournalStatus::RolledBack;
        let path = j.save().unwrap();
        let mut legacy = serde_json::to_value(&j).unwrap();
        legacy.as_object_mut().unwrap().remove("rolledBackOps");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        let mut reloaded = Journal::load(&path).unwrap();
        assert!(reloaded.rolled_back_ops.is_empty(), "旧对象格式没有逐操作进度");
        assert!(reloaded.rollback().is_empty());
        assert_eq!(reloaded.status, JournalStatus::RolledBack);
        assert_eq!(fs::read(&from).unwrap(), b"new source");
        assert_eq!(fs::read(&to).unwrap(), b"new library video");
        assert_eq!(fs::read(&nfo).unwrap(), b"new metadata");
    }

    #[test]
    fn rollback_stops_before_file_changes_when_journal_cannot_be_saved() {
        let d = tempfile::tempdir().unwrap();
        let (from, to, nfo) = (d.path().join("source.mp4"), d.path().join("library.mp4"), d.path().join("movie.nfo"));
        fs::write(&to, b"video").unwrap();
        fs::write(&nfo, b"metadata").unwrap();
        let mut j = Journal::new(d.path());
        j.push_ops([
            Op::MoveFile { from: from.clone(), to: to.clone() },
            Op::WriteFile { path: nfo.clone(), content: String::new() },
        ]);
        let path = j.save().unwrap();
        // 确定性地阻止临时日志写入,不依赖当前用户的文件权限。
        fs::create_dir(path.with_extension("json.tmp")).unwrap();

        let failures = j.rollback();
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("保存撤销日志失败"));
        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), b"video");
        assert_eq!(fs::read(&nfo).unwrap(), b"metadata", "写日志失败之前不得删除生成文件");
        assert!(Journal::load(&path).unwrap().rolled_back_ops.is_empty());
    }

    #[test]
    fn save_load_list_and_legacy_format() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::new(d.path());
        j.push_ops([Op::WriteFile { path: d.path().join("x/movie.nfo"), content: "big".into() }]);
        let p = j.save().unwrap();
        let back = Journal::load(&p).unwrap();
        assert_eq!(back, j);
        assert_eq!(back.ops, vec![Op::WriteFile { path: d.path().join("x/movie.nfo"), content: String::new() }], "内容不入日志");
        assert_eq!(Journal::list(d.path()).len(), 1);

        // CLI 旧格式:裸 Vec<Op>
        let legacy = d.path().join(".kiroku/journal-123.json");
        fs::write(&legacy, serde_json::to_vec(&vec![Op::WriteFile { path: "a".into(), content: String::new() }]).unwrap()).unwrap();
        let l = Journal::load(&legacy).unwrap();
        assert_eq!((l.id.as_str(), l.ts, l.ops.len()), ("123", 123, 1));
        assert_eq!(l.root, d.path());
        assert!(l.rolled_back_ops.is_empty());
    }
}

#[cfg(test)]
mod original_recovery_tests {
    use super::*;
    #[test]
    fn failed_import_sidecar_undo_blocks_original_at_actual_deletion_boundary() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let side = root.join(crate::organize::SIDECAR);
        let original = root.join(".kiroku-original-fixture.png");
        let content =
            serde_json::json!({"artwork":{"original":".kiroku-original-fixture.png"}}).to_string();
        let mut j = Journal::new(&root);
        let plan = crate::organize::Plan {
            ops: vec![
                Op::WriteFile {
                    path: side.clone(),
                    content: content.clone(),
                },
                Op::Download {
                    path: original.clone(),
                    url: "fixture".into(),
                    crop: None,
                    stamp: None,
                },
            ],
        };
        let mut bytes = std::io::Cursor::new(vec![]);
        image::DynamicImage::new_rgb8(4, 6)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        let executed = j.execute(&plan, &mut |_| Ok(bytes.clone()), &|| false);
        assert!(executed.error.is_none());
        assert_eq!(j.ops.len(), 2);
        // Model an actual sidecar undo failure after preflight: its proof becomes unusable.
        // Replacing the proof (not modifying its hard-linked bytes) leaves the live sidecar valid.
        j.protect_original_references().unwrap();
        let proof = j.proofs[&0].stage.clone();
        fs::remove_file(&proof).unwrap();
        fs::write(&proof, b"invalid proof").unwrap();
        assert!(!j.undo_recorded(0).is_empty());
        assert!(
            !j.undo_recorded(1).is_empty(),
            "asset deletion must check the live reference even after preflight"
        );
        assert_eq!(fs::read_to_string(&side).unwrap(), content);
        assert!(original.exists());
        fs::remove_file(&proof).unwrap();
        fs::hard_link(&side, &proof).unwrap();
        assert!(j.rollback().is_empty());
        assert!(!side.exists());
        assert!(!original.exists());
    }
}
