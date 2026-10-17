//! Companion paths are bound to the import's concrete video stem, never the
//! current title/template. Missing asset_stem keeps old libraries compatible.
use crate::metadata::Metadata;
use std::{fs, io, path::{Path, PathBuf}};

pub struct Paths { pub nfo: PathBuf, pub poster: PathBuf, pub fanart: PathBuf }

impl Paths {
    pub fn for_stem(dir: &Path, stem: &str) -> io::Result<Self> {
        if stem.is_empty() || stem == "." || stem == ".." || stem.contains(['/', '\\', ':']) || stem.chars().any(char::is_control) {
            return Err(io::Error::other("配套文件名无效"));
        }
        Ok(Self { nfo: dir.join(format!("{stem}.nfo")), poster: dir.join(format!("{stem}-poster.jpg")), fanart: dir.join(format!("{stem}-fanart.jpg")) })
    }
    pub fn for_movie(dir: &Path, meta: &Metadata) -> io::Result<Self> {
        match meta.asset_stem.as_deref() {
            Some(stem) => Self::for_stem(dir, stem),
            None => Ok(Self { nfo: dir.join("movie.nfo"), poster: dir.join("poster.jpg"), fanart: dir.join("fanart.jpg") }),
        }
    }
}

pub fn is_poster(path: &Path) -> bool { image_role(path, "poster") }
pub fn is_fanart(path: &Path) -> bool { image_role(path, "fanart") }
fn image_role(path: &Path, role: &str) -> bool {
    path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s == role || s.ends_with(&format!("-{role}")))
}

/// The same single-NFO rule used by revision; never choose an arbitrary file.
pub fn nfo_path(dir: &Path) -> io::Result<PathBuf> {
    let mut found = vec![];
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s.eq_ignore_ascii_case("nfo")) { found.push(path); }
    }
    if found.len() > 1 { return Err(io::Error::other("影片目录存在多个 NFO，请先明确保留一个")); }
    Ok(found.pop().unwrap_or_else(|| dir.join("movie.nfo")))
}

pub fn infer_from_nfo(meta: &mut Metadata, nfo: &Path) {
    if meta.asset_stem.is_none() && nfo.file_name().is_some_and(|s| s != "movie.nfo") {
        // Older external tools also wrote named NFOs alongside generic artwork.
        // Those existing assets keep their original paths and crop ownership.
        if let Some(dir) = nfo.parent() {
            if ["poster", "fanart"].iter().any(|role| ["jpg", "jpeg", "png", "webp"].iter()
                .any(|ext| crate::organize::occupied(&dir.join(format!("{role}.{ext}"))))) { return; }
        }
        meta.asset_stem = nfo.file_stem().and_then(|s| s.to_str()).map(String::from);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn companion_stems_cannot_escape_movie_directory() {
        for stem in ["", ".", "..", "../movie", "folder/movie", "folder\\movie", "movie\0"] {
            assert!(Paths::for_stem(Path::new("/library/movie"), stem).is_err());
        }
        assert!(Paths::for_stem(Path::new("/library/movie"), "ABC-123-title.with.dots").is_ok());
    }
}
