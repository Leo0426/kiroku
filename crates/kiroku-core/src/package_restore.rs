//! Read a portable package as data. Only indexed films in the current library are write targets.
use crate::{
    actor_hosting,
    artwork::PreparedImage,
    assets,
    journal::replacement::{digest, within},
    portability::{FileKind, Manifest, PackageFile, PackageMovie, MANIFEST_NAME},
    revision,
    settings::Settings,
    Metadata,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rule {
    Keep,
    #[default]
    Fill,
    Replace,
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct Rules {
    pub title: Rule,
    pub tags: Rule,
    pub actors: Rule,
    pub metadata: Rule,
    pub favorite: Rule,
    pub poster: Rule,
    pub fanart: Rule,
    pub avatars: Rule,
}

fn error(message: &str) -> io::Error {
    io::Error::other(message)
}
fn bounded(root: &Path, path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    within(root, path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > limit {
        return Err(error("资料文件过大或类型错误"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(error("读取期间文件超出大小限制"));
    }
    Ok(bytes)
}

pub struct Package {
    directory: PathBuf,
    pub manifest: Manifest,
    pub signature: String,
}
impl Package {
    pub fn open(directory: &Path) -> io::Result<Self> {
        let directory = directory.canonicalize()?;
        let bytes = bounded(&directory, &directory.join(MANIFEST_NAME), 32 * 1024 * 1024)?;
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        if manifest.version != 1 || manifest.movies.is_empty() {
            return Err(error("资料包为空或版本不受支持"));
        }
        let mut codes = BTreeSet::new();
        for movie in &manifest.movies {
            if movie.code.trim().is_empty() || !codes.insert(&movie.code) {
                return Err(error("资料包的影片编号为空或重复"));
            }
            let mut unique = BTreeSet::new();
            let mut single = BTreeSet::new();
            for file in &movie.files {
                let p = Path::new(&file.path);
                if p.as_os_str().is_empty()
                    || file.path.contains(['\\', ':'])
                    || p.components().any(|c| !matches!(c, Component::Normal(_)))
                    || !unique.insert(&file.path)
                    || file.sha256.len() != 64
                    || !file.sha256.bytes().all(|c| c.is_ascii_hexdigit())
                {
                    return Err(error("资料包有无效路径、重复文件或无效校验值"));
                }
                if let Some(role) = single_role(&file.kind) {
                    if !single.insert(role) {
                        return Err(error("影片含重复的同类资料"));
                    }
                }
            }
        }
        Ok(Self {
            directory,
            manifest,
            signature: digest(&bytes),
        })
    }
    fn read(&self, file: &PackageFile) -> io::Result<Vec<u8>> {
        let limit = match file.kind {
            FileKind::Nfo | FileKind::Sidecar => 16,
            FileKind::Avatar => 8,
            _ => 32,
        } * 1024
            * 1024;
        if file.bytes > limit {
            return Err(error("资料文件超出支持大小"));
        }
        let bytes = bounded(
            &self.directory,
            &self.directory.join(&file.path),
            file.bytes,
        )?;
        if bytes.len() as u64 != file.bytes || !digest(&bytes).eq_ignore_ascii_case(&file.sha256) {
            return Err(error("资料包校验失败"));
        }
        Ok(bytes)
    }
}

// The four singleton roles are mutually exclusive per film. Avatars and video parts are lists.
fn single_role(kind: &FileKind) -> Option<&'static str> {
    match kind {
        FileKind::Nfo => Some("nfo"),
        FileKind::Sidecar => Some("sidecar"),
        FileKind::Fanart => Some("fanart"),
        FileKind::Poster => Some("poster"),
        _ => None,
    }
}

pub struct Candidate {
    pub preview: Option<revision::Preview>,
    pub files: Vec<String>,
    pub warnings: Vec<String>,
}

impl Rule {
    fn take(self, empty: bool) -> bool {
        self == Self::Replace || (self == Self::Fill && empty)
    }
}
fn merge(
    before: &Metadata,
    src: &Metadata,
    rules: &Rules,
    sidecar: bool,
    generated_nfo: Option<&Metadata>,
) -> io::Result<Metadata> {
    let mut out = before.clone();
    let mut fields = serde_json::to_value(&out.fields)?;
    let incoming = serde_json::to_value(&src.fields)?;
    let generated = generated_nfo
        .map(|m| serde_json::to_value(&m.fields))
        .transpose()?;
    for &field in revision::EDITABLE_FIELDS.iter().chain([&"original_title"]) {
        if generated
            .as_ref()
            .is_some_and(|f| f[field] == incoming[field])
        {
            continue;
        }
        let rule = match field {
            "title" => rules.title,
            "tags" => rules.tags,
            "actors" => rules.actors,
            _ => rules.metadata,
        };
        let locked = before.locked_fields.contains(field);
        let imported_lock = sidecar && src.locked_fields.contains(field);
        if rule == Rule::Keep || (locked && rule == Rule::Fill) {
            continue;
        }
        let value = &incoming[field];
        if !crate::refresh::nonempty(value) && !imported_lock {
            continue;
        }
        if rule.take(!crate::refresh::nonempty(&fields[field])) {
            if value != &fields[field] {
                fields[field] = value.clone();
                out.name_projection.remove(field);
                out.provenance.insert(
                    field.to_string(),
                    src.provenance
                        .get(field)
                        .cloned()
                        .unwrap_or_else(|| "package".into()),
                );
            }
        }
        if fields[field] == *value && imported_lock {
            out.locked_fields.insert(field.to_string());
        }
    }
    out.fields = serde_json::from_value(fields)?;
    if sidecar && rules.favorite.take(!before.favorite) {
        out.favorite = src.favorite;
    }
    if rules.metadata.take(before.scores.is_empty())
        && !src.scores.is_empty()
        && generated_nfo.is_none_or(|m| m.scores != src.scores)
    {
        out.scores = src.scores.clone();
    }
    if sidecar
        && rules
            .metadata
            .take(before.text_versions == Default::default())
    {
        out.text_versions = src.text_versions.clone();
    }
    Ok(out)
}

fn image_for_path(raw: Vec<u8>, path: &Path) -> io::Result<Vec<u8>> {
    let expected = image::ImageFormat::from_path(path).map_err(io::Error::other)?;
    let checked = PreparedImage::normalize(&raw)?;
    if image::guess_format(&raw).ok() == Some(expected) {
        return Ok(raw);
    }
    let decoded = image::load_from_memory(checked.bytes()).map_err(io::Error::other)?;
    let mut out = io::Cursor::new(Vec::new());
    if expected == image::ImageFormat::Jpeg {
        decoded
            .to_rgb8()
            .write_to(&mut out, expected)
            .map_err(io::Error::other)?;
    } else {
        decoded
            .write_to(&mut out, expected)
            .map_err(io::Error::other)?;
    }
    Ok(out.into_inner())
}

pub fn prepare(
    package: &Package,
    movie: &PackageMovie,
    root: &Path,
    dir: &Path,
    rules: &Rules,
    settings: &Settings,
    cancel: &CancellationToken,
) -> io::Result<Candidate> {
    let captured = revision::capture(root, dir, &movie.code)?;
    let before = &captured.before;
    let mut nfo = None;
    let mut sidecar = None;
    for file in &movie.files {
        if cancel.is_cancelled() {
            return Err(error("导回已停止"));
        }
        match file.kind {
            FileKind::Nfo => {
                let bytes = package.read(file)?;
                let data = crate::nfo_read::parse_nfo(
                    std::str::from_utf8(&bytes).map_err(io::Error::other)?,
                    &movie.code,
                )
                .ok_or_else(|| error("资料包的 NFO 无法读取"))?;
                if data.code != movie.code {
                    return Err(error("NFO 番号与清单不符"));
                }
                nfo = Some(data);
            }
            FileKind::Sidecar => {
                let data: Metadata = serde_json::from_slice(&package.read(file)?)?;
                if data.code != movie.code {
                    return Err(error("kiroku.json 番号与清单不符"));
                }
                sidecar = Some(data);
            }
            _ => {}
        }
    }
    // External editors may add fields only to NFO. Keep the saved sidecar's
    // choices (including deliberately locked blanks), then fill its gaps.
    let combined = match (&sidecar, &nfo) {
        (Some(side), Some(nfo)) => {
            // Language projection and filename fallbacks can put values in NFO
            // that are deliberately absent from the stored metadata.
            let generated = crate::nfo_read::parse_nfo(&crate::nfo::to_nfo(side), &side.code)
                .ok_or_else(|| error("无法对照资料包自身的 NFO 字段"))?;
            Some(merge(
                side,
                nfo,
                &Rules::default(),
                false,
                Some(&generated),
            )?)
        }
        _ => None,
    };
    let source = combined.as_ref().or(sidecar.as_ref()).or(nfo.as_ref());
    let mut after = match source {
        Some(src) => merge(before, src, rules, sidecar.is_some(), None)?,
        None => before.clone(),
    };
    for actor in &mut after.fields.actors {
        let imported_path = actor
            .thumb
            .as_deref()
            .is_some_and(|url| crate::actor_image::validate_url(url).is_err());
        if imported_path && !before.fields.actors.contains(actor) {
            actor.thumb = None;
        }
    }
    let paths = assets::Paths::for_movie(&captured.dir, before)?;
    let mut art = Vec::new();
    let mut images = BTreeMap::new();
    let mut local_avatars = Vec::new();
    let mut warnings = Vec::new();
    let mut names = Vec::new();
    let mut avatar_bytes = 0u64;
    for file in &movie.files {
        if cancel.is_cancelled() {
            return Err(error("导回已停止"));
        }
        match file.kind {
            FileKind::Poster | FileKind::Fanart => {
                let (rule, path) = if matches!(file.kind, FileKind::Poster) {
                    (rules.poster, &paths.poster)
                } else {
                    (rules.fanart, &paths.fanart)
                };
                if rule == Rule::Keep {
                    continue;
                }
                let path = assets::existing_art(path)?.unwrap_or_else(|| path.to_owned());
                let (hash, _) = revision::snapshot_file(&captured.root, &path)?;
                if !rule.take(hash.is_none()) || hash.as_deref() == Some(file.sha256.as_str()) {
                    continue;
                }
                let bytes = image_for_path(package.read(file)?, &path)?;
                if Some(digest(&bytes)).as_ref() == hash.as_ref() {
                    continue;
                }
                if matches!(file.kind, FileKind::Poster) && after.artwork.is_some() {
                    if rule != Rule::Replace {
                        warnings.push("已保留本地海报裁剪记录".into());
                        continue;
                    }
                    after.artwork = None;
                }
                art.push((path.clone(), hash, bytes));
            }
            FileKind::Avatar if rules.avatars != Rule::Keep => {
                let source = file.source.as_deref();
                let actor = after.fields.actors.iter().find(|a| {
                    file.actor.as_ref() == Some(&a.name)
                        || a.thumb.as_deref().is_some_and(|url| {
                            Some(url) == source
                                || source
                                    .and_then(|k| sidecar.as_ref()?.hosted_actor_images.get(k))
                                    .is_some_and(|img| img.url == url)
                        })
                });
                let Some(actor) = actor else {
                    warnings.push("一张头像未匹配到本片演员".into());
                    continue;
                };
                let existing = crate::supplement::local_avatar_path(&captured.dir, &actor.name)?;
                let write_local = rules.avatars.take(existing.is_none());
                let hosted = actor
                    .thumb
                    .as_deref()
                    .filter(|_| settings.actor_hosting.enabled);
                let cached = match hosted {
                    Some(url) => {
                        actor_hosting::cached_avatar(&after, url, &settings.actor_hosting)?
                    }
                    None => None,
                };
                let bound = hosted
                    .and_then(|url| after.hosted_actor_images.get(url))
                    .is_some_and(|image| {
                        settings.actor_hosting.public_url(&image.file).ok().as_ref()
                            == Some(&image.url)
                    });
                let needs_cache = hosted.is_some()
                    && (rules.avatars == Rule::Replace || !bound || cached.is_none());
                if !write_local && !needs_cache {
                    continue;
                }
                avatar_bytes = avatar_bytes
                    .checked_add(file.bytes)
                    .ok_or_else(|| error("头像容量无效"))?;
                if avatar_bytes > 64 * 1024 * 1024 {
                    return Err(error("单部影片的头像超过 64 MiB"));
                }
                let bytes = package.read(file)?;
                let png = if image::guess_format(&bytes).ok() == Some(image::ImageFormat::Png) {
                    crate::artwork::PreparedImage::normalize(&bytes)?;
                    bytes
                } else {
                    crate::actor_image::thumbnail(&bytes)?
                };
                if write_local {
                    if let Some(path) = existing {
                        let local = image_for_path(png.clone(), &path)?;
                        if bounded(&captured.root, &path, 8 * 1024 * 1024)? != local {
                            local_avatars.push((actor.name.clone(), local));
                        }
                    } else {
                        local_avatars.push((actor.name.clone(), png.clone()));
                    }
                }
                if let Some(url) =
                    hosted.filter(|_| needs_cache && (!bound || cached.as_deref() != Some(&png)))
                {
                    names.push(format!(
                        "头像 → {}/{}.png",
                        settings.actor_hosting.directory,
                        digest(&png)
                    ));
                    images.insert(url.to_owned(), png);
                }
            }
            _ => {}
        }
    }
    let cache = if images.is_empty() {
        None
    } else {
        Some(actor_hosting::from_package(
            &mut after,
            &settings.actor_hosting,
            images,
        )?)
    };
    if *before == after && art.is_empty() && local_avatars.is_empty() && cache.is_none() {
        return Ok(Candidate {
            preview: None,
            files: names,
            warnings,
        });
    }
    let mut preview = captured.prepare_after(after)?;
    for (path, hash, bytes) in art {
        preview = preview.with_package_art(path, hash, bytes)?;
    }
    for (name, bytes) in local_avatars {
        preview = preview.with_package_avatar(&name, bytes)?;
    }
    let has_cache = cache.is_some();
    if let Some(cache) = cache {
        preview = preview.with_actor_cache(cache, vec![])?;
    }
    let old = serde_json::to_value(&preview.before)?;
    let new = serde_json::to_value(&preview.after)?;
    // NFO language projection can already show a multilingual value while
    // its stored field is still empty. Report that sidecar change as well.
    for key in revision::EDITABLE_FIELDS.iter().copied().chain([
        "original_title",
        "locked_fields",
        "scores",
        "text_versions",
        "artwork",
    ]) {
        if old[key] != new[key] && !preview.changes.iter().any(|change| change.field == key) {
            preview.changes.push(revision::Change {
                field: key.into(),
                before: old[key].clone(),
                after: new[key].clone(),
            });
        }
    }
    warnings.extend(preview.warnings.iter().cloned());
    let paths = preview.package_paths();
    if paths.is_empty() && !has_cache {
        return Ok(Candidate {
            preview: None,
            files: vec![],
            warnings,
        });
    }
    names.extend(paths);
    Ok(Candidate {
        preview: Some(preview),
        files: names,
        warnings,
    })
}

impl Candidate {
    pub fn signature(&self) -> Option<String> {
        self.preview
            .as_ref()
            .map(revision::Preview::package_signature)
    }
}
