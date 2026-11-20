//! Keep a single-movie directory's Plex poster in the same revision as its named cover.
use super::{digest, read, within, Preview, ReplacementInput};
use image::ImageFormat;
use std::{fs, io, io::Cursor, path::Path};

fn poster_format(path: &Path) -> Option<ImageFormat> {
    if !path.file_stem()?.eq_ignore_ascii_case("poster") {
        return None;
    }
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "png" => Some(ImageFormat::Png),
        "webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

// A PNG copy of a JPEG crop has different bytes but exactly the same decoded pixels.
fn pixels(bytes: &[u8]) -> Option<image::DynamicImage> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (w, h) = reader.into_dimensions().ok()?;
    if u64::from(w) * u64::from(h) > 32 * 1024 * 1024 {
        return None;
    }
    image::load_from_memory(bytes).ok()
}

impl Preview {
    /// Attach a Plex-compatible poster iff this revision includes a named poster.
    /// An unrelated existing cover stays in place and is reported in the preview.
    pub fn with_plex_poster(mut self) -> io::Result<Self> {
        let named = crate::assets::Paths::for_movie(&self.dir, &self.before)?.poster;
        if poster_format(&named).is_some() {
            return Ok(self);
        }
        let Some(input) = self.inputs.iter().find(|input| input.path == named) else {
            return Ok(self);
        };
        let mut covers = Vec::new();
        let mut unknown_videos = 0;
        let mut videos = 0;
        for item in fs::read_dir(&self.dir)? {
            let entry = item?;
            let path = entry.path();
            if let Some(format) = poster_format(&path) {
                covers.push((path, format));
            } else if path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|ext| {
                    crate::pipeline::VIDEO_EXT
                        .iter()
                        .any(|v| ext.eq_ignore_ascii_case(v))
                })
            {
                videos += 1;
                match crate::parse_code(&entry.file_name().to_string_lossy()) {
                    Some(parsed) if parsed.id != self.code => {
                        self.warnings
                            .push("目录中的视频属于不同影片，已保留现有 Plex 封面。".into());
                        return Ok(self);
                    }
                    None => unknown_videos += 1,
                    _ => {}
                }
            }
        }
        if videos == 0 || (videos > 1 && unknown_videos > 0) || covers.len() > 1 {
            self.warnings
                .push("无法确认唯一的 Plex 封面，已保留目录中的图片。".into());
            return Ok(self);
        }
        let (path, format) = covers
            .pop()
            .unwrap_or_else(|| (self.dir.join("poster.jpg"), ImageFormat::Jpeg));
        let expected = match fs::symlink_metadata(&path) {
            Ok(meta) if !meta.is_file() || meta.len() > crate::net::IMAGE_RESPONSE_LIMIT as u64 => {
                self.warnings
                    .push("已有 Plex 封面无法安全替换，已保留。".into());
                return Ok(self);
            }
            Ok(_) => {
                within(&self.root, &path)?;
                let alias =
                    read(&path)?.ok_or_else(|| io::Error::other("Plex 封面已变化，请重新预览"))?;
                let hash = digest(&alias);
                if input.expected.as_ref() != Some(&hash) {
                    let matches = if let Some(existing) = read(&named)? {
                        if input.expected.as_ref() != Some(&digest(&existing)) {
                            return Err(io::Error::other("海报已变化，请重新预览"));
                        }
                        match (pixels(&existing), pixels(&alias)) {
                            (Some(a), Some(b)) => a.to_rgba8() == b.to_rgba8(),
                            _ => false,
                        }
                    } else {
                        false
                    };
                    if !matches {
                        self.warnings
                            .push("目录已有另一张 Plex 封面，已保留该图。".into());
                        return Ok(self);
                    }
                }
                Some(hash)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        within(&self.root, &path)?;
        let bytes = if image::guess_format(&input.bytes).ok() == Some(format) {
            input.bytes.clone()
        } else {
            let image =
                pixels(&input.bytes).ok_or_else(|| io::Error::other("无法解码 Plex 封面"))?;
            let mut out = Cursor::new(Vec::new());
            image.write_to(&mut out, format).map_err(io::Error::other)?;
            out.into_inner()
        };
        self.inputs.push(ReplacementInput {
            path,
            expected,
            bytes,
        });
        Ok(self)
    }
}
