//! A portable collection of explicit movie files. The manifest is written last.
use crate::{
    actor_hosting, append, assets, library::LibraryReader, organize, pipeline::RootGuard,
    placement, subtitle,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use tokio_util::sync::CancellationToken;
use unicode_normalization::UnicodeNormalization;

pub const MANIFEST_NAME: &str = "kiroku-package.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Nfo,
    Poster,
    Fanart,
    Avatar,
    Sidecar,
    Video,
    Subtitle,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub movies: Vec<PackageMovie>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageMovie {
    pub code: String,
    pub files: Vec<PackageFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackageFile {
    pub kind: FileKind,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview {
    pub destination: PathBuf,
    pub movies: usize,
    pub files: usize,
    pub bytes: u64,
    pub examples: Vec<String>,
    pub warnings: Vec<String>,
    pub warning_count: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct SourceStamp {
    bytes: u64,
    modified: SystemTime,
    #[cfg(unix)]
    inode: (u64, u64),
}
impl SourceStamp {
    fn read(path: &Path) -> io::Result<Self> {
        let meta = fs::symlink_metadata(path)?;
        if !meta.is_file() {
            return Err(io::Error::other("导出来源须为普通文件"));
        }
        Ok(Self {
            bytes: meta.len(),
            modified: meta.modified()?,
            #[cfg(unix)]
            inode: {
                use std::os::unix::fs::MetadataExt;
                (meta.dev(), meta.ino())
            },
        })
    }
}

struct Input {
    path: PathBuf,
    confinement: PathBuf,
    link: Option<(PathBuf, String)>,
    expected_hash: Option<String>,
    stamp: SourceStamp,
    file: PackageFile,
}
struct MoviePlan {
    code: String,
    files: Vec<Input>,
}
pub struct ExportPlan {
    pub preview: ExportPreview,
    parent: RootGuard,
    library: RootGuard,
    movies: Vec<MoviePlan>,
}

fn checked_name(file: &Path) -> io::Result<&str> {
    let name = file
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| io::Error::other("文件名无法用于资料包"))?;
    if name.contains(['/', '\\', ':']) || name.chars().any(char::is_control) {
        return Err(io::Error::other(format!("文件名不适合跨设备导出：{name}")));
    }
    Ok(name)
}

fn add_file(
    movie: &mut MoviePlan,
    path: &Path,
    confinement: &Path,
    relative: String,
    kind: FileKind,
    actor: Option<(&str, Option<&str>)>,
) -> io::Result<()> {
    let path = append::confined_path(confinement, path)?;
    let stamp = SourceStamp::read(&path)?;
    movie.files.push(Input {
        file: PackageFile {
            kind,
            path: relative,
            bytes: stamp.bytes,
            sha256: String::new(),
            actor: actor.map(|(name, _)| name.to_owned()),
            source: actor.and_then(|(_, url)| url.map(str::to_owned)),
        },
        path,
        confinement: confinement.to_path_buf(),
        link: None,
        expected_hash: None,
        stamp,
    });
    Ok(())
}

impl ExportPreview {
    fn warn(&mut self, message: String) {
        self.warning_count += 1;
        if self.warnings.len() < 30 {
            self.warnings.push(message);
        }
    }
}

fn package_avatar_name(actor: &str, ext: &str, used: &mut HashSet<String>) -> String {
    let name = organize::sanitize(actor);
    if !name.is_empty()
        && name.len() <= 240
        && used.insert(name.nfc().collect::<String>().to_lowercase())
    {
        return format!("{name}.{ext}");
    }
    let mut index = used.len() + 1;
    loop {
        let fallback = format!("kiroku-actor-{index:05}");
        if used.insert(fallback.clone()) {
            return format!("{fallback}.{ext}");
        }
        index += 1;
    }
}

pub fn preview(
    reader: &LibraryReader,
    codes: &[String],
    kinds: &[FileKind],
    directory: &Path,
    name: &str,
    avatar_cfg: &actor_hosting::Config,
    cancel: &CancellationToken,
) -> io::Result<ExportPlan> {
    if codes.is_empty() || kinds.is_empty() {
        return Err(io::Error::other("请至少选择一部影片和一种文件"));
    }
    if checked_name(Path::new(name))? != name || !name.starts_with("kiroku-") {
        return Err(io::Error::other("资料包名称无效"));
    }
    let directory = directory.canonicalize()?;
    let root = reader.root().canonicalize()?;
    for ancestor in directory.ancestors() {
        if same_file::is_same_file(ancestor, &root)? {
            return Err(io::Error::other("请选择当前媒体库之外的导出目录"));
        }
    }
    let dest = directory.join(name);
    let parent = RootGuard::capture(&directory, false)?;
    if organize::occupied(&dest) {
        return Err(io::Error::other("目标已存在，请重新选择导出位置"));
    }
    let mut summary = ExportPreview {
        destination: dest,
        movies: 0,
        files: 0,
        bytes: 0,
        examples: vec![],
        warnings: vec![],
        warning_count: 0,
    };
    let mut movies = Vec::new();
    let mut seen = HashSet::new();
    for code in codes {
        append::check_cancel(cancel)?;
        if !seen.insert(code) {
            continue;
        }
        let detail = reader
            .get(code)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::other(format!("库中已找不到 {code}，请重新选择")))?;
        let mut movie = MoviePlan {
            code: code.clone(),
            files: vec![],
        };
        let prefix = format!(
            "movies/{:05}-{}",
            movies.len() + 1,
            organize::sanitize(code)
                .chars()
                .take(70)
                .collect::<String>()
        );
        let dir = append::confined_path(reader.root(), Path::new(&detail.dir))?;
        let meta = append::read_meta(&dir)?;
        if !meta.code.eq_ignore_ascii_case(code) {
            return Err(io::Error::other(format!(
                "{code} 目录的编号已变化，请刷新媒体库"
            )));
        }
        let assets = assets::Paths::for_movie(&dir, &meta)?;
        for (kind, path) in [
            (FileKind::Nfo, assets.nfo),
            (FileKind::Poster, assets.poster),
            (FileKind::Fanart, assets.fanart),
            (FileKind::Sidecar, dir.join(organize::SIDECAR)),
        ] {
            if !kinds.contains(&kind) {
                continue;
            }
            let path = match kind {
                FileKind::Nfo if !organize::occupied(&path) => Some(assets::nfo_path(&dir)?),
                FileKind::Poster | FileKind::Fanart => assets::existing_art(&path)?,
                _ => Some(path),
            };
            if let Some(file) = path.filter(|p| organize::occupied(p)) {
                add_file(
                    &mut movie,
                    &file,
                    &root,
                    format!("{prefix}/{}", checked_name(&file)?),
                    kind,
                    None,
                )?;
            } else {
                summary.warn(format!("{code} 缺少{}", kind.label()));
            }
        }
        if kinds.contains(&FileKind::Avatar) {
            let mut names = HashSet::new();
            for a in &meta.fields.actors {
                append::check_cancel(cancel)?;
                let source = a.thumb.as_deref();
                let stem = organize::sanitize(&a.name);
                let local = if stem.is_empty() || stem.len() > 240 {
                    None
                } else {
                    crate::supplement::local_avatar_path(&dir, &a.name)?
                };
                let hosted = source
                    .and_then(|url| meta.hosted_actor_images.get(url))
                    .filter(|_| avatar_cfg.enabled);
                let (file, confinement, expected_hash) = if let Some(local) = local {
                    (local, root.clone(), None)
                } else if let Some(image) = hosted {
                    if actor_hosting::cached_avatar(&meta, source.unwrap_or_default(), avatar_cfg)?
                        .is_none()
                    {
                        summary.warn(format!("{code} · {} 缺少本地头像", a.name));
                        continue;
                    }
                    let folder = PathBuf::from(&avatar_cfg.directory);
                    (
                        folder.join(&image.file),
                        folder,
                        image.file.strip_suffix(".png").map(str::to_owned),
                    )
                } else {
                    summary.warn(format!("{code} · {} 缺少本地头像", a.name));
                    continue;
                };
                let ext = file
                    .extension()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| io::Error::other("头像缺少扩展名"))?;
                let name = package_avatar_name(&a.name, ext, &mut names);
                add_file(
                    &mut movie,
                    &file,
                    &confinement,
                    format!("{prefix}/.actors/{name}"),
                    FileKind::Avatar,
                    Some((&a.name, source)),
                )?;
                movie.files.last_mut().expect("just added").expected_hash = expected_hash;
            }
        }
        if kinds.contains(&FileKind::Video) || kinds.contains(&FileKind::Subtitle) {
            let mut dirs = vec![dir.clone()];
            let mut subtitles = BTreeSet::new();
            for file in &detail.files {
                append::check_cancel(cancel)?;
                let video = Path::new(&file.path);
                let folder = append::confined_path(
                    reader.root(),
                    video
                        .parent()
                        .ok_or_else(|| io::Error::other("缺少影片目录"))?,
                )?;
                let idx = dirs.iter().position(|p| p == &folder).unwrap_or_else(|| {
                    dirs.push(folder.clone());
                    dirs.len() - 1
                });
                let folder_prefix = if idx == 0 {
                    prefix.clone()
                } else {
                    format!("{prefix}/part-{}", idx + 1)
                };
                let video = folder.join(checked_name(video)?);
                if kinds.contains(&FileKind::Video) {
                    let sibling_meta;
                    let source_meta = if idx == 0 {
                        &meta
                    } else {
                        sibling_meta = append::read_meta(&folder)?;
                        if !sibling_meta.code.eq_ignore_ascii_case(code) {
                            return Err(io::Error::other("视频分片目录的编号已变化"));
                        }
                        &sibling_meta
                    };
                    let resolved = placement::resolve_media_observed(
                        &root,
                        &folder,
                        &video,
                        source_meta,
                        cancel,
                        &|_, _| {},
                    )?;
                    let confinement = if resolved.source_proof.is_some() {
                        resolved
                            .path
                            .parent()
                            .ok_or_else(|| io::Error::other("媒体路径无效"))?
                    } else {
                        &root
                    };
                    add_file(
                        &mut movie,
                        &resolved.path,
                        confinement,
                        format!("{folder_prefix}/{}", checked_name(&video)?),
                        FileKind::Video,
                        None,
                    )?;
                    if let Some(proof) = resolved.source_proof {
                        movie.files.last_mut().expect("just added").link =
                            Some((video.clone(), proof));
                    }
                }
                if kinds.contains(&FileKind::Subtitle) {
                    if let Some(mut parsed) = crate::code::parse_code(code) {
                        parsed.part = file.part;
                        for sibling in subtitle::siblings(&video, &parsed) {
                            if subtitles.insert(sibling.from.clone()) {
                                let name = checked_name(&sibling.from)?;
                                add_file(
                                    &mut movie,
                                    &sibling.from,
                                    &root,
                                    format!("{folder_prefix}/{name}"),
                                    FileKind::Subtitle,
                                    None,
                                )?;
                            }
                        }
                    }
                }
            }
        }
        if movie.files.is_empty() {
            summary.warn(format!("{code} 没有可导出文件，已跳过"));
            continue;
        }
        for input in &movie.files {
            summary.files += 1;
            summary.bytes = summary
                .bytes
                .checked_add(input.stamp.bytes)
                .ok_or_else(|| io::Error::other("容量超出范围"))?;
            if summary.examples.len() < 12 {
                summary.examples.push(input.file.path.clone());
            }
        }
        movies.push(movie);
    }
    if movies.is_empty() {
        return Err(io::Error::other("所选影片没有可导出的本地文件"));
    }
    append::check_cancel(cancel)?;
    summary.movies = movies.len();
    Ok(ExportPlan {
        preview: summary,
        parent,
        library: RootGuard::capture(&root, false)?,
        movies,
    })
}

impl FileKind {
    fn label(self) -> &'static str {
        match self {
            Self::Nfo => "NFO",
            Self::Poster => "海报",
            Self::Fanart => "背景图",
            Self::Sidecar => "kiroku.json",
            Self::Avatar => "头像",
            Self::Video => "视频",
            Self::Subtitle => "字幕",
        }
    }
}

/// Creates an exclusive directory; a manifest identifies a complete export. Never remove
/// a user's replacement or a file subsequently added to the destination.
enum CreatedIdentity {
    Persistent(String),
    Open(same_file::Handle),
}
impl CreatedIdentity {
    fn capture(file: &fs::File) -> io::Result<Self> {
        if let Ok(id) = crate::durable_fs::identity_file(file) {
            return Ok(Self::Persistent(id));
        }
        same_file::Handle::from_file(file.try_clone()?).map(Self::Open)
    }
    fn matches(&self, path: &Path) -> bool {
        match self {
            Self::Persistent(id) => crate::durable_fs::identity(path).as_ref().ok() == Some(id),
            Self::Open(handle) => same_file::Handle::from_path(path).as_ref().ok() == Some(handle),
        }
    }
}
struct Output {
    path: PathBuf,
    guard: RootGuard,
    created: Vec<(PathBuf, CreatedIdentity)>,
    folders: BTreeSet<PathBuf>,
}
impl Output {
    fn new(path: &Path) -> io::Result<Self> {
        fs::create_dir(path)?;
        Ok(Self {
            path: path.into(),
            guard: RootGuard::capture(path, false)?,
            created: vec![],
            folders: BTreeSet::new(),
        })
    }
    fn file(&mut self, relative: &str) -> io::Result<fs::File> {
        self.guard.verify()?;
        let path = append::confined_path(&self.path, &self.path.join(relative))?;
        let parents = path
            .ancestors()
            .skip(1)
            .take_while(|p| *p != self.path)
            .collect::<Vec<_>>();
        for parent in parents.into_iter().rev() {
            if !self.folders.contains(parent) {
                fs::create_dir(parent)?;
                self.folders.insert(parent.to_path_buf());
            }
        }
        append::confined_path(&self.path, &path)?;
        let f = fs::File::create_new(&path)?;
        self.created.push((path, CreatedIdentity::capture(&f)?));
        Ok(f)
    }
    fn cleanup(self) {
        if self.guard.verify().is_err() {
            return;
        }
        for (file, handle) in self.created.into_iter().rev() {
            if append::confined_path(&self.path, &file).is_err() {
                continue;
            }
            if handle.matches(&file) {
                let _ = fs::remove_file(&file);
            }
        }
        for p in self.folders.iter().rev() {
            let _ = fs::remove_dir(p);
        }
        let _ = fs::remove_dir(self.path);
    }
}

pub fn export(
    plan: ExportPlan,
    cancel: &CancellationToken,
    progress: impl Fn(u64, u64),
) -> io::Result<ExportPreview> {
    plan.parent.verify()?;
    plan.library.verify()?;
    append::check_cancel(cancel)?;
    let mut output = Output::new(&plan.preview.destination)?;
    let result = (|| {
        let mut manifest = Manifest {
            version: 1,
            movies: vec![],
        };
        let mut done = 0;
        for movie in plan.movies {
            let mut files = vec![];
            for input in movie.files {
                append::check_cancel(cancel)?;
                plan.parent.verify()?;
                append::confined_path(&input.confinement, &input.path)?;
                if SourceStamp::read(&input.path)? != input.stamp {
                    return Err(io::Error::other(format!(
                        "{} 已变化，请重新预览",
                        input.file.path
                    )));
                }
                if let Some((link, _)) = &input.link {
                    if fs::read_link(link)? != input.path {
                        return Err(io::Error::other("媒体链接已变化，请重新预览"));
                    }
                }
                let mut file = input.file;
                let mut dest = output.file(&file.path)?;
                let proof = append::fingerprint_stream(
                    &input.path,
                    &|| cancel.is_cancelled(),
                    &|n, _| progress(done + n, plan.preview.bytes),
                    Some(&mut dest),
                )?;
                if SourceStamp::read(&input.path)? != input.stamp {
                    return Err(io::Error::other("导出期间来源已变化，请重新预览"));
                }
                if let Some((link, expected)) = &input.link {
                    if fs::read_link(link)? != input.path || &proof != expected {
                        return Err(io::Error::other("媒体链接校验失败，请重新预览"));
                    }
                }
                dest.sync_all()?;
                file.sha256 = proof.rsplit(':').next().unwrap_or_default().to_string();
                if input
                    .expected_hash
                    .as_ref()
                    .is_some_and(|expected| expected != &file.sha256)
                {
                    return Err(io::Error::other("托管头像的内容与登记摘要不符"));
                }
                done += file.bytes;
                files.push(file);
            }
            manifest.movies.push(PackageMovie {
                code: movie.code,
                files,
            });
        }
        append::check_cancel(cancel)?;
        plan.parent.verify()?;
        plan.library.verify()?;
        let mut out = output.file(MANIFEST_NAME)?;
        serde_json::to_writer_pretty(&mut out, &manifest)?;
        out.write_all(b"\n")?;
        out.sync_all()?;
        #[cfg(unix)]
        crate::durable_fs::sync_directory(&output.path)?;
        Ok(())
    })();
    if let Err(e) = result {
        output.cleanup();
        return Err(e);
    }
    Ok(plan.preview)
}
