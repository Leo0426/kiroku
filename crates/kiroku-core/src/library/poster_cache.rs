//! Verified local poster copies are shared by content, but each lookup must be
//! authorized against the requesting movie's current library index.
use super::{LibraryReader, Result};
use rusqlite::OptionalExtension;
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

const EXTENSIONS: [&str; 3] = ["jpg", "png", "webp"];

fn verified_bytes(path: &Path, version: &str) -> io::Result<Option<(Vec<u8>, &'static str)>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let limit = crate::net::IMAGE_RESPONSE_LIMIT as u64;
    if !metadata.is_file() || metadata.len() > limit {
        return Ok(None);
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit || crate::journal::replacement::digest(&bytes) != version {
        return Ok(None);
    }
    let extension = match image::guess_format(&bytes) {
        Ok(image::ImageFormat::Jpeg) => "jpg",
        Ok(image::ImageFormat::Png) => "png",
        Ok(image::ImageFormat::WebP) => "webp",
        _ => return Ok(None),
    };
    Ok(Some((bytes, extension)))
}

fn cache_directory(config: &Path, create: bool) -> io::Result<Option<PathBuf>> {
    if create {
        fs::create_dir_all(config)?;
    }
    let mut dir = config.to_path_buf();
    for part in ["cache", "posters-v1"] {
        dir.push(part);
        if create {
            match fs::create_dir(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        match fs::symlink_metadata(&dir) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(io::Error::other("海报缓存目录不可用")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
    }
    Ok(Some(dir))
}

impl LibraryReader {
    /// Only the exact hash currently associated with this movie is eligible.
    /// A cache hit needs no access to the original media directory.
    pub fn cached_poster(&self, config: &Path, code: &str, version: Option<&str>) -> Result<Option<PathBuf>> {
        let Some(version) = version.filter(|hash| {
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }) else {
            return Ok(None);
        };
        let indexed = || {
            self.snapshot(|library| {
                Ok(library
                    .conn
                    .query_row(
                        "SELECT poster FROM movie WHERE code=?1 AND poster_version=?2 AND poster IS NOT NULL",
                        [code, version],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?)
            })
        };
        let Some(relative) = indexed()? else { return Ok(None) };
        let mut hit = None;
        if let Some(cache) = cache_directory(config, false)? {
            for ext in EXTENSIONS {
                let path = cache.join(format!("{version}.{ext}"));
                if verified_bytes(&path, version)?.is_some_and(|(_, actual)| actual == ext) {
                    hit = Some(path);
                    break;
                }
            }
        }
        if hit.is_none() {
            let source = self.root().join(&relative);
            crate::append::confined_path(self.root(), &source)?;
            if let Some((bytes, ext)) = verified_bytes(&source, version)? {
                let cache =
                    cache_directory(config, true)?.ok_or_else(|| io::Error::other("海报缓存目录已变化"))?;
                let destination = cache.join(format!("{version}.{ext}"));
                let mut temp = tempfile::NamedTempFile::new_in(&cache)?;
                temp.write_all(&bytes)?;
                temp.persist(&destination).map_err(|e| e.error)?;
                hit = Some(destination);
            }
        }
        // A concurrent edit or removal revokes the old cache lookup too.
        if hit.is_some() && indexed()?.is_none() {
            return Ok(None);
        }
        Ok(hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;

    #[test]
    fn exact_indexed_poster_is_cached_and_survives_a_missing_movie_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("library");
        let config = temporary.path().join("config");
        let movie = root.join("example");
        fs::create_dir_all(&movie).unwrap();
        let meta = crate::Metadata {
            code: "AAA-111".into(),
            ..Default::default()
        };
        fs::write(movie.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
        fs::write(movie.join("AAA-111.mp4"), b"video").unwrap();
        let mut bytes = io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 1)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        fs::write(movie.join("poster.jpg"), &bytes).unwrap();
        let version = crate::journal::replacement::digest(&bytes);
        let library = Library::open(&root).unwrap();
        library.reindex_dir(&movie).unwrap();
        let reader = library.reader_handle().open().unwrap();
        let lookup = |code, version| reader.cached_poster(&config, code, version).unwrap();
        assert!(lookup("AAA-111", None).is_none());
        assert!(lookup("AAA-111", Some("../escape")).is_none());
        assert!(lookup("BBB-222", Some(&version)).is_none());
        let cache = lookup("AAA-111", Some(&version)).unwrap();
        assert_eq!(cache, config.join("cache/posters-v1").join(format!("{version}.png")));
        assert_eq!(fs::read(&cache).unwrap(), bytes);
        // Even a cache file whose name is correct must verify against its bytes.
        fs::write(&cache, b"invalid").unwrap();
        assert_eq!(lookup("AAA-111", Some(&version)), Some(cache.clone()));
        assert_eq!(fs::read(&cache).unwrap(), bytes);
        fs::rename(&movie, temporary.path().join("away")).unwrap();
        assert_eq!(lookup("AAA-111", Some(&version)), Some(cache.clone()));
        fs::write(&cache, b"invalid").unwrap();
        assert!(lookup("AAA-111", Some(&version)).is_none());
        fs::write(&cache, &bytes).unwrap();
        library.reindex_dir(&movie).unwrap();
        assert!(lookup("AAA-111", Some(&version)).is_none());
    }
}
