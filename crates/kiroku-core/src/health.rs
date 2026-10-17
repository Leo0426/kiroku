//! Read-only disk and SQLite cache diagnostics. A report never repairs or deletes.
use crate::{append, library::Library, pipeline::VIDEO_EXT};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthIssue {
    pub kind: String,
    pub severity: String,
    pub dir: PathBuf,
    pub code: Option<String>,
    pub path: PathBuf,
    pub message: String,
    pub action: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub root: PathBuf,
    pub scanned_at: u64,
    pub scanned: usize,
    pub cancelled: bool,
    pub issues: Vec<HealthIssue>,
}
fn add(
    r: &mut HealthReport,
    kind: &str,
    dir: &Path,
    code: Option<&str>,
    path: &Path,
    message: impl Into<String>,
    action: &str,
) {
    r.issues.push(HealthIssue {
        kind: kind.into(),
        severity: if kind == "missingArtwork" || kind == "missingIndex" {
            "warning"
        } else {
            "error"
        }
        .into(),
        dir: dir.into(),
        code: code.map(String::from),
        path: path.into(),
        message: message.into(),
        action: action.into(),
    })
}
fn ext(p: &Path) -> String {
    p.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase()
}
pub(crate) fn valid_xml(xml: &str) -> bool {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut stack = vec![];
    let mut movie = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                if stack.is_empty() {
                    if movie || e.name().as_ref() != b"movie" {
                        return false;
                    }
                    movie = true
                }
                stack.push(e.name().as_ref().to_vec())
            }
            Ok(Event::End(e)) => {
                if stack.pop().as_deref() != Some(e.name().as_ref()) {
                    return false;
                }
            }
            Ok(Event::Empty(e)) if stack.is_empty() => {
                if movie || e.name().as_ref() != b"movie" {
                    return false;
                }
                movie = true
            }
            Ok(Event::Eof) => return movie && stack.is_empty(),
            Ok(Event::DocType(_)) => return false,
            Err(_) => return false,
            _ => {}
        }
    }
}
pub fn scan(
    root: &Path,
    cancel: &CancellationToken,
    on_progress: &dyn Fn(usize),
) -> io::Result<HealthReport> {
    let root = root.canonicalize()?;
    fs::read_dir(&root)?;
    let mut r = HealthReport {
        root: root.clone(),
        scanned_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        scanned: 0,
        cancelled: false,
        issues: vec![],
    };
    let mut disk: HashMap<PathBuf, (String, HashSet<PathBuf>, String)> = HashMap::new();
    for e in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".kiroku")
    {
        if cancel.is_cancelled() {
            r.cancelled = true;
            return Ok(r);
        }
        let e = match e {
            Ok(e) => e,
            Err(e) => {
                let path = e.path().unwrap_or(&root).to_path_buf();
                add(
                    &mut r,
                    "unreadable",
                    &path,
                    None,
                    &path,
                    e.to_string(),
                    "reveal",
                );
                continue;
            }
        };
        if !e.file_type().is_dir() {
            continue;
        }
        let dir = e.path();
        let entries = match fs::read_dir(dir).and_then(|rd| {
            rd.map(|e| e.map(|e| e.path()))
                .collect::<Result<Vec<_>, _>>()
        }) {
            Ok(p) => p,
            Err(e) => {
                add(
                    &mut r,
                    "unreadable",
                    dir,
                    None,
                    dir,
                    e.to_string(),
                    "reveal",
                );
                continue;
            }
        };
        let videos: HashSet<_> = entries
            .iter()
            .filter(|p| VIDEO_EXT.contains(&ext(p).as_str()))
            .cloned()
            .collect();
        let side = dir.join("kiroku.json");
        let nfo = entries.iter().find(|p| p.extension().is_some_and(|s| s.eq_ignore_ascii_case("nfo"))).cloned().unwrap_or_else(|| dir.join("movie.nfo"));
        if videos.is_empty() && !side.exists() && !nfo.exists() {
            continue;
        }
        r.scanned += 1;
        on_progress(r.scanned);
        let meta = append::read_meta(dir).ok();
        let code = meta.as_ref().map(|m| m.code.clone());
        if side.exists() {
            if append::confined_path(&root, &side).is_err()
                || append::regular(&side).is_err()
                || meta.is_none()
            {
                add(
                    &mut r,
                    "invalidSidecar",
                    dir,
                    code.as_deref(),
                    &side,
                    "sidecar 无法读取或存在不安全路径",
                    "reveal",
                )
            }
            if append::confined_path(&root, &side).is_ok() && append::regular(&side).is_ok() {
                if let Ok(bytes) = fs::read(&side) {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(original) = v["artwork"]["original"].as_str() {
                            let path = dir.join(original);
                            let valid = Path::new(original).components().count() == 1
                                && original.starts_with(".kiroku-original-")
                                && original.ends_with(".png")
                                && append::confined_path(&root, &path).is_ok()
                                && append::regular(&path).is_ok()
                                && image::open(&path).is_ok();
                            if !valid {
                                add(
                                    &mut r,
                                    "invalidOriginal",
                                    dir,
                                    code.as_deref(),
                                    &path,
                                    "原图引用缺失、损坏或越界；不可直接清理",
                                    "reveal",
                                )
                            }
                        }
                    }
                }
            }
        }
        let mut nfo_code = None;
        if nfo.exists() {
            match append::confined_path(&root, &nfo)
                .and_then(|_| append::regular(&nfo))
                .and_then(|_| fs::read_to_string(&nfo))
            {
                Ok(xml) if valid_xml(&xml) => {
                    nfo_code = crate::nfo_read::parse_nfo(&xml, code.as_deref().unwrap_or(""))
                        .map(|m| m.code)
                }
                _ => add(
                    &mut r,
                    "invalidNfo",
                    dir,
                    code.as_deref(),
                    &nfo,
                    "NFO 损坏或无法读取",
                    "reveal",
                ),
            }
        }
        if let (Some(c), Some(n)) = (&code, &nfo_code) {
            if !n.is_empty() && c != n {
                add(
                    &mut r,
                    "codeMismatch",
                    dir,
                    Some(c),
                    &nfo,
                    format!("sidecar 番号 {c} 与 NFO {n} 不一致"),
                    "reveal",
                )
            }
        }
        if meta.is_none() && !side.exists() {
            add(
                &mut r,
                "missingMetadata",
                dir,
                None,
                dir,
                "视频没有可读取的影片元数据",
                "reveal",
            )
        }
        if videos.is_empty() {
            add(
                &mut r,
                "missingVideo",
                dir,
                code.as_deref(),
                dir,
                "影片目录没有视频",
                "reveal",
            )
        }
        for video in &videos {
            let resolution=meta.as_ref().ok_or_else(||io::Error::other("未登记媒体链接")).and_then(|m|crate::placement::resolve_media_cancellable(&root,dir,video,m,cancel));
            if let Err(error)=resolution {
                if error.kind()==io::ErrorKind::Interrupted{r.cancelled=true;return Ok(r)}
                add(
                    &mut r,
                    if fs::symlink_metadata(video).is_ok_and(|m|m.file_type().is_symlink()){if error.kind()==io::ErrorKind::NotFound{"brokenMediaLink"}else if error.to_string().contains("未登记"){"unregisteredMediaLink"}else{"changedMediaLink"}}else{"unsafeVideo"},
                    dir,
                    code.as_deref(),
                    video,
                    error.to_string(),
                    "reveal",
                )
            }
        }
        let asset_paths = match crate::assets::Paths::for_movie(dir, meta.as_ref().unwrap_or(&Default::default())) {
            Ok(paths) => paths,
            Err(error) => { add(&mut r, "invalidSidecar", dir, code.as_deref(), &side, error.to_string(), "reveal"); continue; }
        };
        for (stem, base) in [("poster", &asset_paths.poster), ("fanart", &asset_paths.fanart)] {
            let paths: Vec<_> = ["jpg", "jpeg", "png"]
                .into_iter()
                .map(|ext| base.with_extension(ext))
                .filter(|p| p.exists())
                .collect();
            if paths.is_empty() {
                add(
                    &mut r,
                    "missingArtwork",
                    dir,
                    code.as_deref(),
                    base,
                    format!("缺少 {stem} 图片"),
                    "repairImages",
                )
            } else if !paths.iter().any(|p| {
                append::confined_path(&root, p).is_ok()
                    && append::regular(p).is_ok()
                    && image::open(p).is_ok()
            }) {
                add(
                    &mut r,
                    "invalidArtwork",
                    dir,
                    code.as_deref(),
                    &paths[0],
                    format!("{stem} 图片损坏或路径不安全"),
                    "reveal",
                )
            }
        }
        if let Some(m) = meta {
            disk.insert(
                dir.to_path_buf(),
                (
                    m.code.clone(),
                    videos,
                    serde_json::to_string(&m).unwrap_or_default(),
                ),
            );
        }
    }
    if cancel.is_cancelled() {
        r.cancelled = true;
        return Ok(r);
    }
    let db = Library::db_path(&root);
    if !db.exists() {
        add(
            &mut r,
            "missingIndex",
            &root,
            None,
            &db,
            "尚无媒体库缓存，可重建索引",
            "rebuildIndex",
        );
        return Ok(r);
    }
    append::confined_path(&root, &db)?;
    append::regular(&db)?;
    let compare = || -> rusqlite::Result<Vec<(String, String, String, HashSet<PathBuf>)>> {
        let conn =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut s = conn.prepare("SELECT code,dir,meta_json FROM movie")?;
        let rows = s
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = vec![];
        for (code, dir, meta) in rows {
            let mut st = conn.prepare("SELECT rel_path FROM file WHERE code=?1")?;
            let files = st
                .query_map([&code], |row| row.get::<_, String>(0))?
                .map(|v| v.map(|v| root.join(v)))
                .collect::<Result<HashSet<_>, _>>()?;
            out.push((code, dir, meta, files));
        }
        Ok(out)
    };
    match compare() {
        Ok(rows) => {
            let mut indexed = HashSet::new();
            for (code, dir, meta, files) in rows {
                let path = root.join(dir);
                indexed.insert(path.clone());
                let matching = disk
                    .get(&path)
                    .is_some_and(|(disk_code, videos, disk_meta)| {
                        *disk_code == code
                            && *videos == files
                            && serde_json::from_str::<serde_json::Value>(disk_meta).ok()
                                == serde_json::from_str::<serde_json::Value>(&meta).ok()
                    });
                if !matching {
                    add(
                        &mut r,
                        "staleIndex",
                        &path,
                        Some(&code),
                        &path,
                        "磁盘与缓存不同，可重建索引；不会改动媒体文件",
                        "rebuildIndex",
                    )
                }
            }
            for (dir, (code, _, _)) in &disk {
                if !indexed.contains(dir) {
                    add(
                        &mut r,
                        "missingIndexEntry",
                        dir,
                        Some(code),
                        dir,
                        "磁盘影片尚未进入缓存",
                        "rebuildIndex",
                    )
                }
            }
        }
        Err(e) => add(
            &mut r,
            "invalidIndex",
            &root,
            None,
            &db,
            format!("缓存无法读取：{e}"),
            "rebuildIndex",
        ),
    }
    Ok(r)
}
