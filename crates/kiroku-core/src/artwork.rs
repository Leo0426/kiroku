//! Immutable, content-addressed crop originals; all mutations use revision transactions.
use crate::{
    badge::{Phase, Stamping},
    journal::replacement::digest,
    revision::{self, Preview},
    Metadata,
};
use image::ImageDecoder;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Cursor},
    path::Path,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artwork {
    pub original: String,
    pub digest: String,
    pub source_url: Option<String>,
    pub crop: Option<CropBox>,
    pub phase: Phase,
}
pub fn original_name(hash: &str) -> String {
    format!(".kiroku-original-{hash}.png")
}
/// Decode once and remove orientation/animation metadata. GUI and crop share this static plane.
pub fn normalize(bytes: &[u8]) -> io::Result<Vec<u8>> {
    normalize_plane(bytes, false).map(|(bytes, _)| bytes)
}

fn normalize_plane(bytes: &[u8], bounded: bool) -> io::Result<(Vec<u8>, (u32, u32))> {
    // New download preparation has a smaller budget. Persisted originals keep
    // the existing validation/crop contract, including previously accepted sizes.
    const WORK_BYTES: u64 = 64 * 1024 * 1024;
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    if bounded {
        let mut header = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(WORK_BYTES);
        header.limits(limits.clone());
        let (width, height) = header.into_dimensions().map_err(io::Error::other)?;
        if width > 8192 || height > 8192 || u64::from(width) * u64::from(height) > 16 * 1024 * 1024 {
            return Err(io::Error::other("图片像素超过 16 Mi 或单边 8192 上限"));
        }
        limits.max_image_width = Some(8192); limits.max_image_height = Some(8192);
        reader.limits(limits);
    }
    let mut decoder = reader.into_decoder().map_err(io::Error::other)?;
    if bounded && decoder.total_bytes() > WORK_BYTES {
        return Err(io::Error::other("图片解码超过 64 MiB 上限"));
    }
    let orientation = decoder.orientation().map_err(io::Error::other)?;
    #[cfg(test)] decode_metrics::record();
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
    img.apply_orientation(orientation);
    let dimensions = (img.width(), img.height());
    let mut out = Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(io::Error::other)?;
    Ok((out.into_inner(), dimensions))
}
pub fn dimensions(bytes: &[u8]) -> io::Result<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?
        .into_dimensions().map_err(io::Error::other)
}

/// Only fully decoded, oriented static planes can bypass validation during publication.
#[derive(Clone)]
pub(crate) struct PreparedImage {
    bytes: std::sync::Arc<Vec<u8>>,
    pub dimensions: (u32, u32),
    pub digest: String,
}

impl PreparedImage {
    pub fn normalize(bytes: &[u8]) -> io::Result<Self> {
        let (bytes, dimensions) = normalize_plane(bytes, true)?;
        let digest = digest(&bytes);
        Ok(Self { bytes: std::sync::Arc::new(bytes), dimensions, digest })
    }
    pub fn bytes(&self) -> &[u8] { &self.bytes }
    pub fn allocation_bytes(&self) -> usize { self.bytes.capacity() }
    pub fn restore(bytes: Vec<u8>, dimensions: (u32, u32), expected: &str) -> io::Result<Self> {
        if digest(&bytes) != expected { return Err(io::Error::other("图片暂存内容已变化")); }
        Ok(Self { bytes: std::sync::Arc::new(bytes), dimensions, digest: expected.into() })
    }
}

pub(crate) enum ImageInput { Unchecked(Vec<u8>), Prepared(PreparedImage) }
impl ImageInput {
    pub fn bytes(&self) -> &[u8] {
        match self { Self::Unchecked(bytes) => bytes, Self::Prepared(image) => image.bytes() }
    }
    pub fn validated(&self) -> bool { matches!(self, Self::Prepared(_)) }
    pub fn crop_is_noop(&self, crop: Option<crate::organize::Crop>) -> bool {
        match crop {
            None => true,
            Some(crate::organize::Crop::PosterRight) => matches!(self, Self::Prepared(image)
                if image.dimensions.0 as f64 / image.dimensions.1 as f64 <= 1.2),
        }
    }
}

#[cfg(test)]
pub(crate) mod decode_metrics {
    std::thread_local! { static COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
    pub fn record() { COUNT.with(|count| count.set(count.get() + 1)); }
    pub fn take() -> usize { COUNT.with(|count| count.replace(0)) }
}

#[cfg(test)]
mod preparation_tests {
    use super::*;

    #[test]
    fn downloaded_original_normalizes_once_and_rejects_before_decode() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let metadata = Metadata { code: "ABC-123".into(), ..Default::default() };
        std::fs::write(root.join("kiroku.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
        let capture = || revision::capture(&root, &root, &metadata.code).unwrap();
        let encode = |image: image::DynamicImage| {
            let mut out = Cursor::new(Vec::new());
            image.write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        decode_metrics::take();
        let original = Original::from_download_bytes(capture(), encode(image::DynamicImage::new_rgb8(40, 60)), None, Phase::default()).unwrap();
        assert_eq!(dimensions(&original.bytes).unwrap(), (40, 60));
        assert_eq!(decode_metrics::take(), 1, "a valid remote original must decode only once");
        for bytes in [
            encode(image::DynamicImage::ImageLuma8(image::GrayImage::new(4097, 4097))),
            encode(image::DynamicImage::ImageRgba16(image::ImageBuffer::new(3000, 3000))),
            vec![0; 32 * 1024 * 1024 + 1],
        ] {
            assert!(Original::from_download_bytes(capture(), bytes, None, Phase::default()).is_err());
            assert_eq!(decode_metrics::take(), 0, "oversized inputs must be rejected before decoding pixels");
        }
    }

    #[test]
    fn excessive_pixel_count_is_rejected_before_normalization() {
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(image::GrayImage::new(4097, 4097))
            .write_to(&mut encoded, image::ImageFormat::Png).unwrap();
        decode_metrics::take();
        let error = PreparedImage::normalize(encoded.get_ref()).err().expect("a compact response may still exceed the pixel budget");
        assert!(error.to_string().contains("图片像素"));
        assert_eq!(decode_metrics::take(), 0, "reject from headers before decoding pixels");
    }

    #[test]
    fn normalized_dimensions_only_need_headers() {
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(40, 60)
            .write_to(&mut encoded, image::ImageFormat::Png).unwrap();
        decode_metrics::take();
        let bytes = normalize(encoded.get_ref()).unwrap();
        assert_eq!(dimensions(&bytes).unwrap(), (40, 60));
        assert_eq!(decode_metrics::take(), 1, "dimensions must read headers without decoding pixels");
        // Keep the PNG header and IDAT length/type, but remove its pixels. Full
        // decoding must fail while dimensions remain available from the header.
        let idat = bytes.windows(4).position(|chunk| chunk == b"IDAT").unwrap();
        let header = &bytes[..idat + 4];
        assert!(image::load_from_memory(header).is_err());
        assert_eq!(dimensions(header).unwrap(), (40, 60));
    }
}
pub fn crop(bytes: &[u8], rect: CropBox, stamp: Option<&Stamping>) -> io::Result<Vec<u8>> {
    let img = image::load_from_memory(bytes).map_err(io::Error::other)?;
    if rect.width == 0
        || rect.height == 0
        || rect
            .x
            .checked_add(rect.width)
            .is_none_or(|v| v > img.width())
        || rect
            .y
            .checked_add(rect.height)
            .is_none_or(|v| v > img.height())
    {
        return Err(io::Error::other("裁剪框必须是原图范围内的正整数像素"));
    }
    let mut rgb = img
        .crop_imm(rect.x, rect.y, rect.width, rect.height)
        .to_rgb8();
    if let Some(stamp) = stamp {
        crate::badge::draw(&mut rgb, stamp);
    }
    let mut out = vec![];
    rgb.write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
        &mut out, 92,
    ))
    .map_err(io::Error::other)?;
    Ok(out)
}
#[derive(Clone)]
pub struct Original {
    pub captured: Preview,
    pub bytes: Vec<u8>,
    pub artwork: Artwork,
    poster_digest: Option<String>,
}
impl Original {
    pub fn from_bytes(
        captured: Preview,
        bytes: Vec<u8>,
        source_url: Option<String>,
        phase: Phase,
    ) -> io::Result<Self> {
        let bytes = normalize(&bytes)?;
        Self::from_normalized(captured, bytes, source_url, phase)
    }
    /// Prepare a newly downloaded original within the image input budget.
    /// Persisted and legacy local originals retain `from_bytes`/`load` behavior.
    pub fn from_download_bytes(
        captured: Preview,
        bytes: Vec<u8>,
        source_url: Option<String>,
        phase: Phase,
    ) -> io::Result<Self> {
        if bytes.len() > crate::net::IMAGE_RESPONSE_LIMIT {
            return Err(io::Error::other("图片响应超过 32 MiB 上限"));
        }
        let (bytes, _) = normalize_plane(&bytes, true)?;
        Self::from_normalized(captured, bytes, source_url, phase)
    }
    fn from_normalized(
        captured: Preview,
        bytes: Vec<u8>,
        source_url: Option<String>,
        phase: Phase,
    ) -> io::Result<Self> {
        let hash = digest(&bytes);
        let (poster_digest, _) =
            revision::snapshot_file(&captured.root, &crate::assets::Paths::for_movie(&captured.dir, &captured.before)?.poster)?;
        Ok(Self {
            captured,
            bytes,
            artwork: Artwork {
                original: original_name(&hash),
                digest: hash,
                source_url: source_url.map(|url| crate::metadata::public_url(&url)),
                crop: None,
                phase,
            },
            poster_digest,
        })
    }
    pub fn load(captured: Preview) -> io::Result<Option<Self>> {
        let Some(artwork) = captured.before.artwork.clone() else {
            return Ok(None);
        };
        if artwork.digest.len() != 64
            || !artwork.digest.bytes().all(|b| b.is_ascii_hexdigit())
            || artwork.original != original_name(&artwork.digest)
        {
            return Err(io::Error::other("无效原图引用"));
        }
        let (hash, bytes) =
            revision::snapshot_file(&captured.root, &captured.dir.join(&artwork.original))?;
        if hash.as_deref() != Some(&artwork.digest) {
            return Err(io::Error::other("原图缺失或已被外部修改"));
        }
        let bytes = bytes.ok_or_else(|| io::Error::other("原图缺失"))?;
        // Persisted originals are normalized static PNG, not browser-dependent original media.
        if normalize(&bytes)? != bytes {
            return Err(io::Error::other("原图不是规范静态图像"));
        }
        let (poster_digest, _) =
            revision::snapshot_file(&captured.root, &crate::assets::Paths::for_movie(&captured.dir, &captured.before)?.poster)?;
        Ok(Some(Self {
            captured,
            bytes,
            artwork,
            poster_digest,
        }))
    }
    pub fn preview(
        &self,
        rect: CropBox,
        stamp: Option<&Stamping>,
    ) -> io::Result<(Preview, Vec<u8>)> {
        let poster = crop(&self.bytes, rect, stamp)?;
        let mut after = self.captured.before.clone();
        let mut artwork = self.artwork.clone();
        artwork.crop = Some(rect);
        after.artwork = Some(artwork);
        after.locked_fields.insert("poster_urls".into());
        after
            .provenance
            .insert("poster_urls".into(), "manual".into());
        let preview = self
            .captured
            .prepare_after(after)?
            .with_original(self.bytes.clone())?
            .with_file(
                crate::assets::Paths::for_movie(&self.captured.dir, &self.captured.before)?.poster,
                self.poster_digest.clone(),
                poster.clone(),
            )?;
        Ok((preview, poster))
    }
}
/// Fresh eligible snapshots supply repair URLs without rewriting non-selected metadata fields.
pub fn repair_metadata(meta: &Metadata, settings: &crate::settings::Settings) -> Metadata {
    let mut out = meta.clone();
    let Some(code) = crate::parse_code(&meta.code) else {
        return out;
    };
    let chains = settings.chains_for(code.content_type);
    let sources = settings.build_sources();
    let parts: Vec<_> = meta
        .raw_sources
        .iter()
        .filter(|(site, snapshot)| {
            settings.sources.iter().any(|cfg| {
                cfg.enabled
                    && cfg.id == **site
                    && snapshot.matches_config(&code, cfg, settings.proxy_for(cfg))
            }) && sources
                .iter()
                .any(|source| source.id() == site.as_str() && source.supports(&code))
        })
        .map(|(id, snapshot)| {
            (
                sources.iter().find(|s| s.id() == id.as_str()).unwrap().id(),
                snapshot.fields.clone(),
            )
        })
        .collect();
    let fresh = crate::aggregate::aggregate(&meta.code, &parts, &chains);
    if !meta.locked_fields.contains("poster_urls") && !fresh.fields.poster_urls.is_empty() {
        out.fields.poster_urls = fresh.fields.poster_urls;
    }
    if !meta.locked_fields.contains("thumb_urls") && !fresh.fields.thumb_urls.is_empty() {
        out.fields.thumb_urls = fresh.fields.thumb_urls;
    }
    out
}
/// Legacy import journals retain the chosen URL and original filename phase, even after renaming.
pub fn legacy_source(root: &Path, dir: &Path) -> io::Result<(Option<String>, Phase)> {
    let mut phase = Phase::default();
    let mut url = None;
    for entry in crate::journal::Journal::inspect(root)? {
        if let Some(j) = entry.journal {
            for (i, op) in j.ops.iter().enumerate() {
                if j.rolled_back_ops.contains(&i) {
                    continue;
                }
                match op {
                    crate::organize::Op::MoveFile { from, to } if to.parent() == Some(dir) => {
                        if let Some(code) = from
                            .file_name()
                            .and_then(|s| s.to_str())
                            .and_then(crate::parse_code)
                        {
                            phase.merge(&Phase::of(&code));
                        }
                    }
                    crate::organize::Op::Download {
                        path, url: chosen, ..
                    } if path.parent() == Some(dir) && crate::assets::is_poster(path) && url.is_none() => {
                        url = Some(chosen.clone())
                    }
                    _ => {}
                }
            }
        }
    }
    Ok((url, phase))
}
/// Valid candidates only; first high-quality portrait wins relative to first valid thumb.
pub fn preferred(candidates: &[(bool, u32, u32)]) -> Option<usize> {
    let thumb = candidates.iter().position(|c| c.0);
    candidates
        .iter()
        .position(|&(is_thumb, w, h)| {
            !is_thumb
                && w < h
                && thumb.is_none_or(|i| u64::from(h) * 10 >= u64::from(candidates[i].2) * 9)
        })
        .or(thumb)
        .or((!candidates.is_empty()).then_some(0))
}
/// A legacy fanart is an original only when its logged source and transform prove equivalence.
pub fn legacy_local_original(root: &Path, dir: &Path, url: &str) -> io::Result<Option<Vec<u8>>> {
    for entry in crate::journal::Journal::inspect(root)? {
        if let Some(j) = entry.journal {
            for (i, op) in j.ops.iter().enumerate() {
                if j.rolled_back_ops.contains(&i) {
                    continue;
                }
                if let crate::organize::Op::Download {
                    path,
                    url: chosen,
                    crop: None,
                    stamp: None,
                } = op
                {
                    if path.parent() == Some(dir) && crate::assets::is_fanart(path) && chosen == url {
                        let (hash, bytes) = revision::snapshot_file(root, path)?;
                        if j.proofs
                            .get(&i)
                            .is_some_and(|proof| hash.as_ref() == Some(&proof.digest))
                        {
                            return Ok(bytes);
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}
