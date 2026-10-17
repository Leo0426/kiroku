//! Add files to an existing on-disk movie without replacing metadata or artwork.
use crate::{
    code::{parse_code, Code},
    journal::{Journal, JournalItem, JournalStatus},
    library::Library,
    metadata::Metadata,
    organize::{Op, Plan},
    pipeline::VIDEO_EXT,
    subtitle::SUB_EXT,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashSet,HashMap},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;
mod targets;
pub(crate) use targets::Targets;
#[cfg(test)] pub(crate) use targets::stats as discovery_stats;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendPreview {
    pub root: PathBuf,
    pub code: String,
    pub target_dir: PathBuf,
    pub ops: Vec<Op>,
    pub skipped: Vec<String>,
    pub conflicts: Vec<String>,
    /// 同分片已有不同内容的视频,本次作为新版本并存的目标(ADR-0008)
    pub new_versions: Vec<PathBuf>,
    #[serde(skip)]
    guards: Vec<(PathBuf, String)>,
    #[serde(skip)]
    directory: Vec<PathBuf>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppendResult {
    pub journal_id: String,
    pub completed: usize,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub cancelled: bool,
}
fn error(s: impl Into<String>) -> io::Error {
    io::Error::other(s.into())
}
pub(crate) fn fingerprint_cancellable(path:&Path,cancel:&CancellationToken)->io::Result<String>{
    fingerprint_observed(path,cancel,&|_,_|{})
}
#[cfg(test)]
pub(crate) mod hash_reads {
    std::thread_local! { static BYTES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
    std::thread_local! { static MEDIA_WRITES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
    pub fn record_write(bytes: usize) { MEDIA_WRITES.with(|n| n.set(n.get() + bytes as u64)); }
    pub fn take_writes() -> u64 { MEDIA_WRITES.with(|n| n.replace(0)) }
    pub fn record(bytes: usize) { BYTES.with(|n| n.set(n.get() + bytes as u64)); }
    pub fn take() -> u64 { BYTES.with(|n| n.replace(0)) }
}
pub(crate) fn fingerprint_observed(path:&Path,cancel:&CancellationToken,progress:&dyn Fn(u64,u64))->io::Result<String>{
    fingerprint_stream(path, &||cancel.is_cancelled(), progress, None)
}

pub(crate) fn check_stopped(cancel: &dyn Fn()->bool) -> io::Result<()> {
    if cancel() { Err(io::Error::new(io::ErrorKind::Interrupted,"已取消文件校验")) } else { Ok(()) }
}

/// Hash exactly the bytes read; optionally write those same chunks to an exclusive stage.
/// Source identity and metadata are checked across the entire stream, without a retained buffer.
pub(crate) fn fingerprint_stream(path:&Path,cancel:&dyn Fn()->bool,progress:&dyn Fn(u64,u64),mut sink:Option<&mut fs::File>)->io::Result<String>{
    check_stopped(cancel)?;regular(path)?;
    let mut file=fs::File::open(path)?;
    let handle=same_file::Handle::from_file(file.try_clone()?)?;
    let before=file.metadata()?;
    if !before.is_file() || same_file::Handle::from_path(path)?!=handle{return Err(error("来源身份已变化"))}
    let identity=|m:&fs::Metadata|->io::Result<String>{
        let mut id=format!("{}:{:?}",m.len(),m.modified()?);
        #[cfg(unix)] {use std::os::unix::fs::MetadataExt;id.push_str(&format!(":{}:{}",m.dev(),m.ino()));}
        Ok(id)
    };
    let id=identity(&before)?;let mut hash=Sha256::new();let mut buffer=[0;65536];let mut done=0;
    loop{check_stopped(cancel)?;let n=file.read(&mut buffer)?;if n==0{break}
        #[cfg(test)] hash_reads::record(n);
        if let Some(output)=sink.as_mut() {
            std::io::Write::write_all(output,&buffer[..n])?;
            #[cfg(test)] hash_reads::record_write(n);
        }
        hash.update(&buffer[..n]);done+=n as u64;progress(done,before.len());}
    check_stopped(cancel)?;regular(path)?;
    if identity(&file.metadata()?)?!=id || identity(&fs::symlink_metadata(path)?)?!=id || same_file::Handle::from_path(path)?!=handle{return Err(error("摘要读取期间来源身份或内容已变化"))}
    Ok(format!("{id}:{:x}",hash.finalize()))
}
/// Normalize only the root; descendants must not contain links or traversal.
pub fn confined_path(root: &Path, path: &Path) -> io::Result<PathBuf> {
    let alias = std::path::absolute(root)?;
    let root = root.canonicalize()?;
    let p = std::path::absolute(path)?;
    let rel = p
        .strip_prefix(&root)
        .or_else(|_| p.strip_prefix(&alias))
        .map_err(|_| error("路径不在媒体库内"))?;
    let mut current = root;
    for part in rel.components() {
        match part {
            std::path::Component::Normal(n) => current.push(n),
            _ => return Err(error("路径含越界片段")),
        };
        match fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => return Err(error("不允许符号链接")),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(current)
}
pub(crate) fn regular(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        return Err(error("不是普通文件或为符号链接"));
    }
    Ok(())
}
pub fn read_meta(dir: &Path) -> io::Result<Metadata> {
    let side = dir.join("kiroku.json");
    if side.exists() {
        regular(&side)?;
        return serde_json::from_slice(&fs::read(side)?).map_err(io::Error::other);
    }
    let nfo = crate::assets::nfo_path(dir)?;
    regular(&nfo)?;
    let mut meta = crate::nfo_read::parse_nfo(
        &fs::read_to_string(&nfo)?,
        dir.file_name().and_then(|s| s.to_str()).unwrap_or(""),
    )
    .ok_or_else(|| error("NFO 无法解析"))?;
    crate::assets::infer_from_nfo(&mut meta, &nfo);
    Ok(meta)
}
pub fn targets(root: &Path, code: &str) -> io::Result<Vec<PathBuf>> {
    targets_cancellable(root, code, &CancellationToken::new())
}
pub fn targets_cancellable(
    root: &Path,
    code: &str,
    cancel: &CancellationToken,
) -> io::Result<Vec<PathBuf>> {
    let snapshot=Targets::capture(root,std::iter::once(code),cancel)?;
    Ok(snapshot.get(code,cancel)?.to_vec())
}

fn entries(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    Ok(paths)
}
fn ext(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
}
pub fn preview(
    root: &Path,
    code: &str,
    inputs: &[PathBuf],
    target_dir: Option<&Path>,
) -> io::Result<AppendPreview> {
    preview_with_video(root, code, inputs, target_dir, None)
}
pub fn preview_with_video(
    root: &Path,
    code: &str,
    inputs: &[PathBuf],
    target_dir: Option<&Path>,
    target_video: Option<&Path>,
) -> io::Result<AppendPreview> {
    preview_with_video_cancellable(
        root,
        code,
        inputs,
        target_dir,
        target_video,
        &CancellationToken::new(),
    )
}
pub fn preview_with_video_cancellable(
    root: &Path,
    code: &str,
    inputs: &[PathBuf],
    target_dir: Option<&Path>,
    target_video: Option<&Path>,
    cancel: &CancellationToken,
) -> io::Result<AppendPreview> {
    check_cancel(cancel)?;
    let root_alias = root.to_path_buf();
    let root = root.canonicalize()?;
    let candidates = targets_cancellable(&root, code, cancel)?;
    preview_from_targets(&root_alias,code,inputs,target_dir,target_video,cancel,&candidates)
}

pub(crate) fn preview_from_targets(root:&Path,code:&str,inputs:&[PathBuf],target_dir:Option<&Path>,target_video:Option<&Path>,cancel:&CancellationToken,candidates:&[PathBuf])->io::Result<AppendPreview> {
    check_cancel(cancel)?;
    let root_alias=root.to_path_buf();let root=root.canonicalize()?;
    let dir = if let Some(d) = target_dir {
        let d = confined_path(&root_alias, d)?;
        if !candidates.contains(&d) {
            return Err(error("目标目录与影片不匹配"));
        }
        d
    } else {
        if candidates.len() != 1 {
            return Err(error(format!(
                "找到 {} 个影片目录，请明确选择目标",
                candidates.len()
            )));
        }
        candidates[0].clone()
    };
    let dir=confined_path(&root_alias,&dir)?;
    let directory = entries(&dir)?;
    let existing_meta=read_meta(&dir)?;
    if !same_code(&existing_meta.code,code){return Err(error("目标影片番号已变化，请重新生成计划"));}
    let mut guards = vec![];
    let mut verified=HashMap::new();
    for p in &directory {
        check_cancel(cancel)?;
        if crate::placement::is_owned_anchor(&root,&dir,p,&existing_meta)?{continue;}
        if fs::symlink_metadata(p)?.file_type().is_symlink(){
            let resolved=crate::placement::resolve_media_observed(&root,&dir,p,&existing_meta,cancel,&|_,_|{})?;
            let proof=resolved.source_proof.clone().ok_or_else(||error("链接缺少已验证源证明"))?;
            guards.push((resolved.path.clone(),proof.clone()));verified.insert(p.clone(),(resolved.path,proof));continue;
        }
        confined_path(&root, p)?;
        if fs::symlink_metadata(p)?.is_file() {
            let proof=fingerprint_cancellable(p,cancel)?;guards.push((p.clone(),proof.clone()));verified.insert(p.clone(),(p.clone(),proof));
        }
    }
    let videos: Vec<_> = directory
        .iter()
        .filter(|p| VIDEO_EXT.contains(&ext(p).as_str()))
        .cloned()
        .collect();
    if videos.is_empty() {
        return Err(error("目标影片没有视频，请先修复"));
    }
    let mut p = AppendPreview {
        root,
        code: read_meta(&dir)?.code,
        target_dir: dir.clone(),
        ops: vec![],
        skipped: vec![],
        conflicts: vec![],
        new_versions: vec![],
        guards,
        directory,
    };
    let selected_video = target_video
        .map(|v| {let path=confined_path(&root_alias,v.parent().ok_or_else(||error("媒体路径无效"))?)?.join(v.file_name().ok_or_else(||error("媒体路径无效"))?);if !verified.contains_key(&path){return Err(error("字幕目标不是已验证视频"))}Ok::<_,io::Error>(path)})
        .transpose()?;
    if selected_video.as_ref().is_some_and(|v| !videos.contains(v)) {
        return Err(error("字幕目标不是该影片的现有视频"));
    }
    let mut claimed = HashSet::new();
    let mut files = vec![];
    for input in inputs {
        check_cancel(cancel)?;
        if fs::symlink_metadata(input)?.is_dir() {
            for e in WalkDir::new(input).follow_links(false) {
                let e = e.map_err(io::Error::other)?;
                if e.file_type().is_file() {
                    files.push(e.into_path())
                } else if e.file_type().is_symlink() {
                    return Err(error("输入目录含符号链接"));
                }
            }
        } else {
            files.push(input.clone())
        }
    }
    for input in files.clone() {
        let extension = ext(&input);
        if extension == "idx" || extension == "sub" {
            let companion = input.with_extension(if extension == "idx" { "sub" } else { "idx" });
            if companion.exists() {
                files.push(companion)
            }
        }
    }
    files.sort();
    files.dedup();
    files.sort_by_key(|p| !VIDEO_EXT.contains(&ext(p).as_str()));
    let mut video_targets = videos.clone();
    let mut moves: Vec<(PathBuf, PathBuf)> = vec![];
    let mut proofs: HashMap<PathBuf, String> = HashMap::new();
    for input in &files {
        check_cancel(cancel)?;
        regular(input)?;
        let extension = ext(input);
        if SUB_EXT.contains(&extension.as_str()) && claimed.contains(input) {
            continue;
        }
        if !VIDEO_EXT.contains(&extension.as_str()) && !SUB_EXT.contains(&extension.as_str()) {
            continue;
        }
        let parse_input = if SUB_EXT.contains(&extension.as_str()) {
            input.file_name().unwrap().to_string_lossy().into_owned()
        } else {
            input.to_string_lossy().into_owned()
        };
        let c = parse_code(&parse_input)
            .or_else(|| {
                if SUB_EXT.contains(&extension.as_str()) {
                    selected_video
                        .as_ref()
                        .and_then(|v| parse_code(&v.to_string_lossy()))
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                error(format!(
                    "无法识别文件番号，请选择字幕目标视频：{}",
                    input.display()
                ))
            })?;
        if !same_code(&c.id, code) {
            p.conflicts.push(format!("番号不匹配：{}", input.display()));
            continue;
        }
        let to = if VIDEO_EXT.contains(&extension.as_str()) {
            let existing: Vec<_> = videos
                .iter()
                .filter(|v| parse_code(&v.to_string_lossy()).is_some_and(|v| v.part == c.part))
                .collect();
            if !existing.is_empty() {
                let incoming = fingerprint_cancellable(input, cancel)?;
                proofs.insert(input.clone(), incoming.clone());
                let digest = |proof: &str| proof.rsplit(':').next().map(str::to_string);
                // 与任一已有版本字节相同:交给下面的去重逻辑跳过(字幕仍可随迁)
                if let Some(same) = existing.iter().find(|v| verified.get(**v).is_some_and(|(_, proof)| digest(proof) == digest(&incoming))) {
                    (*same).clone()
                } else {
                    let anchor = existing.iter().min_by_key(|v| (v.as_os_str().len(), (**v).clone())).unwrap();
                    let taken = |path: &Path| crate::organize::occupied(path) || video_targets.contains(&path.to_path_buf()) || moves.iter().any(|(_, to)| to == path);
                    let to = version_path(anchor, &version_label(&c), &extension, taken);
                    p.new_versions.push(to.clone());
                    to
                }
            } else {
                let name = input.file_name().ok_or_else(|| error("文件名无效"))?;
                let anchor = videos[0].file_stem().unwrap().to_string_lossy();
                let re = regex::Regex::new(r"(?i)([- ](?:cd|disc|part) ?)(\d+)").unwrap();
                if let Some(part) = c.part.filter(|_| re.is_match(&anchor)) {
                    let stem = re.replace(&anchor, |cap: &regex::Captures| {
                        format!("{}{part}", &cap[1])
                    });
                    dir.join(format!("{stem}.{extension}"))
                } else {
                    dir.join(name)
                }
            }
        } else {
            let matched: Vec<_> = video_targets
                .iter()
                .filter(|v| {
                    selected_video
                        .as_ref()
                        .is_none_or(|selected| *v == selected)
                })
                .filter(|v| {
                    parse_code(&v.to_string_lossy()).is_some_and(|v| {
                        v.part == c.part || (v.part == Some(1) && c.part.is_none())
                    })
                })
                .collect();
            if matched.len() != 1 {
                p.conflicts
                    .push(format!("字幕无法唯一匹配分片：{}", input.display()));
                continue;
            }
            let filename = input.file_name().unwrap().to_string_lossy();
            let stem = input.file_stem().unwrap().to_string_lossy();
            let tags = stem.split('.').skip(1).collect::<Vec<_>>().join(".");
            let suffix = if tags.is_empty() {
                format!(".{extension}")
            } else {
                format!(".{tags}.{extension}")
            };
            let _ = filename;
            dir.join(format!(
                "{}{}",
                matched[0].file_stem().unwrap().to_string_lossy(),
                suffix
            ))
        };
        if to.file_name().is_some_and(|n|n.to_string_lossy().len()>240) {p.conflicts.push(format!("追加目标文件名超过 240 UTF-8 字节：{}",to.display()));continue;}
        moves.push((input.clone(), to.clone()));
        if VIDEO_EXT.contains(&extension.as_str()) {
            if !video_targets.contains(&to) {
                video_targets.push(to.clone());
            }
            for sub in crate::subtitle::siblings(input, &c) {
                if claimed.insert(sub.from.clone()) {
                    let target = to.with_file_name(format!(
                        "{}{}",
                        to.file_stem().unwrap().to_string_lossy(),
                        sub.suffix
                    ));
                    if target.file_name().is_some_and(|n|n.to_string_lossy().len()>240) {p.conflicts.push(format!("字幕目标文件名超过 240 UTF-8 字节：{}",target.display()));continue;}
                    moves.push((sub.from, target));
                }
            }
        }
    }
    let mut targets = HashSet::new();
    let mut sources = HashSet::new();
    for (from, to) in moves {
        if !sources.insert(from.clone()) {
            continue;
        }
        regular(&from)?;
        let incoming_proof=match proofs.remove(&from){Some(proof)=>proof,None=>fingerprint_cancellable(&from,cancel)?};
        p.guards.push((from.clone(),incoming_proof.clone()));
        confined_path(&p.root, to.parent().ok_or_else(||error("目标无父目录"))?)?;
        if crate::organize::occupied(&to) {
            let (_,existing_proof)=verified.get(&to).ok_or_else(||error("目标不是已验证媒体文件"))?;
            if incoming_proof.rsplit(':').next()==existing_proof.rsplit(':').next() {
                p.skipped
                    .push(format!("相同文件已存在，保留源：{}", from.display()));
                continue;
            }
            p.conflicts
                .push(format!("目标已存在且内容不同：{}", to.display()));
            continue;
        }
        if !targets.insert(to.clone()) {
            p.conflicts.push(format!("批次目标重复：{}", to.display()));
            continue;
        }
        p.ops.push(Op::MoveFile { from, to });
    }
    Ok(p)
}
impl AppendPreview {
    pub fn set_mode(&mut self,mode:crate::placement::OrganizeMode)->io::Result<()> {for op in &mut self.ops {if let Some(from)=op.source(){let expected=self.guards.iter().find(|(path,_)|path==from).map(|(_,proof)|proof.clone());*op=mode.retained_operation(from.to_path_buf(),op.target().to_path_buf(),expected);}} Ok(())}

    pub fn validate(&self) -> io::Result<()> {
        confined_path(&self.root, &self.target_dir)?;
        if entries(&self.target_dir)? != self.directory {
            return Err(error("影片目录已变化，请重新预览"));
        }
        self.validate_files()
    }
    pub fn validate_files(&self) -> io::Result<()> {
        self.validate_files_cancellable(&CancellationToken::new())
    }
    pub(crate) fn accept_manifest_updates(&mut self,journal:&Journal)->io::Result<()> {
        for (path,fingerprint) in &mut self.guards {if path.file_name().is_some_and(|n|n==crate::organize::SIDECAR){
            let normalized=confined_path(&journal.root,path)?;
            let changes=journal.replacements.iter().filter(|r|r.path==normalized).collect::<Vec<_>>();
            if let (Some(first),Some(last))=(changes.first(),changes.last()) {
                if first.before.as_ref().is_some_and(|digest|fingerprint.ends_with(digest)) && crate::journal::replacement::current(path)?.as_ref()==Some(&last.after){*fingerprint=fingerprint_cancellable(path,&CancellationToken::new())?;}
            }
        }}Ok(())
    }
    pub fn validate_files_cancellable(&self, cancel: &CancellationToken) -> io::Result<()> {
        check_cancel(cancel)?;
        confined_path(&self.root, &self.target_dir)?;
        let metadata=read_meta(&self.target_dir)?;
        if metadata.code != self.code {return Err(error("影片编号已变化"));}
        for placement in &metadata.placements {check_cancel(cancel)?;if let Some(anchor)=&placement.anchor {
            let path=self.target_dir.join(anchor.file_name().ok_or_else(||error("媒体证明文件名无效"))?);
            if !crate::placement::is_owned_anchor(&self.root,&self.target_dir,&path,&metadata)?{return Err(error("媒体证明已变化，请重新预览"))}
        }}
        for (path, hash) in &self.guards {
            regular(path)?;
            if fingerprint_cancellable(path, cancel)? != *hash {
                return Err(error("文件已变化，请重新预览"));
            }
        }
        if !self.conflicts.is_empty() {
            return Err(error(self.conflicts.join("; ")));
        }
        Ok(())
    }
}
pub fn apply(p: &AppendPreview, cancel: &CancellationToken) -> io::Result<AppendResult> {
    let validation = if entries(&p.target_dir)? != p.directory {
        Err(error("影片目录已变化，请重新预览"))
    } else {
        p.validate_files_cancellable(cancel)
    };
    if let Err(e) = validation {
        if e.kind() == io::ErrorKind::Interrupted {
            return Ok(AppendResult {
                journal_id: String::new(),
                completed: 0,
                skipped: vec![],
                failed: vec![],
                cancelled: true,
            });
        }
        return Err(e);
    }
    let mut j = Journal::new(&p.root);
    j.items.push(JournalItem {
        code: p.code.clone(),
        dir: p.target_dir.clone(),
    });
    let ex = crate::placement::execute(
        &mut j, &Plan { ops: p.ops.clone() },
        &mut |_| Err(error("追加不下载图片")),
        &|| cancel.is_cancelled(),
    );
    if ex.error.is_none() && !cancel.is_cancelled() {
        j.status = JournalStatus::Applied;
    }
    j.save()?;
    let mut failed = ex
        .error
        .map(|e| e.to_string())
        .into_iter()
        .collect::<Vec<_>>();
    if let Err(e) = if p.root.is_dir(){Library::open(&p.root).and_then(|l| l.reindex_dir_cancellable(&p.target_dir,cancel))}else{Err(io::Error::other("库目录不可用，保留缓存").into())} {
        if !cancel.is_cancelled(){failed.push(format!("索引更新失败：{e}"))}
    }
    Ok(AppendResult {
        journal_id: j.id,
        completed: ex.done.len(),
        skipped: p.skipped.clone(),
        failed,
        cancelled: cancel.is_cancelled(),
    })
}

pub fn existing_metadata(
    root: &Path,
    codes: &[String],
) -> std::collections::HashMap<String, Metadata> {
    existing_metadata_cancellable(root,codes,&CancellationToken::new()).unwrap_or_default()
}

pub fn existing_metadata_cancellable(root:&Path,codes:&[String],cancel:&CancellationToken)->io::Result<HashMap<String,Metadata>> {
    check_cancel(cancel)?;let mut out=HashMap::new();
    if codes.is_empty(){return Ok(out);}
    let snapshot=match Targets::capture(root,codes.iter().map(String::as_str),cancel){Ok(snapshot)=>snapshot,Err(error)if cancel.is_cancelled()=>return Err(error),Err(_)=>return Ok(out)};
    for code in codes {
        check_cancel(cancel)?;
        if let Ok(directories)=snapshot.get(code,cancel){if directories.len()==1{if let Ok(metadata)=read_meta(&directories[0]){if same_code(&metadata.code,code){out.insert(code.clone(),metadata);}}}}
    }
    check_cancel(cancel)?;if snapshot.verify().is_err(){out.clear();}
    Ok(out)
}

pub(crate) fn check_cancel(cancel: &CancellationToken) -> io::Result<()> {
    if cancel.is_cancelled() {
        Err(io::Error::new(io::ErrorKind::Interrupted, "任务已取消"))
    } else {
        Ok(())
    }
}

/// 版本标签:清晰度、中字、破解/流出;内容类型推出的有码/无码不算(ADR-0008)。
fn version_label(c: &Code) -> String {
    let mut parts: Vec<&str> = c.definition.as_deref().into_iter().collect();
    if c.subtitled {
        parts.push("中字");
    }
    match c.mosaic {
        Some(crate::code::Mosaic::Cracked) => parts.push("破解"),
        Some(crate::code::Mosaic::Leaked) => parts.push("流出"),
        _ => {}
    }
    parts.join(" ")
}

/// `<已有视频主体名> - <标签>.<ext>`,符合 Jellyfin 多版本命名;标签为空或已被占用时依次用 v2、v3…
fn version_path(anchor: &Path, label: &str, extension: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let stem = anchor.file_stem().unwrap_or_default().to_string_lossy();
    let at = |suffix: &str| anchor.with_file_name(format!("{stem} - {suffix}.{extension}"));
    if !label.is_empty() && !taken(&at(label)) {
        return at(label);
    }
    (2..).map(|n| at(&format!("v{n}"))).find(|p| !taken(p)).unwrap()
}

fn same_code(a: &str, b: &str) -> bool {
    let norm = |s: &str| parse_code(s).map(|c| c.id).unwrap_or_else(|| s.to_owned());
    norm(a) == norm(b)
}

pub fn target_videos(root: &Path, code: &str, target: &Path) -> io::Result<Vec<PathBuf>> {
    target_videos_cancellable(root, code, target, &CancellationToken::new())
}
pub fn target_videos_cancellable(
    root: &Path,
    code: &str,
    target: &Path,
    cancel: &CancellationToken,
) -> io::Result<Vec<PathBuf>> {
    let target = confined_path(root, target)?;
    if !targets_cancellable(root, code, cancel)?.contains(&target) {
        return Err(error("目标目录与影片不匹配"));
    }
    let meta=read_meta(&target)?;
    let mut out = vec![];
    for path in entries(&target)? {
        check_cancel(cancel)?;
        if VIDEO_EXT.contains(&ext(&path).as_str()) {
            crate::placement::resolve_media_cancellable(root,&target,&path,&meta,cancel)?;
            out.push(path)
        }
    }
    Ok(out)
}
