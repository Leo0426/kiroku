//! Optional immutable avatar cache for an independently managed static file service.
use crate::{journal::replacement::digest, pipeline::RootGuard, settings::Settings, Metadata};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io::{self, Write}, path::{Path, PathBuf}};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config { pub enabled: bool, pub directory: String, pub base_url: String }
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled { return Ok(()); }
        if self.directory.trim().is_empty() || !Path::new(&self.directory).is_absolute() {
            return Err("头像保存目录须为本机可访问的绝对路径".into());
        }
        self.base().map(|_| ())
    }
    pub(crate) fn public_url(&self, file: &str) -> io::Result<String> {
        Ok(self.base().map_err(io::Error::other)?.join(file).map_err(io::Error::other)?.to_string())
    }
    fn base(&self) -> Result<reqwest::Url, String> {
        let mut u = reqwest::Url::parse(self.base_url.trim()).map_err(|_| "头像 URL 前缀无效")?;
        if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() || !u.username().is_empty() || u.password().is_some() || u.query().is_some() || u.fragment().is_some() {
            return Err("头像 URL 前缀须为不含凭据、查询参数或片段的 HTTP(S) 地址".into());
        }
        if !u.path().ends_with('/') { u.set_path(&format!("{}/", u.path())); }
        Ok(u)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedImage { pub file: String, pub url: String }
#[derive(Debug, Clone)]
pub struct CachePlan { directory: PathBuf, guard: RootGuard, files: BTreeMap<String, Vec<u8>> }
fn valid_file(name: &str) -> bool {
    name.strip_suffix(".png").is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
}
fn existing(dir: &Path, file: &str) -> io::Result<Option<Vec<u8>>> {
    if !valid_file(file) { return Err(io::Error::other("头像缓存文件名无效")); }
    let path = dir.join(file);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
        Ok(m) if !m.is_file() || m.file_type().is_symlink() || m.len() > 8 * 1024 * 1024 => return Err(io::Error::other("头像缓存文件类型或大小无效")),
        _ => {}
    }
    let bytes = fs::read(path)?;
    if format!("{}.png", digest(&bytes)) != file { return Err(io::Error::other("头像缓存已被外部修改")); }
    Ok(Some(bytes))
}
impl CachePlan {
    pub(crate) fn image(&self, file: &str) -> Option<&[u8]> {
        self.files.get(file).map(Vec::as_slice)
    }

    /// Assets are a shared immutable cache: rollback of metadata intentionally retains them.
    pub fn publish(&self) -> io::Result<()> {
        self.guard.verify()?;
        for (file, bytes) in &self.files {
            self.guard.verify()?;
            if existing(&self.directory, file)?.is_some() { continue; }
            let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
            temp.write_all(bytes)?;
            #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; temp.as_file().set_permissions(fs::Permissions::from_mode(0o644))?; }
            temp.as_file().sync_all()?;
            self.guard.verify()?;
            if let Err(e) = temp.persist_noclobber(self.directory.join(file)) {
                if existing(&self.directory, file)?.is_none() { return Err(e.error); }
            }
            #[cfg(unix)]
            crate::durable_fs::sync_directory(&self.directory)?;
        }
        self.guard.verify()
    }
}

/// Read the verified shared PNG for a source URL associated with this movie.
pub fn cached_avatar(meta: &Metadata, source: &str, cfg: &Config) -> io::Result<Option<Vec<u8>>> {
    if !cfg.enabled { return Ok(None); }
    cfg.validate().map_err(io::Error::other)?;
    let Some(image) = meta.hosted_actor_images.get(source) else { return Ok(None); };
    existing(Path::new(&cfg.directory), &image.file)
}

/// Download into memory only. The directory must already exist (never recreate a missing NAS mount).
pub async fn prepare(before: &Metadata, settings: &Settings, cancel: &CancellationToken) -> io::Result<(Metadata, CachePlan, Vec<String>)> {
    let cfg = &settings.actor_hosting;
    if !cfg.enabled { return Err(io::Error::other("请先在设置 → 媒体服务器中启用演员头像托管")); }
    cfg.validate().map_err(io::Error::other)?;
    let base = cfg.base().map_err(io::Error::other)?;
    let directory = PathBuf::from(&cfg.directory);
    let captured_dir = directory.clone();
    let guard = tokio::task::spawn_blocking(move || RootGuard::capture(&captured_dir, false)).await.map_err(io::Error::other)??;
    let mut plan = CachePlan { directory, guard, files: BTreeMap::new() };
    let mut after = before.clone();
    let mut warnings = vec![];
    let mut done = BTreeMap::new();
    for (index, actor) in before.fields.actors.iter().enumerate() {
        if cancel.is_cancelled() { return Err(io::Error::other("已取消")); }
        let Some(source) = actor.thumb.as_deref().filter(|s| !s.is_empty()) else { warnings.push(format!("第 {} 位演员没有来源头像，已跳过", index + 1)); continue; };
        if done.contains_key(source) { continue; }
        let cached = if let Some(old) = before.hosted_actor_images.get(source) {
            let dir = plan.directory.clone(); let name = old.file.clone();
            tokio::task::spawn_blocking(move || existing(&dir, &name)).await.map_err(io::Error::other)??
        } else { None };
        let bytes = if let Some(bytes) = cached { bytes } else {
            let mut http = settings.build_http().map_err(|_| io::Error::other("无法创建图片请求"))?.for_images();
            if let Ok(site) = settings.source_for_url(source) {
                http = http.with_proxy(settings.proxy_for(site)).map_err(|_| io::Error::other("无法创建图片请求"))?;
                http.seed_cookies(&site.base, &site.cookie);
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err(io::Error::other("已取消")),
                result = crate::actor_image::fetch(&http, source) => result?,
            }
        };
        let file = format!("{}.png", digest(&bytes));
        let url = base.join(&file).map_err(io::Error::other)?.to_string();
        plan.files.insert(file.clone(), bytes);
        done.insert(source.to_owned(), HostedImage { file, url });
    }
    if done.is_empty() { return Err(io::Error::other("没有可发布的演员头像，请先更新演员来源")); }
    after.hosted_actor_images = done;
    warnings.push("确认后写入头像缓存并更新 NFO；撤销只恢复 NFO 和元数据，共享头像缓存保留。保存后请在 Plex 刷新本片元数据。".into());
    Ok((after, plan, warnings))
}

/// Rebind portable avatar bytes to this installation's immutable cache and URL prefix.
pub(crate) fn from_package(meta: &mut Metadata, cfg: &Config, images: BTreeMap<String, Vec<u8>>) -> io::Result<CachePlan> {
    if !cfg.enabled { return Err(io::Error::other("请先配置本机头像托管目录和 URL 前缀")); }
    cfg.validate().map_err(io::Error::other)?;
    let directory = PathBuf::from(&cfg.directory);
    let guard = RootGuard::capture(&directory, false)?;
    let base = cfg.base().map_err(io::Error::other)?;
    let mut files = BTreeMap::new();
    for (source, bytes) in images {
        if !meta.fields.actors.iter().any(|a| a.thumb.as_deref() == Some(&source)) {
            return Err(io::Error::other("头像未关联到目标影片演员"));
        }
        let (width, height) = crate::artwork::dimensions(&bytes)?;
        if width > 4096 || height > 4096 || bytes.len() > 8 * 1024 * 1024 || image::guess_format(&bytes).ok() != Some(image::ImageFormat::Png) {
            return Err(io::Error::other("资料包头像不是受支持的 PNG"));
        }
        crate::artwork::PreparedImage::normalize(&bytes)?;
        let file = format!("{}.png", digest(&bytes));
        let url = base.join(&file).map_err(io::Error::other)?.to_string();
        meta.hosted_actor_images.insert(source, HostedImage { file: file.clone(), url });
        files.insert(file, bytes);
    }
    Ok(CachePlan { directory, guard, files })
}
