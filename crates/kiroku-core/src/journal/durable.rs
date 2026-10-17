//! 操作意图先持久化,目标盘暂存文件作为提交/恢复依据。
use super::{Journal, JournalStatus, Pending, Proof};
use crate::organize::{self, Exec, Op, Plan};
use sha2::{Digest, Sha256};
use std::{fs, io::{self, Read, Write}, path::Path};

fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn file_digest_checked(path: &Path,cancel:&dyn Fn()->bool) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop { crate::append::check_stopped(cancel)?; let n = file.read(&mut buf)?; if n == 0 { break; }
        #[cfg(test)] crate::append::hash_reads::record(n);
        hash.update(&buf[..n]); }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod performance_tests {
    use super::*;

    #[test]
    fn media_commit_reads_content_only_three_times_and_still_rolls_back() {
        for mode in [crate::placement::OrganizeMode::Move, crate::placement::OrganizeMode::Copy,
            crate::placement::OrganizeMode::Hardlink, crate::placement::OrganizeMode::Symlink] {
            let d = tempfile::tempdir().unwrap();
            let from = d.path().join("source.mp4");
            let root = d.path().join("library");
            let to = root.join("movie.mp4");
            let bytes = vec![42; 256 * 1024];
            fs::write(&from, &bytes).unwrap();
            let plan = Plan { ops: vec![mode.operation(from.clone(), to.clone())] };
            crate::append::hash_reads::take();
            let mut journal = Journal::new(&root);
            let result = journal.execute(&plan, &mut |_| unreachable!(), &|| false);
            assert!(result.error.is_none(), "{mode:?}: {:?}", result.error);
            let reads = crate::append::hash_reads::take();
            assert!(reads <= 3 * bytes.len() as u64, "{mode:?}: read {reads} bytes");
            assert_eq!(fs::read(&to).unwrap(), bytes);
            let mut reloaded = Journal::load(&journal.path()).unwrap();
            assert!(reloaded.rollback().is_empty(), "{mode:?}");
            assert_eq!(fs::read(&from).unwrap(), bytes);
            assert!(!to.exists());
        }
    }

    #[test]
    fn copied_media_hashes_source_during_copy_without_a_second_source_pass() {
        let t=tempfile::tempdir().unwrap();
        let root=t.path().join("library");
        let from=t.path().join("video.mp4");
        let bytes=vec![31;4*1024*1024];
        fs::write(&from,&bytes).unwrap();
        let plan=Plan{ops:vec![crate::placement::OrganizeMode::Copy.operation(from.clone(),root.join("video.mp4"))]};
        crate::append::hash_reads::take();
        let mut journal=Journal::new(&root);
        assert!(journal.execute(&plan,&mut |_|unreachable!(),&||false).error.is_none());
        // Initial source proof + streaming copy proof + independent stage read.
        assert_eq!(crate::append::hash_reads::take(),3*bytes.len() as u64);
        assert_eq!(fs::read(root.join("video.mp4")).unwrap(),bytes);
        assert!(journal.rollback().is_empty());
        assert_eq!(fs::read(from).unwrap(),bytes);
    }

    #[test]
    fn cancel_during_source_read_keeps_source_and_never_publishes() {
        let t=tempfile::tempdir().unwrap();
        let root=t.path().join("library");
        let from=t.path().join("video.mp4");
        fs::write(&from,vec![31;4*1024*1024]).unwrap();
        let reads=std::cell::Cell::new(0);
        let cancel=|| {let n=reads.get();reads.set(n+1);n>=8};
        let mut journal=Journal::new(&root);
        let out=journal.execute(&Plan{ops:vec![Op::MoveFile{from:from.clone(),to:root.join("video.mp4")}]},&mut |_|unreachable!(),&cancel);
        assert_eq!(out.error.unwrap().kind(),io::ErrorKind::Interrupted);
        assert!(from.exists());assert!(!root.join("video.mp4").exists());
    }

    #[test]
    fn copy_cancel_after_intent_removes_only_its_partial_stage() {
        let t=tempfile::tempdir().unwrap();let root=t.path().join("lib");
        let from=t.path().join("video.mp4");fs::write(&from,vec![8;4*1024*1024]).unwrap();
        let plan=Plan{ops:vec![crate::placement::OrganizeMode::Copy.operation(from.clone(),root.join("video.mp4"))]};
        let copying=std::cell::Cell::new(false);let calls=std::cell::Cell::new(0);
        let cancel=|| {if !copying.get(){return false}let n=calls.get();calls.set(n+1);n>=8};
        let mut j=Journal::new(&root);
        let out=j.execute_observed(&plan,&mut |_|unreachable!(),&cancel,&mut |point|{if point==Checkpoint::Intent{copying.set(true)}Ok(())});
        assert_eq!(out.error.unwrap().kind(),io::ErrorKind::Interrupted);
        assert!(!root.join("video.mp4").exists());assert!(from.exists());
        assert!(!j.pending.as_ref().unwrap().proof.stage.exists());
        let mut saved=Journal::load(&j.path()).unwrap();saved.recover_pending().unwrap();
        assert!(saved.pending.is_none());assert!(from.exists());
    }

    #[test]
    fn retained_source_replacement_after_intent_is_never_authorized() {
        for mode in [crate::placement::OrganizeMode::Copy,crate::placement::OrganizeMode::Hardlink,crate::placement::OrganizeMode::Symlink] {
            let t=tempfile::tempdir().unwrap();let root=t.path().join("lib");let from=t.path().join("video.mp4");
            fs::write(&from,b"video").unwrap();
            let plan=Plan{ops:vec![mode.operation(from.clone(),root.join("video.mp4"))]};
            let mut j=Journal::new(&root);
            let out=j.execute_observed(&plan,&mut |_|unreachable!(),&||false,&mut |point|{
                if point==Checkpoint::Intent {fs::rename(&from,t.path().join("original.mp4"))?;fs::write(&from,b"video")?;}Ok(())
            });
            assert!(out.error.is_some(),"{mode:?}");assert!(!root.join("video.mp4").exists());assert_eq!(fs::read(from).unwrap(),b"video");
        }
    }

    #[test]
    #[ignore = "large media logical disk traffic measurement"]
    fn large_media_disk_traffic() {
        use std::io::Write;
        for mib in [64u64,256] {
            for (mode,passes) in [(crate::placement::OrganizeMode::Move,3),(crate::placement::OrganizeMode::Copy,5),(crate::placement::OrganizeMode::Hardlink,4),(crate::placement::OrganizeMode::Symlink,4)] {
                let t=tempfile::tempdir().unwrap();let from=t.path().join("ABC-123.mp4");
                let mut f=fs::File::create(&from).unwrap();for _ in 0..mib*16 {f.write_all(&[11;65536]).unwrap();}f.sync_all().unwrap();drop(f);
                let code=crate::code::parse_code("ABC-123.mp4").unwrap();let root=t.path().join("lib");
                crate::append::hash_reads::take();crate::append::hash_reads::take_writes();
                let mut plan=crate::organize::plan_with(&from,&code,&crate::Metadata{code:code.id.clone(),..Default::default()},&root,"{code}",true,crate::organize::PlanOpts{mode,..Default::default()});
                let mut j=Journal::new(&root);crate::placement::bind_plan(&mut plan,&j).unwrap();
                assert!(crate::placement::execute(&mut j,&plan,&mut |_|unreachable!(),&||false).error.is_none());
                let read=crate::append::hash_reads::take();let expected=mib*1024*1024*passes;
                // Small NFO/sidecar stage checks add metadata bytes to the media traffic.
                assert!(read>=expected && read<expected+65536,"{mode:?}: {read} vs {expected}");
                let written=crate::append::hash_reads::take_writes();
                assert_eq!(written,if mode==crate::placement::OrganizeMode::Copy{mib*1024*1024}else{0});
                println!("disk mode={mode:?} size={mib}MiB read={read}B media-write={written}B");
                assert!(j.rollback().is_empty());
            }
        }
    }

    #[test]
    fn cross_volume_move_checks_copy_and_keeps_recoverable_boundaries() {
        struct Restore;impl Drop for Restore {fn drop(&mut self){FORCE_MOVE_COPY.set(false);}}
        FORCE_MOVE_COPY.set(true);let _reset=Restore;
        for stop in [None,Some(Checkpoint::Intent),Some(Checkpoint::Staged),Some(Checkpoint::Published),Some(Checkpoint::SourceRemoved)] {
            let t=tempfile::tempdir().unwrap();let from=t.path().join("video.mp4");let root=t.path().join("lib");
            let bytes=vec![17;8*1024*1024];fs::write(&from,&bytes).unwrap();
            let to=root.join("video.mp4");let mut j=Journal::new(&root);crate::append::hash_reads::take_writes();
            let out=j.execute_observed(&Plan{ops:vec![Op::MoveFile{from:from.clone(),to:to.clone()}]},&mut |_|unreachable!(),&||false,&mut |point|{
                if Some(point)==stop {Err(io::Error::other("simulated interruption"))}else{Ok(())}
            });
            assert_eq!(out.error.is_some(),stop.is_some());
            if stop.is_none(){assert_eq!(crate::append::hash_reads::take_writes(),bytes.len() as u64);}
            if matches!(stop,Some(Checkpoint::Intent|Checkpoint::Staged|Checkpoint::Published)){assert_eq!(fs::read(&from).unwrap(),bytes);}
            let mut saved=Journal::load(&j.path()).unwrap();assert!(saved.rollback().is_empty(),"{stop:?}");
            assert_eq!(fs::read(from).unwrap(),bytes);assert!(!to.exists());
        }
    }

    #[test]
    fn dropout_during_initial_source_hash_never_recreates_library() {
        let t=tempfile::tempdir().unwrap();let root=t.path().join("lib");fs::create_dir(&root).unwrap();
        let from=t.path().join("video.mp4");fs::write(&from,vec![7;4*1024*1024]).unwrap();
        let mut j=Journal::new(&root);let polls=std::cell::Cell::new(0);
        let cancel=||{let n=polls.get();polls.set(n+1);if n==8{fs::rename(&root,t.path().join("detached")).unwrap();}false};
        let out=j.execute(&Plan{ops:vec![Op::MoveFile{from:from.clone(),to:root.join("video.mp4")}]},&mut |_|unreachable!(),&cancel);
        assert!(out.error.is_some());assert!(from.exists());assert!(!root.exists());
    }

    #[test]
    fn move_preserves_source_modified_at_each_commit_boundary() {
        for at in [Checkpoint::Intent, Checkpoint::Staged, Checkpoint::Published] {
            let d = tempfile::tempdir().unwrap();
            let from = d.path().join("source.mp4");
            let root = d.path().join("library");
            fs::write(&from, b"video").unwrap();
            let mut journal = Journal::new(&root);
            let result = journal.execute_observed(&Plan { ops: vec![Op::MoveFile { from: from.clone(), to: root.join("movie.mp4") }] },
                &mut |_| unreachable!(), &|| false, &mut |point| {
                    if point == at { fs::write(&from, b"other")?; }
                    Ok(())
                });
            assert!(result.error.is_some(), "{at:?}");
            assert_eq!(fs::read(&from).unwrap(), b"other");
            assert!(Journal::load(&journal.path()).unwrap().pending.is_some());
        }
    }

    #[test]
    fn move_never_deletes_a_same_content_replacement_source() {
        let d = tempfile::tempdir().unwrap();
        let from = d.path().join("source.mp4");
        let root = d.path().join("library");
        let to = root.join("movie.mp4");
        fs::write(&from, b"video").unwrap();
        let mut journal = Journal::new(&root);
        let result = journal.execute_observed(&Plan { ops: vec![Op::MoveFile { from: from.clone(), to: to.clone() }] },
            &mut |_| unreachable!(), &|| false, &mut |point| {
                if point == Checkpoint::Published {
                    fs::remove_file(&from)?;
                    fs::write(&from, b"video")?;
                }
                Ok(())
            });
        assert!(result.error.is_some());
        assert_eq!(fs::read(&from).unwrap(), b"video");
        assert_eq!(fs::read(&to).unwrap(), b"video");
        assert!(Journal::load(&journal.path()).unwrap().pending.is_some());
    }
}
#[cfg(test)]
thread_local! {static FORCE_MOVE_COPY: std::cell::Cell<bool> = const {std::cell::Cell::new(false)};}
fn link_move_stage(from:&Path,stage:&Path)->io::Result<()> {
    #[cfg(test)] if FORCE_MOVE_COPY.get() {return Err(io::Error::new(io::ErrorKind::CrossesDevices,"test cross-volume boundary"))}
    fs::hard_link(from,stage)
}
fn verified(path: &Path, proof: &Proof) -> io::Result<bool> { verified_checked(path,proof,&||false) }
fn verified_checked(path: &Path, proof: &Proof,cancel:&dyn Fn()->bool) -> io::Result<bool> {
    crate::append::check_stopped(cancel)?;
    if let Some(source)=&proof.link {
        return Ok(fs::symlink_metadata(path)?.file_type().is_symlink() && fs::read_link(path)?==*source);
    }
    if !fs::symlink_metadata(path)?.file_type().is_file() { return Err(conflict(path)); }
    Ok(file_digest_checked(path,cancel)? == proof.digest)
}
fn within(root: &Path, target: &Path) -> io::Result<()> {
    crate::append::confined_path(root,target.parent().ok_or_else(||conflict(target))?)?;
    let root = root.canonicalize()?;
    let mut parent = target.parent().ok_or_else(|| conflict(target))?;
    while !organize::occupied(parent) { parent = parent.parent().ok_or_else(|| conflict(target))?; }
    if !parent.canonicalize()?.starts_with(root) { return Err(conflict(target)); }
    Ok(())
}
fn owned(target:&Path,proof:&Proof)->bool{if proof.link.is_some(){crate::placement::same_link(target,&proof.stage).unwrap_or(false)}else{same(target,&proof.stage)}}
fn same(a: &Path, b: &Path) -> bool { same_file::is_same_file(a,b).unwrap_or(false) }
fn conflict(path: &Path) -> io::Error { io::Error::other(format!("恢复需要核对文件,已保留内容: {}", path.display())) }
fn compact(op: &Op) -> Op {
    let abs = |path: &std::path::PathBuf| std::path::absolute(path).unwrap_or_else(|_| path.clone());
    match op {
        Op::WriteFile { path, .. } => Op::WriteFile { path: abs(path), content: String::new() },
        Op::MoveFile { from, to } => Op::MoveFile { from: abs(from), to: abs(to) },
        Op::CopyFile{from,to,expected}=>Op::CopyFile{from:abs(from),to:abs(to),expected:expected.clone()},
        Op::HardlinkFile{from,to,expected}=>Op::HardlinkFile{from:abs(from),to:abs(to),expected:expected.clone()},
        Op::SymlinkFile{from,to,expected}=>Op::SymlinkFile{from:abs(from),to:abs(to),expected:expected.clone()},
        Op::Download { path, url, crop, stamp } => Op::Download { path: abs(path), url: url.clone(), crop: *crop, stamp: stamp.clone() },
    }
}

/// 操作的持久边界,可用于进度汇报或中止执行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checkpoint { Intent, Staged, Published, SourceRemoved, Committed }

impl Journal {
    pub(crate) fn validate_recorded_targets(&self) -> io::Result<()> {
        for op in &self.ops { within(&self.root, op.target())?; }
        Ok(())
    }

    pub fn execute(&mut self, plan: &Plan, fetch: &mut dyn FnMut(&str) -> io::Result<Vec<u8>>, cancel: &dyn Fn() -> bool) -> Exec {
        self.execute_observed(plan, fetch, cancel, &mut |_| Ok(()))
    }

    pub fn execute_observed(&mut self, plan: &Plan, fetch: &mut dyn FnMut(&str) -> io::Result<Vec<u8>>, cancel: &dyn Fn() -> bool,
        checkpoint: &mut dyn FnMut(Checkpoint) -> io::Result<()>) -> Exec {
        self.execute_images_observed(plan, &mut |url| fetch(url).map(crate::artwork::ImageInput::Unchecked), cancel, checkpoint)
    }

    pub(crate) fn execute_images_observed(&mut self, plan: &Plan, fetch: &mut dyn FnMut(&str) -> io::Result<crate::artwork::ImageInput>, cancel: &dyn Fn() -> bool,
        checkpoint: &mut dyn FnMut(Checkpoint) -> io::Result<()>) -> Exec {
        let mut out = Exec::default();
        if cancel() { return out; }
        if let Err(e) = organize::preflight(&plan.ops) {
            log::error!(target: "kiroku::organize", "  目标已被占用,本项不执行: {e}");
            out.error = Some(e); return out;
        }
        self.status = JournalStatus::Partial;
        if self.pending.is_some() { out.error = Some(io::Error::other("批次有待恢复操作,请先检查历史")); return out; }
        // ADR-0006 修订:项目开始(状态、影片记录)和每个操作的提交都不单独同步,
        // 随下一个操作的意图帧原子写入;没有下一个意图时在返回前补写。
        let mut unsynced = true;
        'run: {
            // 原先由开始记录顺带创建库目录;库身份检查需要它已存在(只建目录,不同步)
            if let Err(e) = self.prepare_storage_directory() { out.error = Some(io::Error::other(format!("保存整理日志失败: {e}"))); break 'run; }
            for op in &plan.ops {
                if let Err(e) = within(&self.root, op.target()) { out.error = Some(e); break 'run; }
            }
            let identity=match same_file::Handle::from_path(&self.root){Ok(id)=>id,Err(e)=>{out.error=Some(e);break 'run;}};
            let root=self.root.clone();
            let verify_root=||->io::Result<()> {if same_file::Handle::from_path(&root)?!=identity{return Err(io::Error::other("库目录身份已变化，已停止整理"))}Ok(())};
            let mut checked_checkpoint=|point|->io::Result<()> {verify_root()?;checkpoint(point)?;verify_root()};
            for op in &plan.ops {
                if cancel() { break; }
                match self.commit_op(op, fetch, cancel, &verify_root, &mut checked_checkpoint, &mut unsynced) {
                    Ok(()) => {
                        log::info!(target: "kiroku::organize", "  {}", op.describe());
                        out.done.push(op.clone())
                    }
                    Err(e) if matches!(op, Op::Download { .. }) && self.pending.is_none() && e.kind() != io::ErrorKind::AlreadyExists => {
                        log::warn!(target: "kiroku::organize", "  跳过图片 {}: {e}", op.target().display());
                        out.warnings.push(format!("图片 {}: {e}",op.target().display()))
                    }
                    Err(e) => {
                        log::error!(target: "kiroku::organize", "  失败:{}: {e}", op.describe());
                        out.error = Some(e); break;
                    }
                }
            }
        }
        // 结束前把延后的提交落盘;调用方据此报告完成。日志还没写过说明没有任何媒体改动,
        // 不补写(否则会在库掉线后重建库目录)
        if unsynced && self.storage_initialized() {
            if let Err(e) = self.persist_progress() {
                log::error!(target: "kiroku::organize", "  保存整理日志失败: {e}");
                if out.error.is_none() { out.error = Some(io::Error::other(format!("保存整理日志失败: {e}"))); }
            }
        }
        out
    }

    /// `unsynced`:内存里有尚未落盘的变化(项目开始或上一个提交)。意图帧把它们一起写入;
    /// 本操作的提交只改内存并置位,由下一个意图帧或调用方结束时落盘。
    fn commit_op(&mut self, op: &Op, fetch: &mut dyn FnMut(&str) -> io::Result<crate::artwork::ImageInput>, cancel: &dyn Fn()->bool, verify_root: &dyn Fn()->io::Result<()>, checkpoint: &mut dyn FnMut(Checkpoint) -> io::Result<()>, unsynced: &mut bool) -> io::Result<()> {
        if let Op::SymlinkFile{from,to,..}=op {
            #[cfg(not(unix))] return Err(io::Error::other("此平台无法安全验证软链接所有权，请选择复制或硬链接"));
            if !crate::placement::valid_media(from)||!crate::placement::valid_media(to)||from.components().any(|c|c.as_os_str()==".kiroku") {return Err(io::Error::other("软链接只允许普通视频或字幕来源"))}
        }
        let fetched = match op { Op::Download { url, .. } => Some(fetch(url)?), _ => None };
        let bytes = match op {
            Op::MoveFile { .. } | Op::CopyFile { .. } | Op::HardlinkFile { .. } | Op::SymlinkFile { .. } => None,
            Op::WriteFile { content, .. } => Some(std::borrow::Cow::Borrowed(content.as_bytes())),
            Op::Download { crop, stamp, .. } => {
                let input = fetched.as_ref().unwrap();
                let b = input.bytes();
                Some(if input.crop_is_noop(*crop) && stamp.is_none() {
                    if !input.validated() {
                        #[cfg(test)] crate::artwork::decode_metrics::record();
                        image::load_from_memory(b).map_err(io::Error::other)?;
                    }
                    std::borrow::Cow::Borrowed(b)
                } else { organize::process_image_shared(b, *crop, stamp.as_ref())? })
            }
        };
        if let Op::MoveFile { from, .. } = op {
            // 源摘要复用为日志 digest；暂存与删源前仍分别完整复核。
            let size = fs::metadata(from).map(|m| m.len()).unwrap_or(0);
            if size >= 512 * 1024 * 1024 {
                log::info!(target: "kiroku::organize", "  校验 {}({:.1} GB),确认文件完整后再移动…", from.display(), size as f64 / 1e9);
            }
        }
        let source_proof=op.source().map(|path|crate::append::fingerprint_stream(path,cancel,&|_,_|{},None)).transpose()?;
        match op {Op::CopyFile{from,expected,..}|Op::HardlinkFile{from,expected,..}|Op::SymlinkFile{from,expected,..}=>{if expected.as_ref()!=source_proof.as_ref(){return Err(conflict(from))}},_=>{}}
        // fingerprint contains identity plus the SHA-256 of exactly this read.
        // Reuse only within this operation; never cache across files or jobs.
        let expected = match &source_proof {
            Some(proof) => proof.rsplit(':').next().expect("source fingerprint digest").to_owned(),
            None => digest(bytes.as_ref().unwrap()),
        };
        let target = op.target();
        let parent = target.parent().ok_or_else(|| io::Error::other("目标缺少父目录"))?;
        let proof = Proof { stage: std::path::absolute(parent.join(format!(".kiroku-{}-{}.tmp", self.id, self.ops.len())))?, digest: expected, link: if matches!(op,Op::SymlinkFile{..}){Some(op.source().unwrap().canonicalize()?)}else{None}, source_proof };
        if op.retained(){self.version=4;}
        self.pending = Some(Pending { op: compact(op), proof: proof.clone() });
        verify_root()?; self.persist_progress()?; *unsynced = false; checkpoint(Checkpoint::Intent)?;
        fs::create_dir_all(parent)?;
        let mut copied_proof=None;
        match op {
            Op::MoveFile { from, .. } => {
                if link_move_stage(from, &proof.stage).is_err() {
                    copied_proof=Some(organize::copy_proven(from,&proof.stage,cancel)?);
                }
            }
            Op::CopyFile { from,.. } => {copied_proof=Some(organize::copy_proven(from,&proof.stage,cancel)?);},
            Op::HardlinkFile { from,.. } => fs::hard_link(from,&proof.stage).map_err(|e|io::Error::new(e.kind(),format!("硬链接要求同卷，不能降级；可重新选择复制：{e}")))?,
            Op::SymlinkFile { .. } => crate::placement::symlink(proof.link.as_ref().unwrap(),&proof.stage)?,
            _ => {
                let mut stage = fs::OpenOptions::new().write(true).create_new(true).open(&proof.stage)?;
                stage.write_all(bytes.as_ref().unwrap())?; stage.sync_all()?;
            }
        }
        let linked_stage_verified = if let Op::HardlinkFile {from,..}=op {
            // One read verifies both names only after proving they own the same inode.
            let current=crate::append::fingerprint_stream(&proof.stage,cancel,&|_,_|{},None)?;
            if !same(from,&proof.stage) || proof.source_proof.as_ref()!=Some(&current) {return Err(conflict(from))}
            true
        } else {false};
        if let Some(copied)=copied_proof {
            if proof.source_proof.as_ref()!=Some(&copied) {return Err(conflict(op.source().unwrap()))}
        } else if op.retained() && !linked_stage_verified { let from=op.source().unwrap(); if proof.source_proof.as_ref()!=Some(&crate::append::fingerprint_stream(from,cancel,&|_,_|{},None)?){return Err(conflict(from))} }
        if !linked_stage_verified && !verified_checked(&proof.stage, &proof, cancel)? { return Err(conflict(&proof.stage)); }
        #[cfg(unix)]
        if proof.link.is_none(){fs::File::open(&proof.stage)?.sync_all()?;}
        #[cfg(not(unix))]
        if proof.link.is_none(){fs::OpenOptions::new().write(true).open(&proof.stage)?.sync_all()?;}
        checkpoint(Checkpoint::Staged)?;
        crate::append::check_stopped(cancel)?;
        // 同盘独占发布,暂存硬链接留作所有权证明;不支持安全提交时保留源与暂存并报错。
        fs::hard_link(&proof.stage, target)?;
        #[cfg(unix)] fs::File::open(parent)?.sync_all()?;
        checkpoint(Checkpoint::Published)?;
        crate::append::check_stopped(cancel)?;
        if let Op::MoveFile { from, .. } = op {
            // Check both inode identity and content at the deletion boundary.
            // A same-byte replacement belongs to the user and must be preserved.
            if proof.source_proof.as_ref()!=Some(&crate::append::fingerprint_stream(from,cancel,&|_,_|{},None)?) { return Err(conflict(from)); }
            fs::remove_file(from)?;
            #[cfg(unix)] if let Some(dir) = from.parent() { fs::File::open(dir)?.sync_all()?; }
        }
        checkpoint(Checkpoint::SourceRemoved)?;
        let index = self.ops.len();
        self.push_ops([op.clone()]); self.proofs.insert(index, proof); self.pending = None;
        // 崩溃时提交未落盘:日志里本操作仍是 pending,recover_pending 按暂存证明补记,与原来提交前崩溃相同
        *unsynced = true;
        verify_root()?; checkpoint(Checkpoint::Committed)
    }

    /// 重启后只依据暂存文件身份及摘要判断提交结果,不凭目标存在猜测归属。
    pub fn recover_pending(&mut self) -> io::Result<()> {
        self.check_storage()?;
        let Some(p) = self.pending.clone() else { return Ok(()) };
        let target = p.op.target();
        within(&self.root, target)?;
        within(&self.root, &p.proof.stage)?;
        if organize::occupied(&p.proof.stage) {
            if !verified(&p.proof.stage, &p.proof)? { return Err(conflict(&p.proof.stage)); }
            if organize::occupied(target) {
                if !owned(target, &p.proof) || !verified(target, &p.proof)? { return Err(conflict(target)); }
                let i = self.ops.len(); self.push_ops([p.op]); self.proofs.insert(i, p.proof); self.pending = None; self.persist_progress()?;
                return Ok(());
            }
            if let Op::MoveFile { from, .. } = &p.op {
                if !verified(from, &p.proof)? { return Err(conflict(from)); }
            }
            fs::remove_file(&p.proof.stage)?;
        } else {
            if organize::occupied(target) { return Err(conflict(target)); }
            if let Op::MoveFile { from, .. } = &p.op { if !verified(from, &p.proof)? { return Err(conflict(from)); } }
        }
        self.pending = None; self.persist_progress()?; Ok(())
    }

    pub(super) fn undo_recorded(&self, i: usize) -> Vec<String> {
        let op = &self.ops[i];
        let Some(proof) = self.proofs.get(&i) else {
            if matches!(op, Op::Download { .. }) {
                if let Err(error) = self.ensure_original_unreferenced(op.target()) { return vec![error.to_string()]; }
            }
            return organize::rollback(&self.ops[i..=i], Some(&self.root))
        };
        let undo = (|| -> io::Result<()> {
            within(&self.root, op.target())?;
            within(&self.root, &proof.stage)?;
            if !verified(&proof.stage, proof)? { return Err(conflict(&proof.stage)); }
            let target = op.target();
            if organize::occupied(target) {
                if !owned(target, proof) || !verified(target, proof)? { return Err(conflict(target)); }
                if let Op::MoveFile { from, .. } = op {
                    if organize::occupied(from) {
                        // 发布完成但尚未删源时中断:保留内容一致的原文件。
                        if !verified(from, proof)? { return Err(conflict(from)); }
                        fs::remove_file(target)?;
                        return Ok(());
                    }
                }
                if matches!(op, Op::Download { .. }) { self.ensure_original_unreferenced(target)?; }
                if op.retained(){
                    let side=target.parent().ok_or_else(||conflict(target))?.join(crate::organize::SIDECAR);
                    if organize::occupied(&side){
                        crate::append::confined_path(&self.root,&side)?;
                        let metadata=crate::append::read_meta(side.parent().unwrap())?;
                        if metadata.placements.iter().any(|p|p.anchor.as_ref()==Some(&proof.stage)){
                            return Err(io::Error::other("较新或未恢复的 sidecar 仍引用本次媒体，已保留目标"));
                        }
                    }
                    fs::remove_file(target)?;return Ok(())
                }
                let errors = organize::rollback(&self.ops[i..=i], Some(&self.root));
                if !errors.is_empty() { return Err(io::Error::other(errors.join("; "))); }
            } else if let Op::MoveFile { from, .. } = op {
                if !same(from, &proof.stage) || !verified(from, proof)? { return Err(conflict(from)); }
            }
            // 暂存文件在成功进度持久化之前保留,保证撤销自身也可重试。
            Ok(())
        })();
        undo.err().map(|e| vec![e.to_string()]).unwrap_or_default()
    }
}
