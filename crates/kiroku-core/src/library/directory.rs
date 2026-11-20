use super::{optional_metadata_text, Metadata, SIDECAR, VIDEO_EXT};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

/// Collect names once so refresh and reindex identify a movie and its artwork by
/// the same rules. Video links are recorded without following offline targets.
#[derive(Default)]
pub(super) struct MovieDirectory {
    pub(super) videos: Vec<fs::DirEntry>,
    sidecar: Option<PathBuf>,
    nfo: Option<PathBuf>,
    images: BTreeMap<(String, String), PathBuf>,
}

impl MovieDirectory {
    pub(super) fn read(dir: &Path, cancel: &CancellationToken) -> io::Result<Self> {
        let mut result = Self::default();
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(result),
            Err(e) => return Err(e),
        };
        for entry in entries {
            crate::append::check_cancel(cancel)?;
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("._") {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|x| x.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if name == SIDECAR {
                if entry.file_type()?.is_file() {
                    result.sidecar = Some(path);
                }
            } else if ext == "nfo" && entry.file_type()?.is_file() {
                // Prefer the legacy movie.nfo, otherwise use stable filename order.
                if name == "movie.nfo"
                    || result
                        .nfo
                        .as_ref()
                        .is_none_or(|old| old.file_name().is_none_or(|n| n != "movie.nfo") && path < *old)
                {
                    result.nfo = Some(path);
                }
            } else if VIDEO_EXT.contains(&ext.as_str()) {
                let kind = entry.file_type()?;
                if kind.is_file() || kind.is_symlink() {
                    result.videos.push(entry);
                }
            } else if matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "webp") && entry.file_type()?.is_file() {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                result
                    .images
                    .entry((stem, ext))
                    .and_modify(|old| {
                        if path < *old {
                            old.clone_from(&path);
                        }
                    })
                    .or_insert(path);
            }
        }
        Ok(result)
    }

    pub(super) fn metadata(&self, dir: &Path) -> io::Result<Option<Metadata>> {
        if let Some(path) = &self.sidecar {
            if let Some(meta) = optional_metadata_text(path)?.and_then(|s| serde_json::from_str(&s).ok()) {
                return Ok(Some(meta));
            }
        }
        let Some(path) = &self.nfo else { return Ok(None) };
        let dir_name = dir.file_name().unwrap_or_default().to_string_lossy();
        let meta = optional_metadata_text(path)?.and_then(|s| crate::nfo_read::parse_nfo(&s, &dir_name));
        Ok(meta.map(|mut meta| {
            crate::assets::infer_from_nfo(&mut meta, path);
            meta
        }))
    }

    /// A generic cover belongs to this movie only in a single-movie directory.
    /// A cover named for an actual video or the movie's explicit asset stem can
    /// also be used in a mixed directory.
    pub(super) fn artwork(&self, meta: &Metadata) -> (Option<&Path>, Option<&Path>) {
        if self.images.is_empty() {
            return (None, None);
        }
        let mut generic = !self.videos.is_empty();
        let mut video_stems = Vec::new();
        for video in &self.videos {
            let name = video.file_name();
            let path = Path::new(&name);
            let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
            let belongs = match crate::parse_code(&name.to_string_lossy()) {
                Some(parsed) => parsed.id.eq_ignore_ascii_case(&meta.code),
                None => {
                    self.videos.len() == 1 || meta.asset_stem.as_ref().is_some_and(|s| s.to_lowercase() == stem)
                }
            };
            if belongs {
                video_stems.push(stem);
            } else {
                generic = false;
            }
        }
        video_stems.sort();
        let mut prefixes = Vec::new();
        if let Some(stem) = &meta.asset_stem {
            prefixes.push(stem.to_lowercase());
        }
        prefixes.extend(video_stems.iter().cloned());
        prefixes.push(meta.code.to_lowercase());
        let poster = crate::assets::local_image(
            &self.images,
            &prefixes,
            generic,
            &["poster", "folder", "cover"],
            &video_stems,
        );
        let fanart = crate::assets::local_image(
            &self.images,
            &prefixes,
            generic,
            &["fanart", "background", "backdrop"],
            &[],
        );
        (poster, fanart)
    }
}
