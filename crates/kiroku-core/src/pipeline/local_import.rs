//! Read companions of scanned source videos. Paths and the NFO snapshot live
//! only for the import session; output metadata belongs to its new directory.
use super::{ImageCandidate, ScanItem, VIDEO_EXT};
use crate::{code::Code, metadata::Metadata, organize::Op};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

const NFO_LIMIT: usize = 4 * 1024 * 1024;

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
    generation: DirectoryGeneration,
}

#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum DirectoryGeneration {
    Birth(std::time::SystemTime),
    Change(i64, i64),
}

#[cfg(unix)]
impl DirectoryIdentity {
    fn capture(dir: &Path) -> io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        let metadata = fs::symlink_metadata(dir)?;
        if !metadata.is_dir() {
            return Err(io::Error::other("来源配套路径不是目录"));
        }
        // Retain an identity instead of holding one open directory per imported movie.
        // On filesystems without a birth time, reject a changed directory conservatively.
        let generation = match metadata.created() {
            Ok(time) if time != std::time::UNIX_EPOCH => DirectoryGeneration::Birth(time),
            _ => DirectoryGeneration::Change(metadata.ctime(), metadata.ctime_nsec()),
        };
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            generation,
        })
    }
}

#[cfg(not(unix))]
#[derive(Debug, PartialEq, Eq)]
struct DirectoryIdentity(same_file::Handle);

#[cfg(not(unix))]
impl DirectoryIdentity {
    fn capture(dir: &Path) -> io::Result<Self> {
        same_file::Handle::from_path(dir).map(Self)
    }
}

impl DirectoryIdentity {
    fn verify(&self, dir: &Path) -> io::Result<()> {
        if Self::capture(dir)? != *self {
            return Err(io::Error::other("来源目录已变化"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalImport {
    nfo: Option<String>,
    poster: Option<PathBuf>,
    fanart: Option<PathBuf>,
    dir: PathBuf,
    directory_identity: Arc<DirectoryIdentity>,
    pub(super) refresh: bool,
}

#[derive(Debug, Clone)]
pub(super) struct LocalImage {
    path: PathBuf,
    dir: PathBuf,
    directory_identity: Arc<DirectoryIdentity>,
}

impl LocalImage {
    pub(super) fn read(&self) -> io::Result<Vec<u8>> {
        self.directory_identity.verify(&self.dir)?;
        let bytes = read_regular(&self.path, crate::net::IMAGE_RESPONSE_LIMIT)?;
        self.directory_identity.verify(&self.dir)?;
        Ok(bytes)
    }
}

fn read_regular(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(io::Error::other("配套文件不是普通文件或超出大小限制"));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(io::Error::other("读取时配套文件已变化"));
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("读取时配套文件超出大小限制"));
    }
    Ok(bytes)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}

fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}

fn valid_nfo(path: &Path, code: &Code) -> Option<String> {
    let xml = String::from_utf8(read_regular(path, NFO_LIMIT).ok()?).ok()?;
    let meta = crate::nfo_read::parse_nfo(&xml, &code.id)?;
    if !meta.code.eq_ignore_ascii_case(&code.id) {
        return None;
    }
    crate::nfo::merge_revision(&xml, &meta, &meta).ok()?;
    (crate::metadata::usable_partial(&meta.fields) || !meta.scores.is_empty()).then_some(xml)
}

pub(super) fn discover(
    items: &[ScanItem],
    cancel: &CancellationToken,
) -> HashMap<String, Arc<LocalImport>> {
    let mut dirs = BTreeMap::<&Path, Vec<(&ScanItem, &Code)>>::new();
    for item in items {
        if let (Some(dir), Some(code)) = (item.path.parent(), item.code.as_ref()) {
            dirs.entry(dir).or_default().push((item, code));
        }
    }
    let mut result = HashMap::<String, Arc<LocalImport>>::new();
    for (dir, items) in dirs {
        if cancel.is_cancelled() {
            break;
        }
        let Ok(canonical) = dir.canonicalize() else {
            continue;
        };
        let Ok(identity) = DirectoryIdentity::capture(&canonical) else {
            continue;
        };
        let identity = Arc::new(identity);
        let Ok(entries) = fs::read_dir(&canonical) else {
            continue;
        };
        let mut files = BTreeMap::<(String, String), PathBuf>::new();
        let mut video_names = Vec::new();
        let mut readable = true;
        for entry in entries {
            if cancel.is_cancelled() {
                return result;
            }
            match entry.and_then(|entry| Ok((entry.file_type()?, entry.path()))) {
                Ok((kind, path)) => {
                    let ext = extension(&path);
                    if (kind.is_file() || kind.is_symlink()) && VIDEO_EXT.contains(&ext.as_str()) {
                        video_names.push(path.file_name().unwrap_or_default().to_os_string());
                    }
                    if kind.is_file()
                        && ["nfo", "jpg", "jpeg", "png", "webp"].contains(&ext.as_str())
                    {
                        // Case variants are ambiguous on case-sensitive volumes. Prefer a stable path.
                        files
                            .entry((stem(&path), ext))
                            .and_modify(|old| {
                                if path < *old {
                                    old.clone_from(&path);
                                }
                            })
                            .or_insert(path);
                    }
                }
                Err(_) => readable = false,
            }
        }
        let mut groups = BTreeMap::<&str, Vec<(&ScanItem, &Code)>>::new();
        for (item, code) in items {
            groups.entry(&code.id).or_default().push((item, code));
        }
        let unique_group = groups.len() == 1;
        for (id, mut group) in groups {
            group.sort_by_key(|(item, code)| {
                (
                    code.part.unwrap_or(0),
                    std::cmp::Reverse(item.size),
                    &item.path,
                )
            });
            let code = group[0].1;
            let selected_names: HashSet<_> = group
                .iter()
                .filter_map(|(item, _)| item.path.file_name())
                .collect();
            let generic = unique_group
                && readable
                && video_names.iter().all(|name| {
                    selected_names.contains(name.as_os_str())
                        || crate::parse_code(&name.to_string_lossy())
                            .is_some_and(|other| other.id == id)
                });
            let mut stems: Vec<_> = group.iter().map(|(item, _)| stem(&item.path)).collect();
            if generic {
                stems.push(id.to_lowercase());
            }
            let find_image = |roles: &[&str]| {
                crate::assets::local_image(&files, &stems, generic, roles, &[]).map(Path::to_owned)
            };
            let find_nfo = |wanted: String| {
                files
                    .get(&(wanted, "nfo".into()))
                    .and_then(|path| valid_nfo(path, code))
            };
            let nfo = stems
                .iter()
                .find_map(|s| find_nfo(s.clone()))
                .or_else(|| generic.then(|| find_nfo("movie".into())).flatten());
            let poster = find_image(&["poster", "folder", "cover"]);
            let fanart = find_image(&["fanart", "background", "backdrop"]);
            if nfo.is_none() && poster.is_none() && fanart.is_none() {
                continue;
            }
            if identity.verify(&canonical).is_err() {
                break;
            }
            if result
                .get(id)
                .is_some_and(|found| found.nfo.is_some() || nfo.is_none())
            {
                continue;
            }
            result.insert(
                id.into(),
                Arc::new(LocalImport {
                    nfo,
                    poster,
                    fanart,
                    dir: canonical.clone(),
                    directory_identity: identity.clone(),
                    refresh: false,
                }),
            );
        }
    }
    result
}

impl LocalImport {
    /// Bind companions to the chosen scan row, including image-only source folders.
    pub fn beside(item: &ScanItem, scan: &[ScanItem], cancel: &CancellationToken) -> Option<Arc<Self>> {
        let code = item.code.as_ref()?;
        let siblings = scan.iter().filter(|s| s.path.parent() == item.path.parent()).cloned().collect::<Vec<_>>();
        let found = discover(&siblings, cancel).remove(&code.id);
        if found.is_some() || cancel.is_cancelled() { return found; }
        let dir = item.path.parent()?.canonicalize().ok()?;
        if !fs::symlink_metadata(dir.join(".actors")).ok()?.is_dir() { return None; }
        let directory_identity = Arc::new(DirectoryIdentity::capture(&dir).ok()?);
        Some(Arc::new(Self { nfo:None, poster:None, fanart:None, dir, directory_identity, refresh:false }))
    }

    pub(crate) fn artwork_path(&self, poster: bool) -> Option<&Path> {
        if poster { self.poster.as_deref() } else { self.fanart.as_deref() }
    }

    pub(crate) fn read_artwork(&self, poster: bool) -> io::Result<Vec<u8>> {
        let path = self.artwork_path(poster).ok_or_else(|| io::Error::other("来源图片不存在"))?;
        LocalImage { path: path.into(), dir: self.dir.clone(), directory_identity: self.directory_identity.clone() }.read()
    }

    pub(crate) fn has_actor_image(&self, name: &str) -> io::Result<bool> {
        self.directory_identity.verify(&self.dir)?;
        crate::supplement::local_avatar_path(&self.dir, name).map(|p|p.is_some())
    }

    pub(crate) fn read_actor_image(&self, name: &str) -> io::Result<Vec<u8>> {
        if !self.has_actor_image(name)? {return Err(io::Error::other("来源头像已变化"));}
        let bytes = crate::supplement::read_local_avatar(&self.dir, name)?.ok_or_else(||io::Error::other("来源头像已变化"))?;
        self.directory_identity.verify(&self.dir)?;
        Ok(bytes)
    }

    pub(crate) fn supplement_nfo(&self, after: &Metadata) -> io::Result<Option<String>> {
        self.nfo.as_deref().map(|xml| {
            let before = crate::nfo_read::parse_nfo(xml, &after.code).ok_or_else(|| io::Error::other("来源 NFO 无效"))?;
            crate::nfo::merge_revision(xml, &before, after)
        }).transpose()
    }

    pub(super) fn metadata(&self, id: &str) -> Metadata {
        let mut meta = self
            .nfo
            .as_deref()
            .and_then(|xml| crate::nfo_read::parse_nfo(xml, id))
            .unwrap_or_else(|| Metadata {
                code: id.into(),
                ..Default::default()
            });
        meta.code = id.into();
        for urls in [
            &mut meta.fields.poster_urls,
            &mut meta.fields.thumb_urls,
            &mut meta.fields.trailer_urls,
        ] {
            urls.retain(|value| {
                reqwest::Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            });
        }
        if let Ok(fields) = serde_json::to_value(&meta.fields) {
            for &field in crate::aggregate::FIELDS {
                if crate::refresh::nonempty(&fields[field]) {
                    meta.provenance.insert(field.into(), "local_nfo".into());
                }
            }
        }
        crate::localization::capture_original(&mut meta);
        meta
    }

    pub(super) fn has_nfo(&self) -> bool {
        self.nfo.is_some()
    }

    pub(super) fn prefers_local_poster(&self) -> bool {
        !self.refresh && self.poster.is_some()
    }

    pub(super) fn notice(&self) -> String {
        let assets = [
            (self.nfo.is_some(), "NFO"),
            (self.poster.is_some(), "海报"),
            (self.fanart.is_some(), "背景图"),
        ];
        format!(
            "本地配套：{}",
            assets
                .into_iter()
                .filter_map(|(has, role)| has.then_some(role))
                .collect::<Vec<_>>()
                .join("、")
        )
    }

    pub(super) fn attach(
        &self,
        ops: &mut Vec<Op>,
        candidates: &mut HashMap<PathBuf, Vec<ImageCandidate>>,
        meta: &Metadata,
        dir: &Path,
    ) -> io::Result<()> {
        let Some(stem) = ops.iter().find_map(|op| match op {
            Op::WriteFile { path, .. } if path.extension().is_some_and(|ext| ext == "nfo") => path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned),
            _ => None,
        }) else {
            return Ok(());
        };
        let paths = crate::assets::Paths::for_stem(dir, &stem)?;
        if self.refresh {
            for candidate in candidates.values_mut().flatten() {
                candidate.fresh = true;
            }
        }
        if let Some(xml) = &self.nfo {
            let before = crate::nfo_read::parse_nfo(xml, &meta.code)
                .ok_or_else(|| io::Error::other("来源 NFO 无效"))?;
            let sidecar = ops
                .iter()
                .find_map(|op| match op {
                    Op::WriteFile { path, content }
                        if path
                            .file_name()
                            .is_some_and(|n| n == crate::organize::SIDECAR) =>
                    {
                        Some(content)
                    }
                    _ => None,
                })
                .ok_or_else(|| io::Error::other("整理计划缺少元数据"))?;
            let after: Metadata = serde_json::from_str(sidecar)?;
            let merged = crate::nfo::merge_revision(xml, &before, &after)?;
            if let Some(Op::WriteFile { content, .. }) =
                ops.iter_mut().find(|op| op.target() == paths.nfo)
            {
                *content = merged;
            }
        }
        for (target, src) in [(paths.poster, &self.poster), (paths.fanart, &self.fanart)] {
            let Some(path) = src else {
                continue;
            };
            let key = format!("local-artwork:{}", path.display());
            let candidate = ImageCandidate {
                url: key.clone(),
                crop: None,
                fresh: false,
                local: Some(LocalImage {
                    path: path.clone(),
                    dir: self.dir.clone(),
                    directory_identity: self.directory_identity.clone(),
                }),
            };
            let list = candidates.entry(target.clone()).or_default();
            if self.refresh {
                list.push(candidate);
            } else {
                list.insert(0, candidate);
            }
            if !ops.iter().any(|op| op.target() == target) {
                ops.push(Op::Download {
                    url: key,
                    path: target,
                    crop: None,
                    stamp: None,
                });
            }
        }
        Ok(())
    }
}

/// Adopt nonempty remote fields, keeping the local record for omissions.
pub(super) fn merge_remote(before: &Metadata, online: &Metadata) -> io::Result<Metadata> {
    let fields = crate::revision::EDITABLE_FIELDS
        .iter()
        .map(|field| field.to_string())
        .collect::<Vec<_>>();
    let mut after = crate::refresh::merge_selected(before, online, &fields, true)?;
    let fresh = &online.fields;
    if fresh
        .original_title
        .as_ref()
        .is_some_and(|v| !v.trim().is_empty())
        && !after.locked_fields.contains("original_title")
    {
        after.fields.original_title = fresh.original_title.clone();
        set_source(&mut after, online, "original_title");
    }
    for (field, values, target) in [
        (
            "poster_urls",
            &fresh.poster_urls,
            &mut after.fields.poster_urls,
        ),
        (
            "thumb_urls",
            &fresh.thumb_urls,
            &mut after.fields.thumb_urls,
        ),
        (
            "trailer_urls",
            &fresh.trailer_urls,
            &mut after.fields.trailer_urls,
        ),
        (
            "extrafanart",
            &fresh.extrafanart,
            &mut after.fields.extrafanart,
        ),
    ] {
        if !values.is_empty() && !after.locked_fields.contains(field) {
            target.clone_from(values);
            if let Some(source) = online.provenance.get(field) {
                after.provenance.insert(field.into(), source.clone());
            }
        }
    }
    if !online.scores.is_empty() && !after.locked_fields.contains("score") {
        after.scores.clone_from(&online.scores);
    }
    if !online.extrafanart_by_source.is_empty() && !after.locked_fields.contains("extrafanart") {
        after
            .extrafanart_by_source
            .clone_from(&online.extrafanart_by_source);
    }
    after.external_ids.extend(online.external_ids.clone());
    after.source_urls.extend(online.source_urls.clone());
    after.scrape_notes.clone_from(&online.scrape_notes);
    Ok(after)
}

fn set_source(result: &mut Metadata, online: &Metadata, field: &str) {
    if let Some(source) = online.provenance.get(field) {
        result.provenance.insert(field.into(), source.clone());
    }
}
