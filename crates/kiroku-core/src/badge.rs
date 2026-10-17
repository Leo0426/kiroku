//! 封面角标(水印),对齐 amane `media/images.py` + `media/watermarks/`:
//! 按文件相位在海报四角贴 PNG 角标——中字 / 无码 / 破解 / 流出 / 清晰度。
//!
//! - 角标图:内置 `assets/watermarks/*.png`(取自 amane,GPL-3.0,裁掉透明边并缩小);
//!   用户可在 `<设置目录>/watermarks/{stem}.png` 放同名文件覆盖,清晰度还可补 `1080p.png` 等内置没有的档。
//!   用户文件损坏则回退内置;都没有则跳过该枚,不回退文字。
//! - 尺寸:角标高 = 图高 × scale(海报与横版封面同高时一样大),最小 16px。
//! - 位置:每类各选一角;同角按「中字 → 无码 → 破解 → 流出 → 清晰度」向内叠(上角往下、下角往上),右角右对齐。
//!   `None` 表示该类不画(kiroku 补充,amane 只有四角)。

use crate::code::{Code, ContentType, Mosaic};
use image::{imageops, Rgba, RgbaImage, RgbImage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_SCALE: f32 = 0.08;
pub const SCALE_MIN: f32 = 0.03;
pub const SCALE_MAX: f32 = 0.25;
const STAMP_MIN_HEIGHT: u32 = 16;

/// 用户自定义角标目录名(在设置文件同目录下)。
pub const USER_DIRNAME: &str = "watermarks";

const BUILTIN: &[(&str, &[u8])] = &[
    ("subtitle", include_bytes!("../assets/watermarks/subtitle.png")),
    ("uncensored", include_bytes!("../assets/watermarks/uncensored.png")),
    ("cracked", include_bytes!("../assets/watermarks/cracked.png")),
    ("leaked", include_bytes!("../assets/watermarks/leaked.png")),
    ("4k", include_bytes!("../assets/watermarks/4k.png")),
    ("8k", include_bytes!("../assets/watermarks/8k.png")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// 角标类别;清晰度不论 4K / 1080p 共用一个位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Subtitle,
    Uncensored,
    Cracked,
    Leaked,
    Definition,
}

/// 每类角标的位置;缺省(含旧设置里缺的键)为左上,与 amane 一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Corners {
    pub subtitle: Option<Corner>,
    pub uncensored: Option<Corner>,
    pub cracked: Option<Corner>,
    pub leaked: Option<Corner>,
    pub definition: Option<Corner>,
}

impl Default for Corners {
    fn default() -> Self {
        let tl = Some(Corner::TopLeft);
        Self { subtitle: tl, uncensored: tl, cracked: tl, leaked: tl, definition: tl }
    }
}

impl Corners {
    pub fn get(&self, kind: Kind) -> Option<Corner> {
        match kind {
            Kind::Subtitle => self.subtitle,
            Kind::Uncensored => self.uncensored,
            Kind::Cracked => self.cracked,
            Kind::Leaked => self.leaked,
            Kind::Definition => self.definition,
        }
    }
}

/// 角标设置。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BadgeCfg {
    pub enabled: bool,
    /// 角标高度 = 图高 × scale,范围 [`SCALE_MIN`, `SCALE_MAX`]
    pub scale: f32,
    /// 贴在 poster.jpg(竖版海报)上
    pub poster: bool,
    /// 贴在 fanart.jpg(横版封面,Plex/Jellyfin 的背景)上
    pub fanart: bool,
    pub corners: Corners,
    /// 随迁的外挂字幕是中文时也算中字(文件名没有 -C 标记时)
    pub external_subtitle: bool,
}

impl Default for BadgeCfg {
    fn default() -> Self {
        Self { enabled: false, scale: DEFAULT_SCALE, poster: true, fanart: false, corners: Corners::default(), external_subtitle: true }
    }
}

/// 一部影片的相位汇总(多分片按「任一文件具备」聚合,清晰度取最高档),对齐 amane `FilePhaseSummary`。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Phase {
    pub subtitle: bool,
    /// 文件名无码标记或内容类型为无码(HEYZO 等不必带标记)
    pub uncensored: bool,
    pub cracked: bool,
    pub leaked: bool,
    pub definition: Option<String>,
}

/// 清晰度从高到低,与 `code::detect_definition` 的取值一致。
const DEFINITIONS: &[&str] = &["8K", "4K", "1440p", "1080p", "720p", "480p", "HD", "SD"];

fn definition_rank(d: &str) -> usize {
    DEFINITIONS.iter().position(|x| *x == d).unwrap_or(DEFINITIONS.len())
}

impl Phase {
    pub fn of(code: &Code) -> Self {
        Self {
            subtitle: code.subtitled,
            uncensored: code.mosaic == Some(Mosaic::Uncensored) || code.content_type == ContentType::Uncensored,
            cracked: code.mosaic == Some(Mosaic::Cracked),
            leaked: code.mosaic == Some(Mosaic::Leaked),
            definition: code.definition.clone(),
        }
    }

    pub fn merge(&mut self, other: &Phase) {
        self.subtitle |= other.subtitle;
        self.uncensored |= other.uncensored;
        self.cracked |= other.cracked;
        self.leaked |= other.leaked;
        if let Some(d) = &other.definition {
            if self.definition.as_deref().is_none_or(|cur| definition_rank(d) < definition_rank(cur)) {
                self.definition = Some(d.clone());
            }
        }
    }

    /// 要贴的角标主干,顺序:中字 / 无码 / 破解 / 流出 / 清晰度。破解与流出同时出现时只取破解。
    pub(crate) fn stems(&self) -> Vec<(Kind, String)> {
        let mut out = vec![];
        if self.subtitle {
            out.push((Kind::Subtitle, "subtitle".into()));
        }
        if self.uncensored {
            out.push((Kind::Uncensored, "uncensored".into()));
        }
        if self.cracked {
            out.push((Kind::Cracked, "cracked".into()));
        } else if self.leaked {
            out.push((Kind::Leaked, "leaked".into()));
        }
        if let Some(d) = &self.definition {
            out.push((Kind::Definition, d.to_lowercase()));
        }
        out
    }
}

/// 一枚角标:图片主干(`subtitle`、`4k`、`1080p`…)+ 位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub stem: String,
    pub corner: Corner,
}

/// 写进整理计划的贴图指令:自带尺寸与用户目录,日志恢复时能复现同样的结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stamping {
    pub marks: Vec<Mark>,
    pub scale: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
}

impl BadgeCfg {
    /// 该影片的贴图指令;关闭、无相位或全部位置为「不显示」时为 None。有码是默认状态,不打标。
    pub fn stamping(&self, phase: &Phase, dir: Option<&Path>) -> Option<Stamping> {
        if !self.enabled {
            return None;
        }
        let marks: Vec<Mark> = phase
            .stems()
            .into_iter()
            .filter_map(|(kind, stem)| Some(Mark { stem, corner: self.corners.get(kind)? }))
            .collect();
        (!marks.is_empty()).then(|| Stamping { marks, scale: self.scale, dir: dir.map(Path::to_path_buf) })
    }
}

/// `<设置目录>/watermarks`
pub fn user_dir(settings_path: &Path) -> PathBuf {
    settings_path.parent().unwrap_or(Path::new(".")).join(USER_DIRNAME)
}

/// 主干只允许小写字母数字,避免拼出 `../x` 之类的路径。
pub fn is_stem(stem: &str) -> bool {
    !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// 用户文件优先;损坏回退内置;都没有返回 None。
pub fn load_stamp(stem: &str, user_dir: Option<&Path>) -> Option<RgbaImage> {
    if !is_stem(stem) {
        return None;
    }
    if let Some(p) = user_dir.map(|d| d.join(format!("{stem}.png"))).filter(|p| p.is_file()) {
        if let Ok(img) = image::open(&p) {
            return Some(img.to_rgba8());
        }
    }
    let (_, bytes) = BUILTIN.iter().find(|(s, _)| *s == stem)?;
    image::load_from_memory(bytes).ok().map(|i| i.to_rgba8())
}

/// 裁掉透明边,再按高度等比缩放。无可见像素返回 None。
fn fit(stamp: &RgbaImage, target_h: u32) -> Option<RgbaImage> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in stamp.enumerate_pixels() {
        if p.0[3] > 0 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    let cropped = imageops::crop_imm(stamp, x0, y0, x1 - x0, y1 - y0).to_image();
    let w = ((cropped.width() as f32 * target_h as f32 / cropped.height() as f32).round() as u32).max(1);
    Some(imageops::resize(&cropped, w, target_h, imageops::FilterType::Lanczos3))
}

/// 把角标贴到图上。
pub fn draw(img: &mut RgbImage, s: &Stamping) {
    let (w, h) = img.dimensions();
    let target_h = ((h as f32 * s.scale.clamp(SCALE_MIN, SCALE_MAX)).round() as u32).max(STAMP_MIN_HEIGHT);
    let pad = (target_h / 8).max(4);
    let gap = (target_h / 10).max(4);
    let mut offset = [0u32; 4];
    for m in &s.marks {
        let Some(stamp) = load_stamp(&m.stem, s.dir.as_deref()).and_then(|r| fit(&r, target_h)) else { continue };
        let (sw, sh) = stamp.dimensions();
        let slot = m.corner as usize;
        let right = matches!(m.corner, Corner::TopRight | Corner::BottomRight);
        let bottom = matches!(m.corner, Corner::BottomLeft | Corner::BottomRight);
        let x = if right { w as i64 - pad as i64 - sw as i64 } else { pad as i64 };
        let y = if bottom { h as i64 - pad as i64 - offset[slot] as i64 - sh as i64 } else { (pad + offset[slot]) as i64 };
        offset[slot] += sh + gap;
        overlay(img, &stamp, x, y);
    }
}

/// 按 alpha 合成;超出画布的部分丢弃。
fn overlay(img: &mut RgbImage, stamp: &RgbaImage, x: i64, y: i64) {
    for (sx, sy, Rgba([r, g, b, a])) in stamp.enumerate_pixels() {
        let (px, py) = (x + sx as i64, y + sy as i64);
        if *a == 0 || px < 0 || py < 0 || px >= img.width() as i64 || py >= img.height() as i64 {
            continue;
        }
        let dst = img.get_pixel_mut(px as u32, py as u32);
        let a = *a as f32 / 255.0;
        for (d, s) in dst.0.iter_mut().zip([r, g, b]) {
            *d = (*d as f32 * (1.0 - a) + *s as f32 * a).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::parse_code;

    fn phase(name: &str) -> Phase {
        Phase::of(&parse_code(name).unwrap())
    }

    fn stems(cfg: &BadgeCfg, name: &str) -> Vec<(String, Corner)> {
        cfg.stamping(&phase(name), None).map(|s| s.marks.into_iter().map(|m| (m.stem, m.corner)).collect()).unwrap_or_default()
    }

    #[test]
    fn stems_follow_amane_order_and_rules() {
        let on = BadgeCfg { enabled: true, ..Default::default() };
        assert!(BadgeCfg::default().stamping(&phase("SMPL-001-UC.mp4"), None).is_none(), "默认关闭");
        let tl = Corner::TopLeft;
        assert_eq!(stems(&on, "SMPL-001-UC-4K.mp4"), [("subtitle".into(), tl), ("cracked".into(), tl), ("4k".into(), tl)]);
        assert!(stems(&on, "SMPL-001.mp4").is_empty(), "有码不打标");
        // 无码看内容类型,可与流出同时出现
        assert_eq!(stems(&on, "HEYZO-1234-leak.mp4"), [("uncensored".into(), tl), ("leaked".into(), tl)]);
        assert_eq!(stems(&on, "SMPL-001 1080p.mp4"), [("1080p".into(), tl)]);
        // 每类自选位置;None 不画
        let cfg = BadgeCfg { corners: Corners { subtitle: Some(Corner::BottomRight), cracked: None, ..Default::default() }, ..on };
        assert_eq!(stems(&cfg, "SMPL-001-UC.mp4"), [("subtitle".into(), Corner::BottomRight)]);
    }

    #[test]
    fn phases_merge_across_parts() {
        let mut p = phase("SMPL-001-CD1 720p.mp4");
        p.merge(&phase("SMPL-001-C-CD2 4K.mp4"));
        p.merge(&phase("SMPL-001-CD3 1080p.mp4"));
        assert!(p.subtitle && !p.cracked);
        assert_eq!(p.definition.as_deref(), Some("4K"));
    }

    #[test]
    fn old_settings_fill_missing_corners() {
        let c: BadgeCfg = serde_json::from_str(r#"{"enabled":true,"corners":{"subtitle":"top_right"}}"#).unwrap();
        assert_eq!((c.corners.subtitle, c.corners.definition, c.scale), (Some(Corner::TopRight), Some(Corner::TopLeft), DEFAULT_SCALE));
    }

    #[test]
    fn builtin_and_user_stamps() {
        for (s, _) in BUILTIN {
            assert!(load_stamp(s, None).is_some(), "{s}");
        }
        assert!(load_stamp("1080p", None).is_none());
        for bad in ["", "../subtitle", "subtitle.png", "SUBTITLE", "4K"] {
            assert!(!is_stem(bad));
        }
        let d = tempfile::tempdir().unwrap();
        RgbaImage::from_pixel(8, 8, Rgba([0, 255, 0, 255])).save(d.path().join("subtitle.png")).unwrap();
        RgbaImage::from_pixel(8, 8, Rgba([0, 0, 255, 255])).save(d.path().join("1080p.png")).unwrap();
        std::fs::write(d.path().join("cracked.png"), b"garbage").unwrap();
        assert_eq!(load_stamp("subtitle", Some(d.path())).unwrap().get_pixel(0, 0).0, [0, 255, 0, 255], "用户文件优先");
        assert!(load_stamp("1080p", Some(d.path())).is_some(), "用户可补内置没有的清晰度");
        assert!(load_stamp("cracked", Some(d.path())).unwrap().width() > 8, "损坏回退内置");
    }

    #[test]
    fn stacks_inward_in_the_chosen_corner() {
        let (w, h) = (379, 538);
        let mut img = RgbImage::new(w, h);
        let marks = ["subtitle", "cracked"].map(|s| Mark { stem: s.into(), corner: Corner::BottomRight }).to_vec();
        draw(&mut img, &Stamping { marks, scale: 0.08, dir: None });
        let th = (h as f32 * 0.08).round() as u32;
        let painted = |x0: u32, y0: u32, x1: u32, y1: u32| (y0..y1).any(|y| (x0..x1).any(|x| img.get_pixel(x, y).0 != [0, 0, 0]));
        assert!(painted(w / 2, h - th - 6, w, h - 6), "最靠角的一枚");
        assert!(painted(w / 2, h - 2 * th - 12, w, h - th - 10), "第二枚向上叠");
        assert!(!painted(0, 0, w, h - 2 * th - 20) && !painted(0, 0, w / 3, h), "其余区域不动");
        // 右对齐:贴近右边距
        assert!(painted(w - 8, h - th, w - 4, h - th / 2 - 4));
    }

    #[test]
    fn size_follows_image_height() {
        let mut img = RgbImage::new(800, 538);
        draw(&mut img, &Stamping { marks: vec![Mark { stem: "leaked".into(), corner: Corner::TopLeft }], scale: 0.1, dir: None });
        let rows = (0..538).filter(|&y| (0..800).any(|x| img.get_pixel(x, y).0 != [0, 0, 0])).count() as u32;
        // 角标边缘的半透明抗锯齿可能落不到整像素,允许几像素误差
        assert!(rows.abs_diff(54) <= 3, "高 = 538 × 0.1,实际 {rows}");
    }
}
