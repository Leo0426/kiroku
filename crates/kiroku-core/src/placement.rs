//! Explicit source-preserving media placement and narrowly registered file links.
use crate::{metadata::Metadata, organize::Op};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrganizeMode {
    #[default]
    Move,
    Copy,
    Hardlink,
    Symlink,
}
impl OrganizeMode {
    pub fn operation(self, from: PathBuf, to: PathBuf) -> Op {
        if self == Self::Move {
            return Op::MoveFile { from, to };
        }
        let expected = source_proof(&from).ok();
        self.retained_operation(from, to, expected)
    }
    pub fn operation_cancellable(
        self,
        from: PathBuf,
        to: PathBuf,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> io::Result<Op> {
        if self == Self::Move {
            return Ok(Op::MoveFile { from, to });
        }
        let expected = Some(source_proof_cancellable(&from, cancel)?);
        Ok(self.retained_operation(from, to, expected))
    }
    pub(crate) fn retained_operation(
        self,
        from: PathBuf,
        to: PathBuf,
        expected: Option<String>,
    ) -> Op {
        match self {
            Self::Move => Op::MoveFile { from, to },
            Self::Copy => Op::CopyFile { from, to, expected },
            Self::Hardlink => Op::HardlinkFile { from, to, expected },
            Self::Symlink => Op::SymlinkFile { from, to, expected },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub path: PathBuf,
    pub mode: OrganizeMode,
    pub source: PathBuf,
    pub source_proof: String,
    pub size: u64,
    #[serde(default)]
    pub anchor: Option<PathBuf>,
}
pub fn source_proof(path: &Path) -> io::Result<String> {
    source_proof_cancellable(path, &tokio_util::sync::CancellationToken::new())
}
pub fn source_proof_cancellable(
    path: &Path,
    cancel: &tokio_util::sync::CancellationToken,
) -> io::Result<String> {
    source_proof_observed(path, cancel, &|_, _| {})
}
pub fn source_proof_observed(
    path: &Path,
    cancel: &tokio_util::sync::CancellationToken,
    progress: &dyn Fn(u64, u64),
) -> io::Result<String> {
    crate::append::fingerprint_observed(path, cancel, progress)
}
pub fn capture(
    mode: OrganizeMode,
    source: &Path,
    target: &Path,
    dir: &Path,
) -> io::Result<Placement> {
    capture_proven(mode, source, target, dir, source_proof(source)?)
}

// A caller may reuse a proof only from its own fresh read in the current phase.
// This builds a record; it is never authorization for a later execution.
pub(crate) fn capture_proven(mode: OrganizeMode, source: &Path, target: &Path, dir: &Path, source_proof: String) -> io::Result<Placement> {
    let source = source.canonicalize()?;
    let size = source_proof.split(':').next().and_then(|s| s.parse().ok())
        .ok_or_else(||io::Error::other("媒体证明大小无效"))?;
    Ok(Placement { path: target.strip_prefix(dir).map_err(io::Error::other)?.to_path_buf(),
        mode, source, source_proof, size, anchor: None })
}

pub struct ResolvedMedia {
    pub path: PathBuf,
    pub source_proof: Option<String>,
}
pub fn resolve_media(
    root: &Path,
    movie_dir: &Path,
    file: &Path,
    meta: &Metadata,
) -> io::Result<PathBuf> {
    resolve_media_cancellable(
        root,
        movie_dir,
        file,
        meta,
        &tokio_util::sync::CancellationToken::new(),
    )
}
pub fn resolve_media_cancellable(
    root: &Path,
    movie_dir: &Path,
    file: &Path,
    meta: &Metadata,
    cancel: &tokio_util::sync::CancellationToken,
) -> io::Result<PathBuf> {
    Ok(resolve_media_observed(root, movie_dir, file, meta, cancel, &|_, _| {})?.path)
}
pub fn resolve_media_observed(
    root: &Path,
    movie_dir: &Path,
    file: &Path,
    meta: &Metadata,
    cancel: &tokio_util::sync::CancellationToken,
    progress: &dyn Fn(u64, u64),
) -> io::Result<ResolvedMedia> {
    crate::append::check_cancel(cancel)?;
    crate::append::confined_path(root, movie_dir)?;
    let parent = file
        .parent()
        .ok_or_else(|| io::Error::other("媒体路径无效"))?;
    crate::append::confined_path(root, parent)?;
    let rel = file.strip_prefix(movie_dir).map_err(io::Error::other)?;
    let m = fs::symlink_metadata(file)?;
    if !m.file_type().is_symlink() {
        crate::append::regular(file)?;
        return Ok(ResolvedMedia {
            path: file.to_path_buf(),
            source_proof: None,
        });
    }
    let p = meta
        .placements
        .iter()
        .find(|p| p.path == rel && p.mode == OrganizeMode::Symlink)
        .ok_or_else(|| io::Error::other("未登记媒体链接"))?;
    if fs::read_link(file)? != p.source {
        return Err(io::Error::other("媒体链接目标已变化"));
    }
    no_link_ancestors(&p.source)?;
    if !valid_media(&p.source)
        || !valid_media(&p.path)
        || p.source.components().any(|c| c.as_os_str() == ".kiroku")
    {
        return Err(io::Error::other("链接目标不是允许的媒体文件"));
    }
    let anchor = registration_anchor(root, file, p)?;
    if !same_link(file, &anchor)? {
        return Err(io::Error::other("媒体链接被替换"));
    }
    let source_proof = source_proof_observed(&p.source, cancel, progress)?;
    if source_proof != p.source_proof {
        return Err(io::Error::other("媒体链接来源身份或内容已变化"));
    }
    no_link_ancestors(&p.source)?;
    verify_registration(root, file, p)?;
    if !same_link(file, &anchor)? || fs::read_link(file)? != p.source {
        return Err(io::Error::other("摘要读取期间媒体链接已变化"));
    }
    Ok(ResolvedMedia {
        path: p.source.clone(),
        source_proof: Some(source_proof),
    })
}
pub(crate) fn same_link(a: &Path, b: &Path) -> io::Result<bool> {
    let a = fs::symlink_metadata(a)?;
    let b = fs::symlink_metadata(b)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = (a, b);
        Err(io::Error::other("此平台无法安全验证软链接身份"))
    }
}
pub(crate) fn symlink(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, target)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(source, target)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (source, target);
        Err(io::Error::other("不支持软链接"))
    }
}

/// Validate preview source proofs, then bind manifest records to durable ownership anchors.
pub fn bind_plan(
    plan: &mut crate::organize::Plan,
    journal: &crate::journal::Journal,
) -> io::Result<()> {
    bind_plan_checked(plan,journal,&||false)
}

pub(crate) fn bind_plan_checked(plan:&mut crate::organize::Plan,journal:&crate::journal::Journal,cancel:&dyn Fn()->bool)->io::Result<()> {
    crate::append::check_stopped(cancel)?;
    let mut fresh = std::collections::HashMap::new();
    for op in plan.ops.iter().filter(|op|op.retained()) {
        let from=op.source().unwrap();
        let canonical=from.canonicalize()?;
        if !fresh.contains_key(&canonical) { fresh.insert(canonical.clone(),crate::append::fingerprint_stream(from,cancel,&|_,_|{},None)?); }
        let expected=match op { Op::CopyFile{expected,..}|Op::HardlinkFile{expected,..}|Op::SymlinkFile{expected,..}=>expected.as_ref(),_=>None };
        if expected != fresh.get(&canonical) { return Err(io::Error::other("预览后源文件已变化，请重新预览")); }
    }
    for op in &mut plan.ops {
        if let Op::WriteFile { path, content } = op {
            if path
                .file_name()
                .is_some_and(|n| n == crate::organize::SIDECAR)
            {
                let mut meta: Metadata = serde_json::from_str(content).map_err(io::Error::other)?;
                for p in &mut meta.placements {
                    if p.anchor.is_none() {
                        no_link_ancestors(&p.source)?;
                        if !valid_media(&p.source)
                            || !valid_media(&p.path)
                            || p.source.components().any(|c| c.as_os_str() == ".kiroku")
                        {
                            return Err(io::Error::other("链接目标不是允许的媒体文件"));
                        }
                        let current = match fresh.get(&p.source) {
                            Some(proof)=>proof.clone(),
                            None=> {let proof=crate::append::fingerprint_stream(&p.source,cancel,&|_,_|{},None)?;fresh.insert(p.source.clone(),proof.clone());proof}
                        };
                        if current != p.source_proof {
                            return Err(io::Error::other("预览后源文件已变化，请重新预览"));
                        }
                    }
                }
                *content = serde_json::to_string_pretty(&meta).map_err(io::Error::other)?;
            }
        }
    }
    let prior = journal
        .ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.retained())
        .filter_map(|(i, op)| journal.proofs.get(&i).map(|proof| (op, proof)))
        .map(|(op, proof)| {
            let mode = match op {
                Op::CopyFile { .. } => OrganizeMode::Copy,
                Op::HardlinkFile { .. } => OrganizeMode::Hardlink,
                _ => OrganizeMode::Symlink,
            };
            let source_proof = proof
                .source_proof
                .clone()
                .ok_or_else(|| io::Error::other("媒体缺少源证明"))?;
            let size = source_proof
                .split(':')
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| io::Error::other("媒体证明大小无效"))?;
            let p = Placement {
                mode,
                path: op
                    .target()
                    .file_name()
                    .ok_or_else(|| io::Error::other("媒体文件名无效"))?
                    .into(),
                source: proof.link.clone().unwrap_or_else(|| {
                    op.source()
                        .unwrap()
                        .canonicalize()
                        .unwrap_or_else(|_| op.source().unwrap().to_path_buf())
                }),
                source_proof,
                size,
                anchor: Some(proof.stage.clone()),
            };
            Ok((op.target().parent().unwrap().to_path_buf(), p))
        })
        .collect::<io::Result<Vec<_>>>()?;
    for op in &mut plan.ops {
        if let Op::WriteFile { path, content } = op {
            if path
                .file_name()
                .is_some_and(|n| n == crate::organize::SIDECAR)
            {
                let mut meta: Metadata = serde_json::from_str(content).map_err(io::Error::other)?;
                for (dir, p) in &prior {
                    if path.parent() == Some(dir) {
                        meta.placements.retain(|old| old.path != p.path);
                        meta.placements.push(p.clone());
                    }
                }
                *content = serde_json::to_string_pretty(&meta).map_err(io::Error::other)?;
            }
        }
    }
    let placements = plan
        .ops
        .iter()
        .enumerate()
        .filter(|(_, op)| op.retained())
        .map(|(i, op)| {
            let from = op.source().unwrap();
            let dir = op.target().parent().unwrap();
            let mode = match op {
                Op::CopyFile { .. } => OrganizeMode::Copy,
                Op::HardlinkFile { .. } => OrganizeMode::Hardlink,
                _ => OrganizeMode::Symlink,
            };
            let mut p = capture_proven(mode, from, op.target(), dir, fresh.get(&from.canonicalize()?).expect("validated source").clone())?;
            p.anchor = Some(std::path::absolute(dir.join(format!(
                ".kiroku-{}-{}.tmp",
                journal.id,
                journal.ops.len() + i
            )))?);
            Ok((dir.to_path_buf(),p))
        })
        .collect::<io::Result<Vec<_>>>()?;
    for op in &mut plan.ops {
        if let Op::WriteFile { path, content } = op {
            if path
                .file_name()
                .is_some_and(|n| n == crate::organize::SIDECAR)
            {
                let mut meta: Metadata = serde_json::from_str(content).map_err(io::Error::other)?;
                for (dir,p) in &placements {
                    if path.parent()==Some(dir) {
                        meta.placements.retain(|old| old.path != p.path);
                        meta.placements.push(p.clone());
                    }
                }
                *content = serde_json::to_string_pretty(&meta).map_err(io::Error::other)?;
            }
        }
    }
    Ok(())
}

/// Existing-movie manifests use the v3 replacement protocol inside a v4 placement journal.
pub fn execute(
    journal: &mut crate::journal::Journal,
    plan: &crate::organize::Plan,
    fetch: &mut dyn FnMut(&str) -> io::Result<Vec<u8>>,
    cancel: &dyn Fn() -> bool,
) -> crate::organize::Exec {
    execute_images(journal, plan, &mut |url| fetch(url).map(crate::artwork::ImageInput::Unchecked), cancel)
}

pub(crate) fn execute_images(
    journal: &mut crate::journal::Journal,
    plan: &crate::organize::Plan,
    fetch: &mut dyn FnMut(&str) -> io::Result<crate::artwork::ImageInput>,
    cancel: &dyn Fn() -> bool,
) -> crate::organize::Exec {
    let prepare = (|| -> io::Result<Option<crate::journal::ReplacementInput>> {
        if !plan.ops.iter().any(Op::retained)||plan.ops.iter().any(|op|matches!(op,Op::WriteFile{path,..} if path.file_name().is_some_and(|n|n==crate::organize::SIDECAR))){return Ok(None)}
        let dir = plan
            .ops
            .iter()
            .find(|op| op.retained())
            .unwrap()
            .target()
            .parent()
            .unwrap();
        let path = dir.join(crate::organize::SIDECAR);
        if !crate::organize::occupied(&path) {
            return Ok(None);
        }
        let mut metadata = crate::append::read_meta(dir)?;
        for (i, op) in plan.ops.iter().enumerate().filter(|(_, op)| op.retained()) {
            let mode = match op {
                Op::CopyFile { .. } => OrganizeMode::Copy,
                Op::HardlinkFile { .. } => OrganizeMode::Hardlink,
                _ => OrganizeMode::Symlink,
            };
            let proof=match op {Op::CopyFile{expected,..}|Op::HardlinkFile{expected,..}|Op::SymlinkFile{expected,..}=>expected.clone().ok_or_else(||io::Error::other("媒体缺少预览来源证明"))?,_=>unreachable!()};
            // The media commit revalidates this exact preview proof before publishing.
            let mut p = capture_proven(mode, op.source().unwrap(), op.target(), dir, proof)?;
            p.anchor = Some(std::path::absolute(dir.join(format!(
                ".kiroku-{}-{}.tmp",
                journal.id,
                journal.ops.len() + i
            )))?);
            metadata.placements.push(p);
        }
        Ok(Some(crate::journal::ReplacementInput {
            expected: crate::journal::replacement::current(&path)?,
            path,
            bytes: serde_json::to_vec_pretty(&metadata).map_err(io::Error::other)?,
        }))
    })();
    let replacement = match prepare {
        Ok(p) => p,
        Err(e) => {
            return crate::organize::Exec {
                error: Some(e),
                ..Default::default()
            }
        }
    };
    let mut out = journal.execute_images_observed(plan, fetch, cancel, &mut |_| Ok(()));
    if out.error.is_none() && !cancel() {
        if let Some(input) = replacement {
            if let Err(e) = journal.replace_files_checked(&[input],&mut |_|Ok(()),cancel) {
                out.error = Some(e);
            }
        }
    }
    out
}

pub fn revalidate(root: &Path, dir: &Path, meta: &Metadata) -> io::Result<Metadata> {
    crate::append::confined_path(root, dir)?;
    let mut after = meta.clone();
    for p in after
        .placements
        .iter_mut()
        .filter(|p| p.mode == OrganizeMode::Symlink)
    {
        let file = dir.join(&p.path);
        crate::append::confined_path(
            root,
            file.parent()
                .ok_or_else(|| io::Error::other("媒体路径无效"))?,
        )?;
        let anchor = p
            .anchor
            .as_ref()
            .ok_or_else(|| io::Error::other("缺少媒体链接所有权证明"))?;
        crate::append::confined_path(
            root,
            anchor
                .parent()
                .ok_or_else(|| io::Error::other("证明路径无效"))?,
        )?;
        verify_registration(root, &file, p)?;
        if !same_link(&file, anchor)? || fs::read_link(&file)? != p.source {
            return Err(io::Error::other("链接被替换，不能重新登记任意链接"));
        }
        no_link_ancestors(&p.source)?;
        if !valid_media(&p.source) {
            return Err(io::Error::other("链接目标不是媒体文件"));
        }
        p.source_proof = source_proof(&p.source)?;
        p.size = fs::metadata(&p.source)?.len();
    }
    Ok(after)
}
pub fn revalidation_preview(
    root: &Path,
    dir: &Path,
    code: &str,
) -> io::Result<crate::revision::Preview> {
    let meta = crate::append::read_meta(dir)?;
    crate::revision::prepare(root, dir, code, revalidate(root, dir, &meta)?)
}

pub(crate) fn valid_media(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    crate::pipeline::VIDEO_EXT.contains(&ext.as_str())
        || crate::subtitle::SUB_EXT.contains(&ext.as_str())
}
fn no_link_ancestors(path: &Path) -> io::Result<()> {
    let mut current = path.parent();
    while let Some(parent) = current {
        if fs::symlink_metadata(parent)?.file_type().is_symlink() {
            return Err(io::Error::other("媒体来源父目录为符号链接"));
        }
        current = parent.parent();
    }
    Ok(())
}

fn verify_registration(root: &Path, file: &Path, p: &Placement) -> io::Result<()> {
    registration_anchor(root, file, p).map(|_| ())
}
fn registration_anchor(root: &Path, file: &Path, p: &Placement) -> io::Result<PathBuf> {
    let failure = || io::Error::other("媒体链接没有有效日志登记证明");
    let anchor = p.anchor.as_ref().ok_or_else(failure)?;
    let name = anchor
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(failure)?;
    let (id, index) = name
        .strip_prefix(".kiroku-")
        .and_then(|n| n.strip_suffix(".tmp"))
        .and_then(|n| n.rsplit_once('-'))
        .ok_or_else(failure)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(failure());
    }
    let index: usize = index.parse().map_err(|_| failure())?;
    let journal_path = crate::journal::dir(root).join(format!("journal-{id}.json"));
    crate::append::confined_path(root, &journal_path)?;
    let j = crate::journal::Journal::load(&journal_path)?;
    if j.version != 4
        || j.status == crate::journal::JournalStatus::RolledBack
        || j.rolled_back_ops.contains(&index)
    {
        return Err(failure());
    }
    let op = j.ops.get(index).ok_or_else(failure)?;
    let proof = j.proofs.get(&index).ok_or_else(failure)?;
    let normalize = |path: &Path| -> io::Result<PathBuf> {
        Ok(
            crate::append::confined_path(root, path.parent().ok_or_else(failure)?)
                .or_else(|_| {
                    crate::journal::replacement::recorded_path(
                        root,
                        &j.root,
                        path.parent().ok_or_else(failure)?,
                    )
                })?
                .join(path.file_name().ok_or_else(failure)?),
        )
    };
    let matching_mode = matches!(
        (p.mode, op),
        (OrganizeMode::Symlink, Op::SymlinkFile { .. })
            | (OrganizeMode::Copy, Op::CopyFile { .. })
            | (OrganizeMode::Hardlink, Op::HardlinkFile { .. })
    );
    if !matching_mode
        || normalize(op.target())? != normalize(file)?
        || normalize(&proof.stage)? != normalize(anchor)?
        || (p.mode == OrganizeMode::Symlink && proof.link.as_ref() != Some(&p.source))
    {
        return Err(failure());
    }
    normalize(anchor)
}

/// Only an exact live manifest/journal-owned staging name may be omitted from append inputs.
pub(crate) fn is_owned_anchor(
    root: &Path,
    dir: &Path,
    path: &Path,
    meta: &Metadata,
) -> io::Result<bool> {
    let candidate = crate::append::confined_path(
        root,
        path.parent()
            .ok_or_else(|| io::Error::other("证明无父目录"))?,
    )?
    .join(
        path.file_name()
            .ok_or_else(|| io::Error::other("证明文件名无效"))?,
    );
    for p in &meta.placements {
        if let Some(anchor) = &p.anchor {
            if anchor.file_name() == path.file_name() {
                let media = dir.join(&p.path);
                let registered = registration_anchor(root, &media, p)?;
                if registered != candidate {
                    return Err(io::Error::other("媒体证明位置不匹配"));
                }
                if p.mode == OrganizeMode::Symlink {
                    if fs::read_link(path)? != p.source || !same_link(path, &media)? {
                        return Err(io::Error::other("媒体证明被替换"));
                    }
                } else if !same_file::is_same_file(path, &media)? {
                    return Err(io::Error::other("媒体证明被替换"));
                }
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod read_budget_tests {
    use super::*;
    #[test]
    fn retained_plan_and_binding_each_read_source_once() {
        let t=tempfile::tempdir().unwrap();
        let source=t.path().join("ABC-123.mp4");
        fs::write(&source,vec![7;1024*1024]).unwrap();
        let root=t.path().join("library");
        let code=crate::code::parse_code("ABC-123.mp4").unwrap();
        crate::append::hash_reads::take();
        let mut plan=crate::organize::plan_with(&source,&code,&Metadata{code:code.id.clone(),..Default::default()},&root,"{code}",true,
            crate::organize::PlanOpts{mode:OrganizeMode::Copy,..Default::default()});
        assert_eq!(crate::append::hash_reads::take(),1024*1024,"preview must not hash twice to construct its manifest");
        bind_plan(&mut plan,&crate::journal::Journal::new(&root)).unwrap();
        assert_eq!(crate::append::hash_reads::take(),1024*1024,"binding validates preview and constructs manifest from the same fresh read");
    }
}
