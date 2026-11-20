//! A source-folder crop has its own on-disk revision journal. These local
//! references let the originating library show and undo it without moving media.
use kiroku_core::{
    append, assets,
    journal::{self, HistoryFilter, HistoryPage, HistorySummary, Journal},
    library::MovieCard,
    revision,
    settings::Settings,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

const HISTORY_SUFFIX: &str = "~source";

pub fn history_id(id: &str) -> String {
    format!("{id}{HISTORY_SUFFIX}")
}
fn key(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn store(active: &Path) -> io::Result<PathBuf> {
    let config = Settings::default_path();
    let active = active.canonicalize()?;
    Ok(config
        .with_file_name("linked-artwork-v1")
        .join(key(active.as_os_str().as_encoded_bytes())))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub root: PathBuf,
    pub relative: PathBuf,
    pub code: String,
}
impl Link {
    pub fn dir(&self) -> PathBuf {
        self.root.join(&self.relative)
    }
    fn allowed(&self, sources: &[String]) -> bool {
        !self.relative.as_os_str().is_empty()
            && self
                .relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
            && sources.iter().any(|s| {
                Path::new(s) == self.root
                    || Path::new(s).canonicalize().is_ok_and(|p| p == self.root)
            })
    }
}

fn atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("无效的预览目录"))?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
fn read<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn movie_entry(store: &Path, code: &str) -> PathBuf {
    store
        .join("movies")
        .join(format!("{}.json", key(code.as_bytes())))
}

pub fn register(active: &Path, source: &Path, dir: &Path, code: &str) -> Result<(), String> {
    let action = || -> io::Result<()> {
        let confined = append::confined_path(source, dir)?;
        let rel = confined
            .strip_prefix(source)
            .map_err(io::Error::other)?
            .to_path_buf();
        let link = Link {
            root: source.to_owned(),
            relative: rel,
            code: code.to_owned(),
        };
        let location = store(active)?
            .join("bindings")
            .join(format!("{}.json", key(code.as_bytes())));
        atomic(&location, &serde_json::to_vec(&link)?)
    };
    action().map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize)]
struct Published {
    link: Link,
    version: String,
    ext: String,
}

pub fn note_journal(
    active: &Path,
    source: &Path,
    dir: &Path,
    code: &str,
    id: &str,
) -> Result<(), String> {
    let action = || -> io::Result<()> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(io::Error::other("无效的裁剪日志编号"));
        }
        let base = store(active)?;
        let binding = base
            .join("bindings")
            .join(format!("{}.json", key(code.as_bytes())));
        let link: Link = read(&binding)?.ok_or_else(|| io::Error::other("来源裁剪记录不存在"))?;
        if link.root != source || link.dir() != dir || link.code != code {
            return Err(io::Error::other("来源裁剪目录已变化"));
        }
        atomic(
            &base.join("journals").join(format!("{id}.json")),
            &serde_json::to_vec(&link)?,
        )?;
        publish(&base, link)
    };
    action().map_err(|e| e.to_string())
}

fn publish(base: &Path, link: Link) -> io::Result<()> {
    let dir = append::confined_path(&link.root, &link.dir())?;
    let metadata = append::read_meta(&dir)?;
    if metadata.code != link.code {
        return Err(io::Error::other("来源影片编号已变化"));
    }
    let path = assets::Paths::for_movie(&dir, &metadata)?.poster;
    let (hash, bytes) = revision::snapshot_file(&link.root, &path)?;
    let entry = movie_entry(base, &link.code);
    let (Some(version), Some(bytes)) = (hash, bytes) else {
        return match fs::remove_file(entry) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        };
    };
    let ext = if bytes.starts_with(&[255, 216, 255]) {
        "jpg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "webp"
    } else {
        return Err(io::Error::other("无法识别来源海报格式"));
    };
    let copy = base.join("images").join(format!("{version}.{ext}"));
    atomic(&copy, &bytes)?;
    atomic(
        &entry,
        &serde_json::to_vec(&Published {
            link,
            version,
            ext: ext.to_string(),
        })?,
    )
}

/// Only local previews created by completed source-folder revisions are
/// projected. An available original library directory always takes precedence.
pub fn project(
    app: &AppHandle,
    active: &Path,
    sources: &[String],
    cards: &mut [MovieCard],
) -> io::Result<()> {
    let base = store(active)?;
    if !base.join("movies").try_exists()? {
        return Ok(());
    }
    for card in cards {
        let Some(p) = read::<Published>(&movie_entry(&base, &card.code))? else {
            continue;
        };
        if p.link.code != card.code
            || !p.link.allowed(sources)
            || p.version.len() != 64
            || !p.version.bytes().all(|b| b.is_ascii_hexdigit())
            || !["jpg", "png", "webp"].contains(&p.ext.as_str())
        {
            continue;
        }
        let original = active.join(&p.link.relative);
        if original.try_exists().unwrap_or(true) {
            continue;
        }
        let path = base.join("images").join(format!("{}.{}", p.version, p.ext));
        if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        app.asset_protocol_scope()
            .allow_file(&path)
            .map_err(io::Error::other)?;
        card.poster = Some(path.to_string_lossy().into_owned());
        card.poster_version = Some(p.version);
    }
    Ok(())
}
pub fn history_target(
    active: &Path,
    sources: &[String],
    public_id: &str,
) -> io::Result<Option<(Link, String)>> {
    let Some(id) = public_id.strip_suffix(HISTORY_SUFFIX) else {
        return Ok(None);
    };
    if id.is_empty() || id.len() > 32 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(io::Error::other("无效的来源日志编号"));
    }
    let link: Link = read(&store(active)?.join("journals").join(format!("{id}.json")))?
        .ok_or_else(|| io::Error::other("找不到来源裁剪记录"))?;
    if !link.allowed(sources) {
        return Err(io::Error::other("影片的来源目录已从设置中移除"));
    }
    Ok(Some((link, id.to_owned())))
}
pub fn checked_journal(link: &Link, id: &str, cancel: &CancellationToken) -> io::Result<Journal> {
    let path = append::confined_path(
        &link.root,
        &journal::dir(&link.root).join(format!("journal-{id}.json")),
    )?;
    let j = Journal::load_cancellable(&path, cancel)?;
    if j.id != id
        || j.items.len() != 1
        || j.items[0].code != link.code
        || j.items[0].dir != link.dir()
    {
        return Err(io::Error::other("来源裁剪日志与影片不符"));
    }
    Ok(j)
}
pub fn merge_history(
    active: &Path,
    sources: &[String],
    mut page: HistoryPage,
    before: Option<&str>,
    limit: usize,
    filter: &HistoryFilter,
    cancel: &CancellationToken,
) -> io::Result<HistoryPage> {
    let entries = match fs::read_dir(store(active)?.join("journals")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(page),
        Err(e) => return Err(e),
    };
    for entry in entries {
        if cancel.is_cancelled() {
            return Err(io::Error::other("历史读取已取消"));
        }
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Some(link) = read::<Link>(&entry.path())? else {
            continue;
        };
        if !link.allowed(sources) {
            continue;
        }
        let mut summary = match checked_journal(&link, id, cancel) {
            Ok(j) => HistorySummary::of(j),
            Err(e) => HistorySummary {
                id: id.to_owned(),
                ts: id.parse().unwrap_or_default(),
                status: journal::JournalStatus::Partial,
                item_count: 1,
                op_count: 0,
                codes: vec![link.code.clone()],
                trashed: 0,
                pending_operations: 0,
                error: Some(e.to_string()),
            },
        };
        summary.id = history_id(&summary.id);
        if filter.matches(&summary, &link.code.to_lowercase()) {
            page.total += 1;
            if before.is_none_or(|b| summary.id.as_str() < b) {
                page.items.push(summary)
            }
        }
    }
    page.items.sort_by(|a, b| b.id.cmp(&a.id));
    let more = page.items.len() > limit || page.next.is_some();
    page.items.truncate(limit);
    page.next = if more {
        page.items.last().map(|s| s.id.clone())
    } else {
        None
    };
    Ok(page)
}
