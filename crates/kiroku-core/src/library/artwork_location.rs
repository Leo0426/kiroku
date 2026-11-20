use super::{optional_metadata_text, MovieDirectory};
use crate::{append, assets, nfo_read};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtworkLocation {
    pub root: PathBuf,
    pub dir: PathBuf,
}

/// The original relative directory is an identity hint. Only configured source
/// roots can supply a replacement; a filename or a poster on its own cannot.
pub fn resolve_artwork_location(
    active_root: &Path,
    indexed_dir: &Path,
    code: &str,
    source_dirs: &[String],
    cancel: &CancellationToken,
) -> io::Result<ArtworkLocation> {
    append::check_cancel(cancel)?;
    let active = active_root.canonicalize()?;
    let dir = append::confined_path(active_root, indexed_dir)?;
    match fs::symlink_metadata(&dir) {
        Ok(metadata) if metadata.is_dir() => return Ok(ArtworkLocation { root: active, dir }),
        Ok(_) => return Err(io::Error::other("原影片路径不是文件夹")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let relative = dir.strip_prefix(&active).map_err(io::Error::other)?;
    let mut roots = HashSet::new();
    let mut found = None;
    for setting in source_dirs {
        append::check_cancel(cancel)?;
        let path = Path::new(setting);
        if !path.is_absolute() {
            continue;
        }
        let root = match path.canonicalize() {
            Ok(root) => root,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if root == active || !roots.insert(root.clone()) {
            continue;
        }
        let candidate = append::confined_path(&root, &root.join(relative))?;
        let contents = MovieDirectory::read(&candidate, cancel)?;
        if !contents
            .videos
            .iter()
            .any(|v| v.file_type().is_ok_and(|t| t.is_file()))
        {
            continue;
        }
        // Sidecar and explicit NFO number must agree when both are present.
        // Neither the directory name nor a fallback NFO number authorizes a write.
        let mut identity = None;
        let side = append::confined_path(&root, &candidate.join(crate::organize::SIDECAR))?;
        if let Some(json) = optional_metadata_text(&side)? {
            let movie: serde_json::Value = serde_json::from_str(&json)?;
            if movie["code"].as_str() != Some(code) {
                continue;
            }
            identity = Some(code);
        }
        let nfo = append::confined_path(&root, &assets::nfo_path(&candidate)?)?;
        if let Some(xml) = optional_metadata_text(&nfo)? {
            let Some(movie) = nfo_read::parse_nfo(&xml, "") else {
                continue;
            };
            if !movie.code.is_empty() {
                if movie.code != code {
                    continue;
                }
                identity = Some(code);
            }
        }
        if identity.is_none() {
            continue;
        }
        if found.is_some() {
            return Err(io::Error::other(
                "多个来源目录中都有这部影片，请核对来源目录后再裁剪",
            ));
        }
        found = Some(ArtworkLocation {
            root,
            dir: candidate,
        });
    }
    found.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "找不到影片的原图目录。请连接媒体盘，并核对「待整理目录」中影片的位置。",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_original_can_use_one_verified_source() {
        let temp = tempfile::tempdir().unwrap();
        let lib = temp.path().join("library");
        fs::create_dir(&lib).unwrap();
        let mut sources = Vec::new();
        let old = lib.join("group/movie");
        let cancel = CancellationToken::new();
        for name in ["first", "second"] {
            let root = temp.path().join(name);
            let dir = root.join("group/movie");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("clip.mp4"), b"test").unwrap();
            fs::write(dir.join("movie.nfo"), b"<movie><num>EX-101</num></movie>").unwrap();
            sources.push(root.to_string_lossy().into_owned());
            let found = resolve_artwork_location(&lib, &old, "EX-101", &sources, &cancel);
            if name == "first" {
                assert_eq!(found.unwrap().dir, dir.canonicalize().unwrap());
            } else {
                assert!(found.is_err());
                fs::write(dir.join("kiroku.json"), br#"{"code":"EX-999"}"#).unwrap();
                assert!(resolve_artwork_location(&lib, &old, "EX-101", &sources, &cancel).is_ok());
            }
        }
        fs::create_dir_all(&old).unwrap();
        assert_eq!(
            resolve_artwork_location(&lib, &old, "EX-101", &sources, &cancel)
                .unwrap()
                .root,
            lib.canonicalize().unwrap()
        );
        cancel.cancel();
        assert!(resolve_artwork_location(&lib, &old, "EX-101", &sources, &cancel).is_err());
    }
}
