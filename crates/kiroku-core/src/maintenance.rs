//! Explicit retention preview. Fail closed if current references cannot be established.
use crate::{
    append,
    journal::{Journal, JournalStatus},
    organize::Op,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionPolicy {
    pub history_days: u32,
    pub log_days: u32,
}
impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            history_days: 90,
            log_days: 90,
        }
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceCandidate {
    pub id: String,
    pub kind: String,
    pub paths: Vec<PathBuf>,
    pub bytes: u64,
    pub description: String,
    #[serde(skip)]
    fingerprints: Vec<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenancePreview {
    pub root: PathBuf,
    pub log_dir: PathBuf,
    pub policy: RetentionPolicy,
    pub candidates: Vec<MaintenanceCandidate>,
    pub protected: Vec<String>,
    pub candidate_bytes: u64,
    pub resource_bytes: u64,
    pub protected_bytes: u64,
    pub scanned_at: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceResult {
    pub removed: usize,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub cancelled: bool,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn old(path: &Path, days: u32) -> bool {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .is_some_and(|t| now().saturating_sub(t.as_secs()) >= u64::from(days) * 86400)
}
fn original(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|s| s.starts_with(".kiroku-original-") && s.ends_with(".png"))
}
fn original_reference(
    root: &Path,
    dir: &Path,
    bytes: &[u8],
    live: &mut HashSet<PathBuf>,
) -> io::Result<()> {
    let meta: crate::metadata::Metadata =
        serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if meta.code.trim().is_empty() {
        return Err(io::Error::other("sidecar 缺少影片编号"));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if let Some(s) = value["artwork"]["original"].as_str() {
        if Path::new(s).components().count() != 1 {
            return Err(io::Error::other("原图引用越界"));
        }
        live.insert(append::confined_path(root, &dir.join(s))?);
    }
    Ok(())
}
fn candidate(
    root: &Path,
    id: String,
    kind: &str,
    mut paths: Vec<PathBuf>,
    description: String,
    cancel: &CancellationToken,
) -> io::Result<MaintenanceCandidate> {
    paths.sort();
    paths.dedup();
    let mut fingerprints = vec![];
    let mut bytes = 0;
    for p in &paths {
        append::confined_path(root, p)?;
        append::regular(p)?;
        let m=fs::symlink_metadata(p)?;
        #[cfg(unix)] {use std::os::unix::fs::MetadataExt;if m.nlink()==1{bytes+=m.len();}}
        #[cfg(not(unix))] {bytes+=m.len();}
        fingerprints.push(append::fingerprint_cancellable(p, cancel)?)
    }
    Ok(MaintenanceCandidate {
        id,
        kind: kind.into(),
        paths,
        bytes,
        description,
        fingerprints,
    })
}
pub fn preview(
    root: &Path,
    log_dir: &Path,
    policy: &RetentionPolicy,
) -> io::Result<MaintenancePreview> {
    preview_cancellable(root, log_dir, policy, &CancellationToken::new())
}
pub fn preview_cancellable(
    root: &Path,
    log_dir: &Path,
    policy: &RetentionPolicy,
    cancel: &CancellationToken,
) -> io::Result<MaintenancePreview> {
    append::check_cancel(cancel)?;
    let root = root.canonicalize()?;
    fs::read_dir(&root)?;
    let log_dir = if log_dir.exists() {
        log_dir.canonicalize()?
    } else {
        std::path::absolute(log_dir)?
    };
    let mut p = MaintenancePreview {
        root: root.clone(),
        log_dir: log_dir.clone(),
        policy: policy.clone(),
        candidates: vec![],
        protected: vec![],
        candidate_bytes: 0,
        resource_bytes: 0,
        protected_bytes: 0,
        scanned_at: now(),
    };
    let mut live = HashSet::new();
    let mut originals = vec![];
    let mut metadata_texts = vec![];
    let mut resource_paths = HashSet::new();
    let mut blocked = false;
    for e in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".kiroku")
    {
        append::check_cancel(cancel)?;
        let e = match e {
            Ok(e) => e,
            Err(e) => {
                blocked = true;
                p.protected
                    .push(format!("库目录无法完整读取，保留资源：{e}"));
                continue;
            }
        };
        if e.file_type().is_symlink() {
            blocked = true;
            p.protected
                .push(format!("库内有符号链接，保留资源：{}", e.path().display()));
            continue;
        }
        if !e.file_type().is_file() {
            continue;
        }
        if e.file_name() == "kiroku.json" {
            let read = append::confined_path(&root, e.path())
                .and_then(|_| fs::read(e.path()))
                .and_then(|b| {
                    metadata_texts.push(String::from_utf8_lossy(&b).into_owned());
                    let meta:crate::Metadata=serde_json::from_slice(&b).map_err(io::Error::other)?;
                    for placement in &meta.placements {if let Some(anchor)=&placement.anchor {let parent=anchor.parent().ok_or_else(||io::Error::other("媒体证明路径无效"))?;append::confined_path(&root,parent)?;if !anchor.starts_with(&root){return Err(io::Error::other("媒体证明不在库内"))}live.insert(anchor.clone());}}
                    original_reference(&root, e.path().parent().unwrap(), &b, &mut live)
                });
            if let Err(err) = read {
                blocked = true;
                p.protected.push(format!(
                    "sidecar 引用无法确认：{}：{err}",
                    e.path().display()
                ));
            }
        }
        if e.path()
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("nfo"))
        {
            match fs::read_to_string(e.path()) {
                Ok(text) if crate::health::valid_xml(&text) => metadata_texts.push(text),
                _ => {
                    blocked = true;
                    p.protected.push(format!(
                        "NFO 引用无法确认，保留资源：{}",
                        e.path().display()
                    ));
                }
            }
        }
        if original(e.path()) {
            resource_paths.insert(e.path().to_path_buf());
            originals.push(e.path().to_path_buf());
        }
    }
    let mut journals = HashMap::new();
    let storage = root.join(".kiroku");
    if storage.exists() {
        append::confined_path(&root, &storage)?;
        for e in fs::read_dir(&storage)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("journal-") && name.ends_with(".json") {
                let result = append::confined_path(&root, &e.path())
                    .and_then(|_| append::regular(&e.path()))
                    .and_then(|_| Journal::load(&e.path()));
                match result {
                    Ok(j)
                        if (1..=4).contains(&j.version)
                            && (j.version < 4 || j.validate_recorded_targets().is_ok())
                            && !j.id.is_empty()
                            && j.id.bytes().all(|b| b.is_ascii_digit())
                            && name == format!("journal-{}.json", j.id)
                            && j.root.canonicalize().ok() == Some(root.clone()) =>
                    {
                        journals.insert(j.id.clone(), (e.path(), j));
                    }
                    _ => {
                        blocked = true;
                        p.protected
                            .push(format!("损坏或未知日志，保留库内资源：{name}"));
                    }
                }
            }
        }
    }
    let mut required_ids = HashSet::new();
    for (_, j) in journals.values() {
        for r in &j.replacements {
            if let Some(owner) = &r.before_owner {
                required_ids.insert(owner.journal_id.clone());
            }
        }
    }
    let mut retired = vec![];
    for (path, j) in journals.values() {
        resource_paths.insert(path.clone());
        append::check_cancel(cancel)?;
        let complete = j.status == JournalStatus::RolledBack
            && j.pending.is_none()
            && j.ops
                .iter()
                .enumerate()
                .all(|(i, _)| j.rolled_back_ops.contains(&i))
            && j.replacements.iter().all(|r| r.undone);
        let eligible = complete
            && !required_ids.contains(&j.id)
            && now().saturating_sub(j.ts / 1000) >= u64::from(policy.history_days) * 86400;
        let mut assets = vec![];
        for (i, proof) in &j.proofs {
            let expected = j
                .ops
                .get(*i)
                .and_then(|o| o.target().parent())
                .map(|parent| parent.join(format!(".kiroku-{}-{}.tmp", j.id, i)));
            let canonical_expected = expected
                .as_ref()
                .map(|p| crate::journal::replacement::recorded_path(&root, &j.root, p))
                .transpose();
            let stage = crate::journal::replacement::recorded_path(&root, &j.root, &proof.stage);
            match (canonical_expected, stage) {
                (Ok(Some(expected)), Ok(stage)) if stage == expected => assets.push(stage),
                _ => {
                    blocked = true;
                    p.protected.push(format!("日志暂存路径不可信：{}", j.id));
                }
            }
        }
        if let Some(pending) = &j.pending {
            match crate::journal::replacement::recorded_path(&root, &j.root, &pending.proof.stage) {
                Ok(stage) => {
                    live.insert(stage);
                }
                Err(_) => blocked = true,
            }
        }
        for (index, r) in j.replacements.iter().enumerate() {
            let base = storage.join(format!("revision-{}", j.id));
            if r.stage != base.join(format!("{index}.after"))
                && crate::journal::replacement::recorded_path(&root, &j.root, &r.stage).ok()
                    != Some(base.join(format!("{index}.after")))
            {
                blocked = true;
                p.protected.push(format!("备份位置不可信：{}", j.id));
                continue;
            }
            assets.push(crate::journal::replacement::recorded_path(
                &root, &j.root, &r.stage,
            )?);
            if !eligible && !r.undone && original(&r.path) {
                live.insert(crate::journal::replacement::recorded_path(
                    &root, &j.root, &r.path,
                )?);
            }
            if let Some(backup) = &r.backup {
                let backup = crate::journal::replacement::recorded_path(&root, &j.root, backup)?;
                if backup != base.join(format!("{index}.before")) {
                    blocked = true;
                    continue;
                }
                if !eligible
                    && r.path.file_name().is_some_and(|n| n == "kiroku.json")
                    && backup.exists()
                {
                    if fs::read(&backup)
                        .and_then(|bytes| {
                            original_reference(
                                &root,
                                crate::journal::replacement::recorded_path(
                                    &root, &j.root, &r.path,
                                )?
                                .parent()
                                .unwrap(),
                                &bytes,
                                &mut live,
                            )
                        })
                        .is_err()
                    {
                        blocked = true;
                        p.protected
                            .push(format!("历史 sidecar 引用无法确认：{}", j.id));
                    }
                }
                if !eligible
                    && r.path
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.eq_ignore_ascii_case("nfo"))
                    && backup.exists()
                {
                    match fs::read_to_string(&backup) {
                        Ok(text) if crate::health::valid_xml(&text) => metadata_texts.push(text),
                        _ => {
                            blocked = true;
                            p.protected.push(format!("历史 NFO 引用无法确认：{}", j.id));
                        }
                    }
                }
                assets.push(backup);
            }
        }
        resource_paths.extend(assets.iter().cloned());
        if eligible {
            assets.retain(|p| p.exists());
            assets.push(path.clone());
            retired.push((j.id.clone(), assets));
        } else {
            p.protected.push(format!(
                "保留历史 {}：仍有有效撤销、依赖或未到保留期限",
                j.id
            ));
            live.extend(assets);
            for op in &j.ops {
                if let Op::Download { path, .. } = op {
                    if original(path) {
                        live.insert(crate::journal::replacement::recorded_path(
                            &root, &j.root, path,
                        )?);
                    }
                }
            }
        }
    }
    if !blocked {
        for path in originals {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if metadata_texts.iter().any(|text| text.contains(name)) {
                    live.insert(path.clone());
                }
            }
            if live.contains(&path) {
                p.protected
                    .push(format!("保留原图（影片或历史引用）：{}", path.display()));
                continue;
            }
            if !old(&path, policy.history_days) {
                p.protected
                    .push(format!("保留原图（未到保留期限）：{}", path.display()));
                continue;
            }
            let id = format!("original:{}", path.strip_prefix(&root).unwrap().display());
            p.candidates.push(candidate(
                &root,
                id,
                "original",
                vec![path],
                "无引用且已到保留期限的原图".into(),
                cancel,
            )?);
        }
        for (id, assets) in retired {
            if assets.iter().any(|a| live.contains(a)) {
                p.protected.push(format!("保留历史 {id}：文件仍有引用"));
                continue;
            }
            p.candidates.push(candidate(
                &root,
                format!("history:{id}"),
                "history",
                assets,
                "已完整撤销且无依赖的旧记录及备份".into(),
                cancel,
            )?);
        }
    }
    if log_dir.exists() {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        for e in fs::read_dir(&log_dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            let date = name
                .strip_prefix("kiroku-")
                .and_then(|s| s.strip_suffix(".log"));
            if date.is_some_and(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
                && e.file_type()?.is_file()
            {
                resource_paths.insert(e.path());
            }
            if date.is_some_and(|s| {
                s != today && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
            }) && old(&e.path(), policy.log_days)
                && e.file_type()?.is_file()
            {
                p.candidates.push(candidate(
                    &log_dir,
                    format!("log:{name}"),
                    "log",
                    vec![e.path()],
                    "过期的非当日日志".into(),
                    cancel,
                )?);
            }
        }
    }
    p.candidate_bytes = p.candidates.iter().map(|c| c.bytes).sum();
    p.resource_bytes = resource_paths
        .iter()
        .filter_map(|path| {
            fs::symlink_metadata(path)
                .ok()
                .filter(|m| m.is_file())
                .map(|m| m.len())
        })
        .sum();
    p.protected_bytes = p.resource_bytes.saturating_sub(p.candidate_bytes);
    Ok(p)
}
pub fn apply(
    p: &MaintenancePreview,
    selected: &[String],
    trash: &dyn Fn(&Path) -> io::Result<()>,
    cancel: &CancellationToken,
) -> io::Result<MaintenanceResult> {
    let mut r = MaintenanceResult {
        removed: 0,
        skipped: vec![],
        failed: vec![],
        cancelled: false,
    };
    for id in selected {
        if cancel.is_cancelled() {
            r.cancelled = true;
            break;
        }
        let Some(c) = p.candidates.iter().find(|c| &c.id == id) else {
            r.skipped.push(format!("未知候选：{id}"));
            continue;
        };
        let fresh = match preview_cancellable(&p.root, &p.log_dir, &p.policy, cancel) {
            Ok(p) => p,
            Err(e) => {
                if e.kind() == io::ErrorKind::Interrupted {
                    r.cancelled = true
                } else {
                    r.failed.push(e.to_string())
                }
                break;
            }
        };
        let Some(current) = fresh
            .candidates
            .iter()
            .find(|a| a.id == c.id && a.paths == c.paths && a.fingerprints == c.fingerprints)
        else {
            r.skipped.push(format!("引用或文件已变化，保留：{id}"));
            continue;
        };
        let mut paths = current.paths.clone();
        paths.sort_by_key(|p| {
            p.file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with("journal-"))
        });
        for path in paths {
            if cancel.is_cancelled() {
                r.cancelled = true;
                break;
            }
            let base = if c.kind == "log" { &p.log_dir } else { &p.root };
            if let Err(e)=append::confined_path(base,&path).and_then(|_|append::regular(&path)){r.failed.push(format!("{}：{e}",path.display()));break}
            let boundary = match preview_cancellable(&p.root, &p.log_dir, &p.policy, cancel) {
                Ok(p) => p,
                Err(e) => {
                    if e.kind() == io::ErrorKind::Interrupted {
                        r.cancelled = true
                    } else {
                        r.failed.push(e.to_string())
                    }
                    break;
                }
            };
            if !boundary
                .candidates
                .iter()
                .any(|a| a.id == c.id && a.paths.contains(&path))
            {
                r.skipped
                    .push(format!("依赖已变化，保留：{}", path.display()));
                break;
            }
            let index = c.paths.iter().position(|p| *p == path).unwrap();
            let fingerprint=match append::fingerprint_cancellable(&path,cancel){Ok(h)=>h,Err(e)=>{if e.kind()==io::ErrorKind::Interrupted{r.cancelled=true}else{r.failed.push(format!("{}：{e}",path.display()))}break}};
            if fingerprint != c.fingerprints[index] {
                r.skipped.push(format!("文件已变化：{}", path.display()));
                break;
            }
            if cancel.is_cancelled() {
                r.cancelled = true;
                break;
            }
            match trash(&path) {
                Ok(()) => r.removed += 1,
                Err(e) => {
                    r.failed.push(format!("{}：{e}", path.display()));
                    break;
                }
            }
        }
    }
    Ok(r)
}
