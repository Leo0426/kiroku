//! Fill selected, missing companions of an existing library movie. The preview
//! owns its disk bindings and sources. Only exclusive companion writes enter history.
use crate::{
    actor_image, append, assets,
    journal::{Checkpoint, Journal, JournalItem, JournalStatus},
    metadata::{Actor, Metadata},
    organize::{self, Crop, Op, Plan},
    pipeline::{LocalImport, RootGuard},
    settings::Settings,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Nfo,
    Poster,
    Fanart,
    Actors,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Missing,
    Existing,
    Unavailable,
}
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub kind: Kind,
    pub path: Option<PathBuf>,
    pub status: Status,
    pub reason: String,
    #[serde(skip)]
    sources: Vec<Source>,
}
#[derive(Debug, Clone)]
enum Source {
    Text(String),
    Local(bool),
    Url(String, Option<Crop>),
    Avatar(String),
    CachedAvatar(String),
    LocalAvatar(String),
}

pub struct Preview {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub code: String,
    pub entries: Vec<Entry>,
    root_guard: RootGuard,
    dir_guard: RootGuard,
    video: PathBuf,
    video_identity: same_file::Handle,
    before: Option<Metadata>,
    local: Option<Arc<LocalImport>>,
}
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub written: Vec<PathBuf>,
    pub skipped: Vec<String>,
    pub journal_id: Option<String>,
    pub cancelled: bool,
}

fn fail(msg: &str) -> io::Error {
    io::Error::other(msg)
}
fn actor_base(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let name = organize::sanitize(name);
    if name.is_empty() || name.len() > 240 {
        return Err(fail("演员名不能生成有效的头像文件名"));
    }
    Ok(dir.join(".actors").join(format!("{name}.png")))
}
const IMAGE_EXT: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
fn existing_image(base: &Path) -> io::Result<Option<PathBuf>> {
    for ext in IMAGE_EXT {
        let p = base.with_extension(ext);
        match fs::symlink_metadata(&p) {
            Ok(_) => return Ok(Some(p)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}
pub fn local_avatar_path(dir: &Path, name: &str) -> io::Result<Option<PathBuf>> {
    let base = append::confined_path(dir, &actor_base(dir, name)?)?;
    if let Some(path) = existing_image(&base)? {
        append::regular(&path)?;
        return Ok(Some(path));
    }
    Ok(None)
}
pub fn read_local_avatar(dir: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let Some(path) = local_avatar_path(dir, name)? else {
        return Ok(None);
    };
    use std::io::Read;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let max = 8 * 1024 * 1024;
    if !file.metadata()?.is_file() || file.metadata()?.len() > max {
        return Err(fail("演员头像文件无效或超过 8 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(fail("演员头像文件超过 8 MiB"));
    }
    actor_image::thumbnail(&bytes).map(Some)
}
fn disk_meta(dir: &Path) -> io::Result<Option<Metadata>> {
    let nfo = assets::nfo_path(dir)?;
    if !organize::occupied(&dir.join("kiroku.json")) && !organize::occupied(&nfo) {
        return Ok(None);
    }
    append::read_meta(dir).map(Some)
}
fn existing(dir: &Path, kind: Kind, target: &Path, video: &Path) -> io::Result<Option<PathBuf>> {
    if kind == Kind::Nfo {
        let path = assets::nfo_path(dir)?;
        return Ok(organize::occupied(&path).then_some(path));
    }
    if kind == Kind::Actors {
        return existing_image(target);
    }
    let role = if kind == Kind::Poster {
        "poster"
    } else {
        "fanart"
    };
    let stem = video.file_stem().unwrap_or_default().to_string_lossy();
    for p in [
        target.to_owned(),
        dir.join(format!("{role}.jpg")),
        dir.join(format!("{stem}-{role}.jpg")),
    ] {
        if let Some(found) = existing_image(&p)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}
fn targets_actor<'a>(disk: &'a Metadata, incoming: Option<&'a Metadata>) -> Vec<&'a Actor> {
    let mut actors = Vec::new();
    let mut names = HashSet::new();
    for actor in disk
        .fields
        .actors
        .iter()
        .chain(incoming.into_iter().flat_map(|m| &m.fields.actors))
    {
        if !names.insert(actor.name.as_str()) {
            continue;
        }
        let matched = incoming.and_then(|m| {
            m.fields
                .actors
                .iter()
                .find(|a| a.name == actor.name && a.thumb.is_some())
        });
        actors.push(if actor.thumb.is_some() {
            actor
        } else {
            matched.unwrap_or(actor)
        });
    }
    actors
}
pub fn preview(
    root: &Path,
    dir: &Path,
    code: &str,
    video: &Path,
    incoming: Option<&Metadata>,
    local: Option<Arc<LocalImport>>,
    settings: &Settings,
) -> io::Result<Preview> {
    let dir = append::confined_path(root, dir)?;
    let video_parent = video
        .parent()
        .map(|p| append::confined_path(root, p))
        .transpose()?;
    let root = root.canonicalize()?;
    if dir == root || video_parent.as_ref() != Some(&dir) {
        return Err(fail("库中影片视频与目录不匹配，请更新索引"));
    }
    let video = dir.join(video.file_name().ok_or_else(|| fail("影片路径无效"))?);
    if !crate::pipeline::VIDEO_EXT.contains(
        &video
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
            .as_str(),
    ) || !video.is_file()
    {
        return Err(fail("库内原视频不可用，请重新检查媒体库"));
    }
    let video_identity = same_file::Handle::from_path(&video)?;
    let root_guard = RootGuard::capture(&root, false)?;
    let dir_guard = RootGuard::capture(&dir, false)?;
    let before = disk_meta(&dir)?;
    if before.as_ref().is_some_and(|m| m.code != code) || incoming.is_some_and(|m| m.code != code) {
        return Err(fail("磁盘或导入资料的影片编号不匹配"));
    }
    if before.is_none()
        && !crate::parse_code(&video.file_name().unwrap_or_default().to_string_lossy())
            .is_some_and(|c| c.id == code)
    {
        return Err(fail("本片缺少磁盘资料，原视频编号也无法匹配"));
    }
    let empty = Metadata {
        code: code.into(),
        ..Default::default()
    };
    let meta = before.as_ref().or(incoming).unwrap_or(&empty);
    let paths = assets::Paths::for_movie(&dir, meta)?;
    let local = local.or_else(|| incoming.and_then(|m| m.import_local.clone()));
    let source_meta = crate::artwork::repair_metadata(meta, settings);
    let mut entries = vec![];
    let mut add = |kind, target: PathBuf, sources: Vec<Source>, reason: &str| -> io::Result<()> {
        let found = existing(&dir, kind, &target, &video)?;
        let status = if found.is_some() {
            Status::Existing
        } else if sources.is_empty() {
            Status::Unavailable
        } else {
            Status::Missing
        };
        entries.push(Entry {
            kind,
            path: Some(found.unwrap_or(target)),
            status,
            reason: match status {
                Status::Existing => "保留已有文件",
                Status::Missing => "可补齐",
                Status::Unavailable => reason,
            }
            .into(),
            sources,
        });
        Ok(())
    };
    let xml = if crate::metadata::usable_partial(&meta.fields) || !meta.scores.is_empty() {
        Some(
            local
                .as_ref()
                .map(|l| l.supplement_nfo(meta))
                .transpose()?
                .flatten()
                .unwrap_or_else(|| crate::nfo::to_nfo(meta)),
        )
    } else {
        None
    };
    let nfo = if meta.asset_stem.is_some() {
        paths.nfo
    } else {
        video.with_extension("nfo")
    };
    add(
        Kind::Nfo,
        nfo,
        xml.into_iter().map(Source::Text).collect(),
        "没有可写入 NFO 的影片资料",
    )?;
    for (poster, target) in [(true, paths.poster), (false, paths.fanart)] {
        let mut sources = Vec::new();
        if local
            .as_ref()
            .and_then(|l| l.artwork_path(poster))
            .is_some()
        {
            sources.push(Source::Local(poster))
        }
        for m in incoming.into_iter().chain(std::iter::once(&source_meta)) {
            if poster {
                for u in &m.fields.poster_urls {
                    sources.push(Source::Url(u.clone(), None))
                }
            }
            for u in &m.fields.thumb_urls {
                sources.push(Source::Url(u.clone(), poster.then_some(Crop::PosterRight)))
            }
        }
        sources.retain(|s| !matches!(s,Source::Url(u,_) if actor_image::validate_url(u).is_err()));
        let locked = poster && meta.locked_fields.contains("poster_urls");
        if locked {
            sources.clear()
        }
        add(
            if poster { Kind::Poster } else { Kind::Fanart },
            target,
            sources,
            if locked {
                "海报已人工锁定"
            } else {
                "没有可用图片来源，请先重新抓取"
            },
        )?;
    }
    let actors = targets_actor(meta, incoming);
    let mut filenames = HashSet::new();
    let collisions = actors
        .iter()
        .filter_map(|actor| actor_base(&dir, &actor.name).ok())
        .map(|path| path.to_string_lossy().to_lowercase())
        .filter(|name| !filenames.insert(name.clone()))
        .collect::<HashSet<_>>();
    let mut unavailable = Vec::new();
    for actor in &actors {
        let prepare = || -> io::Result<(PathBuf, Vec<Source>)> {
            let target = actor_base(&dir, &actor.name)?;
            if collisions.contains(&target.to_string_lossy().to_lowercase()) {
                return Err(fail("名称对应同一头像文件，请先核对演员名"));
            }
            append::confined_path(&root, target.parent().unwrap())?;
            let mut sources = Vec::new();
            if local
                .as_ref()
                .is_some_and(|assets| assets.has_actor_image(&actor.name).unwrap_or(false))
            {
                sources.push(Source::LocalAvatar(actor.name.clone()));
            }
            if let Some(url) = &actor.thumb {
                if meta.hosted_actor_images.contains_key(url) && settings.actor_hosting.enabled {
                    sources.push(Source::CachedAvatar(url.clone()))
                }
                if actor_image::validate_url(url).is_ok() {
                    sources.push(Source::Avatar(url.clone()))
                }
            }
            Ok((target, sources))
        };
        match prepare().and_then(|(target, sources)| {
            add(Kind::Actors, target, sources, "该演员没有可用头像来源")
        }) {
            Ok(()) => (),
            Err(e) => unavailable.push(format!("{}：{e}", actor.name)),
        }
    }
    if actors.is_empty() {
        unavailable.push("影片资料中没有演员".into());
    }
    entries.extend(unavailable.into_iter().map(|reason| Entry {
        kind: Kind::Actors,
        path: None,
        status: Status::Unavailable,
        reason,
        sources: vec![],
    }));
    Ok(Preview {
        root,
        dir,
        code: code.into(),
        entries,
        local,
        before,
        video,
        video_identity,
        root_guard,
        dir_guard,
    })
}

impl Preview {
    fn verify(&self) -> io::Result<()> {
        self.root_guard.verify()?;
        self.dir_guard.verify()?;
        if self.video_identity != same_file::Handle::from_path(&self.video)? {
            return Err(fail("库内原视频身份已变化，请重新预览"));
        }
        Ok(())
    }
}

/// Network reads are supplied by the desktop worker. Each created file has its
/// own durable intent and publication proof; existing files never enter history.
pub fn apply(
    p: Preview,
    selected: &[Kind],
    settings: &Settings,
    cancel: &CancellationToken,
    fetch: &mut dyn FnMut(&str, bool) -> io::Result<Vec<u8>>,
) -> io::Result<Outcome> {
    p.verify()?;
    if selected.is_empty() {
        return Err(fail("请选择要补的资料"));
    }
    if let Some(before) = &p.before {
        if disk_meta(&p.dir)?.as_ref() != Some(before) {
            return Err(fail("影片资料已变化，请重新预览"));
        }
    }
    let mut result = Outcome::default();
    let mut journal = Journal::new(&p.root);
    journal.items.push(JournalItem {
        dir: p.dir.clone(),
        code: p.code.clone(),
    });
    for entry in p
        .entries
        .iter()
        .filter(|e| selected.contains(&e.kind) && e.status == Status::Missing)
    {
        if cancel.is_cancelled() {
            break;
        }
        let path = entry.path.as_ref().unwrap();
        let check = || -> io::Result<()> {
            p.verify()?;
            append::confined_path(&p.root, path)?;
            if existing(&p.dir, entry.kind, path, &p.video)?.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "文件已出现，已保留",
                ));
            }
            Ok(())
        };
        if let Err(e) = check() {
            result.skipped.push(format!("{}：{e}", path.display()));
            continue;
        }
        let mut prepared = None;
        let mut last = String::new();
        for source in &entry.sources {
            if cancel.is_cancelled() {
                break;
            }
            let mut read = || -> io::Result<(Op, Vec<u8>)> {
                let (bytes, crop) = match source {
                    Source::Text(content) => {
                        return Ok((
                            Op::WriteFile {
                                path: path.clone(),
                                content: content.clone(),
                            },
                            vec![],
                        ))
                    }
                    Source::Local(poster) => {
                        (p.local.as_ref().unwrap().read_artwork(*poster)?, None)
                    }
                    Source::Url(url, crop) => (fetch(url, false)?, *crop),
                    Source::Avatar(url) => (fetch(url, true)?, None),
                    Source::LocalAvatar(name) => {
                        (p.local.as_ref().unwrap().read_actor_image(name)?, None)
                    }
                    Source::CachedAvatar(url) => (
                        crate::actor_hosting::cached_avatar(
                            p.before.as_ref().ok_or_else(|| fail("无本片头像缓存"))?,
                            url,
                            &settings.actor_hosting,
                        )?
                        .ok_or_else(|| fail("无本片头像缓存"))?,
                        None,
                    ),
                };
                let bytes = if entry.kind == Kind::Actors {
                    actor_image::thumbnail(&bytes)?
                } else {
                    let plane = crate::artwork::PreparedImage::normalize(&bytes)?;
                    let processed = organize::process_image_shared(plane.bytes(), crop, None)?;
                    let mut out = Vec::new();
                    image::load_from_memory(&processed)
                        .map_err(io::Error::other)?
                        .to_rgb8()
                        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                            &mut out, 92,
                        ))
                        .map_err(io::Error::other)?;
                    out
                };
                Ok((
                    Op::Download {
                        path: path.clone(),
                        url: "supplement".into(),
                        crop: None,
                        stamp: None,
                    },
                    bytes,
                ))
            };
            match read() {
                Ok(value) => {
                    prepared = Some(value);
                    break;
                }
                Err(e) => last = e.to_string(),
            }
        }
        if cancel.is_cancelled() {
            break;
        }
        let Some((op, bytes)) = prepared else {
            result.skipped.push(format!("{}：{}", path.display(), last));
            continue;
        };
        if let Err(e) = check() {
            result.skipped.push(format!("{}：{e}", path.display()));
            continue;
        }
        let mut bytes = Some(bytes);
        let out = journal.execute_observed(
            &Plan { ops: vec![op] },
            &mut |_| Ok(bytes.take().unwrap_or_default()),
            &|| cancel.is_cancelled(),
            &mut |point| {
                if matches!(point, Checkpoint::Intent | Checkpoint::Staged) {
                    check()?
                }
                Ok(())
            },
        );
        result
            .written
            .extend(out.done.iter().map(|op| op.target().to_path_buf()));
        result.skipped.extend(out.warnings);
        if let Some(e) = out.error {
            result.skipped.push(format!("{}：{e}", path.display()));
            break;
        }
    }
    result.cancelled = cancel.is_cancelled();
    if journal.path().exists() {
        journal.status = if result.skipped.is_empty() && !result.cancelled {
            JournalStatus::Applied
        } else {
            JournalStatus::Partial
        };
        if let Err(e) = journal.save() {
            result.skipped.push(format!("日志状态保存失败：{e}"))
        }
        result.journal_id = Some(journal.id.clone());
    }
    Ok(result)
}
