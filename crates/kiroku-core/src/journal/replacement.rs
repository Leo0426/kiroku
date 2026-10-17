//! Explicit replacement transaction. Backups are independent copies, never hard links.
use super::{Journal, JournalStatus};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Replacement {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: String,
    pub backup: Option<PathBuf>,
    pub stage: PathBuf,
    #[serde(default)]
    pub undone: bool,
    #[serde(default)]
    pub mode: Option<u32>,
    #[serde(default)]
    pub readonly: Option<bool>,
    /// Validated ownership in an older v2 journal; the independent backup remains mandatory.
    #[serde(default)]
    pub before_owner: Option<BeforeOwner>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeforeOwner {
    pub journal_id: String,
    pub op_index: usize,
    pub device: u64,
    pub inode: u64,
}
fn identity(path: &Path) -> io::Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::symlink_metadata(path)?;
        if !m.file_type().is_file() {
            return Err(conflict(path));
        }
        Ok((m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        Err(io::Error::other(format!(
            "文件身份锚在此平台不可用: {}",
            path.display()
        )))
    }
}

fn owner_stage(root: &Path, r: &Replacement, owner: &BeforeOwner) -> io::Result<PathBuf> {
    if owner.journal_id.is_empty() || !owner.journal_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(conflict(&r.path));
    }
    let path = super::dir(&root.canonicalize()?).join(format!("journal-{}.json", owner.journal_id));
    within(root, &path)?;
    let journal = Journal::load(&path)?;
    owner_stage_loaded(root,r,owner,&journal)
}
fn owner_stage_loaded(root:&Path,r:&Replacement,owner:&BeforeOwner,journal:&Journal)->io::Result<PathBuf>{
    if owner.journal_id.is_empty() || !owner.journal_id.bytes().all(|b|b.is_ascii_digit()){return Err(conflict(&r.path))}
    let path=super::dir(root).join(format!("journal-{}.json",owner.journal_id));
    if journal.id != owner.journal_id
        || !matches!(journal.version,2|4)
        || journal.status == JournalStatus::RolledBack
        || journal.rolled_back_ops.contains(&owner.op_index)
        || journal.root.canonicalize()? != root.canonicalize()?
    {
        return Err(conflict(&path));
    }
    let op = journal
        .ops
        .get(owner.op_index)
        .ok_or_else(|| conflict(&path))?;
    let proof = journal
        .proofs
        .get(&owner.op_index)
        .ok_or_else(|| conflict(&path))?;
    let expected_stage = r
        .path
        .parent()
        .ok_or_else(|| conflict(&r.path))?
        .join(format!(
            ".kiroku-{}-{}.tmp",
            owner.journal_id, owner.op_index
        ));
    let target = recorded_path(root, &journal.root, op.target())?;
    let stage = recorded_path(root, &journal.root, &proof.stage)?;
    if target != r.path || stage != expected_stage || Some(&proof.digest) != r.before.as_ref() {
        return Err(conflict(&path));
    }
    within(root, &stage)?;
    if identity(&stage)? != (owner.device, owner.inode) || current(&stage)? != r.before {
        return Err(conflict(&stage));
    }
    Ok(stage)
}

/// Normalize only the library root alias, not arbitrary descendant symlinks.
pub(crate) fn recorded_path(root: &Path, recorded_root: &Path, path: &Path) -> io::Result<PathBuf> {
    if recorded_root.canonicalize()? != root.canonicalize()? {
        return Err(conflict(path));
    }
    let canonical_root = root.canonicalize()?;
    let relative = path
        .strip_prefix(recorded_root)
        .or_else(|_| path.strip_prefix(&canonical_root))
        .map_err(|_| conflict(path))?;
    let normalized = canonical_root.join(relative);
    within(root, &normalized)?;
    Ok(normalized)
}

/// One disk pass per replacement transaction; candidates are never reused across
/// transactions or recovery boundaries. Only requested target records survive.
fn find_before_owners(root:&Path,requests:&[Replacement],cancel:&dyn Fn()->bool)->io::Result<Vec<Option<BeforeOwner>>>{
    let mut owners=vec![None;requests.len()];
    if !requests.iter().any(|r|r.before.is_some()){return Ok(owners)}
    #[cfg(not(unix))] {let _=(root,cancel);return Ok(owners)}
    #[cfg(unix)] {
        let guard=crate::pipeline::RootGuard::capture(root,false)?;
        let mut candidates:Vec<Option<(u64,BeforeOwner)>>=vec![None;requests.len()];
        let canonical=root.canonicalize()?;
        let targets:std::collections::HashMap<_,_>=requests.iter().enumerate().filter(|(_,r)|r.before.is_some()).filter_map(|(i,r)|r.path.strip_prefix(&canonical).ok().map(|path|(path.to_path_buf(),i))).collect();
        let entries=match fs::read_dir(super::dir(root)){Ok(entries)=>Some(entries),Err(e) if e.kind()==io::ErrorKind::NotFound=>None,Err(e)=>return Err(e)};
        for entry in entries.into_iter().flatten(){
            crate::append::check_stopped(cancel)?;guard.verify()?;let entry=entry?;
            let name=entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("journal-") || !name.ends_with(".json"){continue}
            let journal=match Journal::load_checked(&entry.path(),cancel){Ok(j)=>j,Err(e) if e.kind()==io::ErrorKind::Interrupted=>return Err(e),Err(_)=>continue};
            if !matches!(journal.version,2|4) || journal.status==JournalStatus::RolledBack{continue}
            let rolled:std::collections::HashSet<_>=journal.rolled_back_ops.iter().copied().collect();
            for (op_index,op) in journal.ops.iter().enumerate(){
                crate::append::check_stopped(cancel)?;
                if rolled.contains(&op_index){continue}
                // Prune by borrowed relative path before any per-operation disk I/O.
                // A matching path still goes through the original confinement checks.
                let Ok(relative)=op.target().strip_prefix(&journal.root).or_else(|_|op.target().strip_prefix(&canonical)) else{continue};
                let Some(&index)=targets.get(relative) else{continue};let r=&requests[index];
                if recorded_path(root,&journal.root,op.target()).ok().as_ref()!=Some(&r.path){continue}
                if candidates[index].as_ref().is_some_and(|(ts,_)|*ts>=journal.ts){continue}
                let Some(proof)=journal.proofs.get(&op_index).filter(|p|Some(&p.digest)==r.before.as_ref()) else{continue};
                if !same_file::is_same_file(&r.path,&proof.stage).unwrap_or(false){continue}
                let (device,inode)=identity(&r.path)?;
                candidates[index]=Some((journal.ts,BeforeOwner{journal_id:journal.id.clone(),op_index,device,inode}));
            }
        }
        // Authorize from fresh selected journals, one at a time, after discovery.
        // A full content recheck also catches changes while checking target anchors.
        let mut groups:std::collections::BTreeMap<String,Vec<usize>>=Default::default();
        for (i,candidate) in candidates.iter().enumerate(){if let Some((_,owner))=candidate{groups.entry(owner.journal_id.clone()).or_default().push(i);}}
        for (id,indices) in groups {
            crate::append::check_stopped(cancel)?;guard.verify()?;
            let journal=Journal::load_checked(&super::dir(root).join(format!("journal-{id}.json")),cancel)?;
            for i in indices {crate::append::check_stopped(cancel)?;let owner=&candidates[i].as_ref().unwrap().1;owner_stage_loaded(root,&requests[i],owner,&journal)?;owners[i]=Some(owner.clone());}
            journal.verify_read_checked(cancel)?;
        }
        guard.verify()?;crate::append::check_stopped(cancel)?;Ok(owners)
    }
}
#[derive(Debug, Clone)]
pub struct ReplacementInput {
    pub path: PathBuf,
    pub expected: Option<String>,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementCheckpoint {
    Intent,
    Backup(usize),
    Prepared,
    Replaced(usize),
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_file() => fs::read(path).map(Some),
        Ok(_) => Err(io::Error::other(format!(
            "不是普通文件: {}",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
pub(crate) fn current(path: &Path) -> io::Result<Option<String>> {
    Ok(read(path)?.map(|b| digest(&b)))
}
pub(crate) fn within(root: &Path, path: &Path) -> io::Result<()> {
    let root = root.canonicalize()?;
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        || !path.starts_with(&root)
    {
        return Err(io::Error::other("修订路径不在媒体库内"));
    }
    let mut p = path;
    while p != root {
        match fs::symlink_metadata(p) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(io::Error::other("修订路径不能包含符号链接"))
            }
            Ok(_) => (),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        p = p.parent().ok_or_else(|| io::Error::other("无效修订路径"))?;
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    sync_parent(path)
}
fn original_mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path)
            .ok()
            .map(|m| m.permissions().mode() & 0o7777)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}
fn restore_permissions(path: &Path, r: &Replacement) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(mode) = r.mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    if let Some(readonly) = r.readonly {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(readonly);
        fs::set_permissions(path, permissions)?;
    }
    fs::File::open(path)?.sync_all()
}
fn sync_parent(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path.parent().ok_or_else(|| io::Error::other("无父目录"))?)?.sync_all()?;
    Ok(())
}
fn conflict(path: &Path) -> io::Error {
    io::Error::other(format!("文件发生外部变动，已保留内容: {}", path.display()))
}

impl Journal {
    pub fn replace_files(&mut self, inputs: &[ReplacementInput]) -> io::Result<()> {
        self.replace_files_observed(inputs, &mut |_| Ok(()))
    }
    /// Hooks expose filesystem boundaries for deterministic interruption/failure tests.
    pub fn replace_files_observed(
        &mut self,
        inputs: &[ReplacementInput],
        checkpoint: &mut dyn FnMut(ReplacementCheckpoint) -> io::Result<()>,
    ) -> io::Result<()> {self.replace_files_checked(inputs,checkpoint,&||false)}
    pub fn replace_files_cancellable(&mut self,inputs:&[ReplacementInput],cancel:&tokio_util::sync::CancellationToken)->io::Result<()>{self.replace_files_checked(inputs,&mut |_|Ok(()),&||cancel.is_cancelled())}
    pub(crate) fn replace_files_checked(&mut self,inputs:&[ReplacementInput],checkpoint:&mut dyn FnMut(ReplacementCheckpoint)->io::Result<()>,cancel:&dyn Fn()->bool)->io::Result<()> {
        crate::append::check_stopped(cancel)?;
        if self.version!=4 && (!self.ops.is_empty() || !self.replacements.is_empty()) {
            return Err(io::Error::other("修订必须使用独立日志"));
        }
        let root_guard=crate::pipeline::RootGuard::capture(&self.root,false)?;
        let mut checked_checkpoint=|point| {crate::append::check_stopped(cancel)?;root_guard.verify()?;checkpoint(point)?;root_guard.verify()?;crate::append::check_stopped(cancel)};
        let checkpoint=&mut checked_checkpoint;
        let mut targets = std::collections::HashSet::new();
        for input in inputs {
            within(&self.root, &input.path)?;
            if !targets.insert(&input.path) || current(&input.path)? != input.expected {
                return Err(conflict(&input.path));
            }
        }
        if self.version!=4{self.version = 3;}
        self.status = JournalStatus::Partial;
        let storage = super::dir(&self.root.canonicalize()?).join(format!("revision-{}", self.id));
        within(&self.root, &storage)?;
        let start=self.replacements.len();
        self.replacements.extend(inputs
            .iter()
            .enumerate()
            .map(|(i, input)| {let i=start+i;Replacement {
                path: input.path.clone(),
                before: input.expected.clone(),
                after: digest(&input.bytes),
                backup: input
                    .expected
                    .as_ref()
                    .map(|_| storage.join(format!("{i}.before"))),
                stage: storage.join(format!("{i}.after")),
                undone: false,
                mode: original_mode(&input.path),
                readonly: fs::metadata(&input.path)
                    .ok()
                    .map(|m| m.permissions().readonly()),
                before_owner: None,
            }}));
        let owners=find_before_owners(&self.root,&self.replacements[start..],cancel)?;
        for (r,owner) in self.replacements[start..].iter_mut().zip(owners){r.before_owner=owner;}
        crate::append::check_stopped(cancel)?;
        root_guard.verify()?;self.persist_progress()?;
        checkpoint(ReplacementCheckpoint::Intent)?;
        if start==0{fs::create_dir(&storage)?;}
        // Prepare every backup and replacement before changing any target.
        for (i, (input, r)) in inputs.iter().zip(&self.replacements[start..]).enumerate() {
            let old = read(&r.path)?;
            if old.as_ref().map(|b| digest(b)) != r.before {
                return Err(conflict(&r.path));
            }
            if let (Some(path), Some(bytes)) = (&r.backup, old) {
                write_new(path, &bytes)?;
            }
            checkpoint(ReplacementCheckpoint::Backup(i))?;
            write_new(&r.stage, &input.bytes)?;
        }
        self.persist_progress()?;
        checkpoint(ReplacementCheckpoint::Prepared)?;
        self.validate_replacements()?;
        for r in &self.replacements[start..] {
            if let Some(backup) = &r.backup {
                if current(backup)? != r.before {
                    return Err(conflict(backup));
                }
            }
            if current(&r.stage)? != Some(r.after.clone()) {
                return Err(conflict(&r.stage));
            }
        }
        for (i, r) in self.replacements.iter().enumerate().skip(start) {
            within(&self.root, &r.path)?;
            if current(&r.path)? != r.before {
                return Err(conflict(&r.path));
            }
            if let Some(owner) = &r.before_owner {
                let anchor = owner_stage(&self.root, r, owner)?;
                if !same_file::is_same_file(&r.path, &anchor).unwrap_or(false) {
                    return Err(conflict(&r.path));
                }
            }
            let bytes = read(&r.stage)?.ok_or_else(|| conflict(&r.stage))?;
            if digest(&bytes) != r.after {
                return Err(conflict(&r.stage));
            }
            let publish = r.stage.with_extension("publish");
            write_new(&publish, &bytes)?;
            restore_permissions(&publish, r)?;
            if current(&r.path)? != r.before {
                return Err(conflict(&r.path));
            }
            if let Some(owner) = &r.before_owner {
                let anchor = owner_stage(&self.root, r, owner)?;
                if !same_file::is_same_file(&r.path, &anchor).unwrap_or(false) {
                    return Err(conflict(&r.path));
                }
            }
            fs::rename(&publish, &r.path)?;
            sync_parent(&r.path)?;
            checkpoint(ReplacementCheckpoint::Replaced(i))?;
            self.persist_progress()?;
        }
        self.status = JournalStatus::Applied;
        if let Err(error) = self.persist_progress() {
            self.status = JournalStatus::Partial;
            return Err(error);
        }
        Ok(())
    }
    pub(super) fn rollback_replacements(&mut self) -> Vec<String> {
        if self.status == JournalStatus::RolledBack {
            return vec![];
        }
        if let Err(e) = self.validate_replacements() {
            return vec![e.to_string()];
        }
        self.status = JournalStatus::Partial;
        if let Err(e) = self.persist_progress() {
            return vec![e.to_string()];
        }
        let mut failures = vec![];
        let mut order = (0..self.replacements.len()).rev().collect::<Vec<_>>();
        order.sort_by_key(|&i| {
            self.replacements[i].before.is_none()
                && Self::is_original_asset(&self.replacements[i].path)
        });
        for i in order {
            if self.replacements[i].undone {
                continue;
            }
            let r = &self.replacements[i];
            let result = (|| -> io::Result<()> {
                let now = current(&r.path)?;
                // Also covers interruption before replacement or after restore but before progress save.
                if now == r.before {
                    if let Some(owner) = &r.before_owner {
                        let backup = r.backup.as_ref().ok_or_else(|| conflict(&r.path))?;
                        if current(backup)? != r.before {
                            return Err(conflict(backup));
                        }
                        let anchor = owner_stage(&self.root, r, owner)?;
                        if !same_file::is_same_file(&r.path, &anchor).unwrap_or(false) {
                            return Err(conflict(&r.path));
                        }
                    }
                    return Ok(());
                }
                if now != Some(r.after.clone()) {
                    return Err(conflict(&r.path));
                }
                if let Some(backup) = &r.backup {
                    let bytes = read(backup)?.ok_or_else(|| conflict(backup))?;
                    if Some(digest(&bytes)) != r.before {
                        return Err(conflict(backup));
                    }
                    let restore = r.stage.with_extension("restore");
                    if let Some(owner) = &r.before_owner {
                        let anchor = owner_stage(&self.root, r, owner)?;
                        // Reuse only a previously linked restore, never an equal-byte copy.
                        if read(&restore)?.is_some() {
                            if !same_file::is_same_file(&restore, &anchor).unwrap_or(false) {
                                return Err(conflict(&restore));
                            }
                        } else {
                            fs::hard_link(&anchor, &restore)?;
                            sync_parent(&restore)?;
                        }
                        restore_permissions(&restore, r)?;
                        // Recheck the live source journal and inode after all preparation.
                        owner_stage(&self.root, r, owner)?;
                        if current(&r.path)? != now {
                            return Err(conflict(&r.path));
                        }
                        fs::rename(&restore, &r.path)?;
                        return sync_parent(&r.path);
                    }
                    // A previous interrupted restore may have left its independent temporary copy.
                    if let Some(b) = read(&restore)? {
                        if Some(digest(&b)) != r.before {
                            return Err(conflict(&restore));
                        }
                    } else {
                        write_new(&restore, &bytes)?;
                    }
                    restore_permissions(&restore, r)?;
                    if current(&r.path)? != now {
                        return Err(conflict(&r.path));
                    }
                    fs::rename(&restore, &r.path)?;
                } else {
                    self.ensure_original_unreferenced(&r.path)?;
                    fs::remove_file(&r.path)?;
                }
                sync_parent(&r.path)
            })();
            match result {
                Ok(()) => {
                    self.replacements[i].undone = true;
                    if let Err(e) = self.persist_progress_undo(Some(i)) {
                        failures.push(e.to_string());
                        return failures;
                    }
                }
                Err(e) => failures.push(e.to_string()),
            }
        }
        if failures.is_empty() {
            self.status = JournalStatus::RolledBack;
            if let Err(e) = self.save() {
                self.status = JournalStatus::Partial;
                failures.push(e.to_string());
            }
        }
        failures
    }
    pub(super) fn validate_replacements(&self) -> io::Result<()> {self.validate_replacements_checked(&||false)}
    pub(super) fn validate_replacements_checked(&self,cancel:&dyn Fn()->bool)->io::Result<()> {
        let storage = super::dir(&self.root.canonicalize()?).join(format!("revision-{}", self.id));
        for (i, r) in self.replacements.iter().enumerate() {
            crate::append::check_stopped(cancel)?;
            for path in [&r.path, &r.stage].into_iter().chain(r.backup.iter()) {
                within(&self.root, path)?;
            }
            if r.stage != storage.join(format!("{i}.after"))
                || r.backup
                    != r.before
                        .as_ref()
                        .map(|_| storage.join(format!("{i}.before")))
            {
                return Err(io::Error::other("修订备份路径不匹配"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod discovery_read_tests {
    use super::*;
    #[test]
    fn all_replacement_owners_are_discovered_in_one_history_pass(){
        let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();
        let mut imported=Journal::new(&root);
        let plan=crate::organize::Plan{ops:(0..4).map(|i|crate::organize::Op::WriteFile{path:root.join(format!("{i}.nfo")),content:"before".into()}).collect()};
        assert!(imported.execute(&plan,&mut |_|unreachable!(),&||false).error.is_none());imported.save().unwrap();
        let before=fs::metadata(imported.path()).unwrap().len();
        super::super::storage::metrics::take_reads();
        let mut revised=Journal::new(&root);
        revised.replace_files(&plan.ops.iter().map(|op|ReplacementInput{path:op.target().to_path_buf(),expected:Some(digest(b"before")),bytes:b"after".to_vec()}).collect::<Vec<_>>()).unwrap();
        let reads=super::super::storage::metrics::take_reads();
        // Discovery + independent fresh authorization + unchanged per-boundary
        // owner checks. The discovery itself no longer multiplies by inputs.
        assert!(reads<=before*12,"read {reads}, journal {before}");
        assert!(revised.replacements.iter().all(|r|r.before_owner.is_some()));
        assert!(revised.rollback().is_empty());assert!(imported.rollback().is_empty());
    }
}

#[cfg(test)]
mod discovery_growth_tests {
    use super::*;
    #[test]
    #[ignore="isolated ownership discovery elapsed and actual log read bytes"]
    fn ownership_discovery_growth(){
        for (histories,ops,inputs) in [(10,100,10),(100,500,20),(100,500,100)] {
            let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();
            for h in 0..histories {let mut j=Journal::new(&root);j.push_ops((0..ops).map(|i|crate::organize::Op::WriteFile{path:root.join(format!("unused-{h}-{i}.nfo")),content:String::new()}));j.save().unwrap();}
            let mut owned=Journal::new(&root);let plan=crate::organize::Plan{ops:(0..inputs).map(|i|crate::organize::Op::WriteFile{path:root.join(format!("target-{i}.nfo")),content:"before".into()}).collect()};
            assert!(owned.execute(&plan,&mut |_|unreachable!(),&||false).error.is_none());owned.save().unwrap();
            let rs=plan.ops.iter().enumerate().map(|(i,op)|Replacement{path:op.target().to_path_buf(),before:Some(digest(b"before")),after:digest(b"after"),backup:None,stage:root.join(format!("{i}.after")),undone:false,mode:None,readonly:None,before_owner:None}).collect::<Vec<_>>();
            // Conservative baseline: repeat the current safe fresh discovery for
            // one request at a time, without the old unpruned per-op disk overhead.
            super::super::storage::metrics::take_reads();let start=std::time::Instant::now();
            let reference=rs.iter().map(|r|find_before_owners(&root,std::slice::from_ref(r),&||false).unwrap().remove(0)).collect::<Vec<_>>();
            let old_ms=start.elapsed().as_millis();let old_reads=super::super::storage::metrics::take_reads();let start=std::time::Instant::now();
            let batch=find_before_owners(&root,&rs,&||false).unwrap();let ms=start.elapsed().as_millis();let reads=super::super::storage::metrics::take_reads();
            assert_eq!(batch,reference);assert!(batch.iter().all(Option::is_some));assert!(reads*2<old_reads);
            println!("owners histories={histories} ops={ops} inputs={inputs} repeated={old_ms}ms/{old_reads}B batch={ms}ms/{reads}B");
        }
    }
}
