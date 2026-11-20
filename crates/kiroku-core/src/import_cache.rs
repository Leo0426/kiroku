//! Disposable, crash-safe import cache; never an authority for media file operations.
use crate::{code::Code, metadata::SourceSnapshot, settings::Settings};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
const MAGIC: &[u8] = b"KIROKU-CACHE-1\n";
#[derive(Clone)]
pub struct ImportCache {
    root: PathBuf,
}
pub fn cache_path(settings: &Path) -> PathBuf {
    settings
        .parent()
        .unwrap_or(Path::new("."))
        .join("cache/import-v1")
}
pub fn image_scope(settings: &Settings) -> String {
    hash(&serde_json::to_vec(&(&settings.sources, &settings.proxy)).expect("settings serialize"))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
impl ImportCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn path(&self, kind: &str, key: &str) -> PathBuf {
        self.root.join(kind).join(hash(key.as_bytes()))
    }
    pub(crate) fn read(&self, kind: &str, key: &str, limit: usize) -> Option<Vec<u8>> {
        self.read_checked(kind, key, limit).ok()
    }
    fn read_checked(&self, kind: &str, key: &str, limit: usize) -> io::Result<Vec<u8>> {
        let mut file = fs::File::open(self.path(kind, key))?;
        let invalid = || io::Error::new(io::ErrorKind::InvalidData, "缓存内容校验失败");
        if file.metadata()?.len() > (limit + MAGIC.len() + 64) as u64 {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take((limit + MAGIC.len() + 65) as u64)
            .read_to_end(&mut bytes)?;
        let body = bytes.get(MAGIC.len() + 64..).ok_or_else(invalid)?;
        if !bytes.starts_with(MAGIC)
            || body.len() > limit
            || bytes.get(MAGIC.len()..MAGIC.len() + 64) != Some(hash(body).as_bytes())
        {
            return Err(invalid());
        }
        Ok(body.to_vec())
    }
    pub(crate) fn write(&self, kind: &str, key: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(kind, key);
        let parent = path.parent().unwrap();
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(MAGIC)?;
        file.write_all(hash(bytes).as_bytes())?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        file.persist(&path).map_err(|e| e.error)?;
        #[cfg(unix)]
        crate::durable_fs::sync_directory(parent)?;
        Ok(())
    }
    fn source_key(code: &Code, site: &str) -> String {
        serde_json::to_string(&(&code.id, code.content_type, site)).unwrap()
    }
    pub(crate) fn source(&self, code: &Code, site: &str) -> io::Result<SourceSnapshot> {
        serde_json::from_slice(&self.read_checked(
            "sources",
            &Self::source_key(code, site),
            4 * 1024 * 1024,
        )?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "来源缓存内容无效"))
    }
    pub fn usage(&self, cancel: &tokio_util::sync::CancellationToken) -> io::Result<CacheUsage> {
        let mut usage = CacheUsage::default();
        for (kind, size) in [
            ("sources", &mut usage.sources),
            ("images", &mut usage.images),
        ] {
            crate::append::check_cancel(cancel)?;
            let path = self.root.join(kind);
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(io::Error::other("缓存目录不是普通目录"));
            }
            for entry in fs::read_dir(path)? {
                crate::append::check_cancel(cancel)?;
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if metadata.is_file() && !metadata.file_type().is_symlink() {
                    size.files += 1;
                    size.bytes = size.bytes.saturating_add(metadata.len());
                }
            }
        }
        Ok(usage)
    }
    pub(crate) fn save_source(
        &self,
        code: &Code,
        site: &str,
        snapshot: &SourceSnapshot,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec(snapshot)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(io::Error::other("来源缓存超过大小限制"));
        }
        self.write("sources", &Self::source_key(code, site), &bytes)
    }
    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheSummary {
    pub reused: usize,
    pub missing: usize,
    pub invalidated: usize,
    pub corrupt: usize,
    pub unreadable: usize,
    pub skipped_movies: usize,
}
impl CacheSummary {
    pub(crate) fn merge(&mut self, other: &Self) {
        self.reused += other.reused;
        self.missing += other.missing;
        self.invalidated += other.invalidated;
        self.corrupt += other.corrupt;
        self.unreadable += other.unreadable;
        self.skipped_movies += other.skipped_movies;
    }
}
#[derive(Debug, Default, Serialize)]
pub struct CacheSize {
    pub files: u64,
    pub bytes: u64,
}
#[derive(Debug, Default, Serialize)]
pub struct CacheUsage {
    pub sources: CacheSize,
    pub images: CacheSize,
}
