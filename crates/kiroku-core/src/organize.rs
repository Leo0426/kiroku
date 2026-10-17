//! 元数据 + 命名模板 → 磁盘。默认 dry-run；执行时写操作日志以支持回滚。
//! 跨盘移动：先复制、校验大小，再删除源文件。

use crate::{badge::{self, BadgeCfg, Phase, Stamping}, code::Code, metadata::Metadata, nfo, subtitle, template};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::{Path, PathBuf}};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    MoveFile { from: PathBuf, to: PathBuf },
    CopyFile { from: PathBuf, to: PathBuf, #[serde(default)] expected: Option<String> },
    HardlinkFile { from: PathBuf, to: PathBuf, #[serde(default)] expected: Option<String> },
    SymlinkFile { from: PathBuf, to: PathBuf, #[serde(default)] expected: Option<String> },
    WriteFile { path: PathBuf, content: String },
    /// 图片等二进制:由调用方提供下载器(核心不做网络 I/O)
    Download {
        url: String,
        path: PathBuf,
        #[serde(default)]
        crop: Option<Crop>,
        /// 封面角标(中字/破解…),在裁剪之后贴
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stamp: Option<Stamping>,
    },
}

impl Op {
    pub fn source(&self) -> Option<&Path> { match self { Self::MoveFile{from,..}|Self::CopyFile{from,..}|Self::HardlinkFile{from,..}|Self::SymlinkFile{from,..}=>Some(from),_=>None } }
    pub fn retained(&self)->bool { matches!(self, Self::CopyFile{..}|Self::HardlinkFile{..}|Self::SymlinkFile{..}) }
    pub fn target(&self) -> &Path {
        match self {
            Self::MoveFile { to, .. } | Self::CopyFile { to, .. } | Self::HardlinkFile { to, .. } | Self::SymlinkFile { to, .. } => to,
            Self::WriteFile { path, .. } | Self::Download { path, .. } => path,
        }
    }
}

impl Op {
    /// 一行可读描述,用于运行日志:`移动 a → b`、`写入 …/movie.nfo`、`保存 …/poster.jpg(裁剪海报,角标 subtitle cracked)`。
    pub fn describe(&self) -> String {
        match self {
            Self::MoveFile { from, to } => format!("移动 {} → {}", from.display(), to.display()),
            Self::CopyFile { from,to,.. } => format!("复制 {} → {}",from.display(),to.display()),
            Self::HardlinkFile { from,to,.. } => format!("硬链接 {} → {}",from.display(),to.display()),
            Self::SymlinkFile { from,to,.. } => format!("软链接 {} → {}",from.display(),to.display()),
            Self::WriteFile { path, .. } => format!("写入 {}", path.display()),
            Self::Download { path, crop, stamp, .. } => {
                let mut how = vec![];
                if crop.is_some() {
                    how.push("裁剪海报".to_string());
                }
                if let Some(s) = stamp {
                    how.push(format!("角标 {}", s.marks.iter().map(|m| m.stem.as_str()).collect::<Vec<_>>().join(" ")));
                }
                let how = if how.is_empty() { String::new() } else { format!("({})", how.join(",")) };
                format!("保存 {}{how}", path.display())
            }
        }
    }
}

pub fn occupied(path: &Path) -> bool { fs::symlink_metadata(path).is_ok() }

/// 整个计划先体检;执行时仍独占创建,以保护预览后才出现的文件。
pub fn preflight(ops: &[Op]) -> io::Result<()> {
    let mut targets = std::collections::HashSet::new();
    for op in ops {
        let target = op.target();
        if occupied(target) || !targets.insert(target) {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("目标已被占用: {}", target.display())));
        }
    }
    Ok(())
}

/// 下载后的图片处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Crop {
    /// 横版封面(封底|封面)裁出右侧封面作 poster
    PosterRight,
}

/// DMM 封面:封面宽 ≈ 高 × 0.7047(约 47.25% 总宽)。
const POSTER_RATIO: f64 = 0.7047;

/// 从横版封面裁出右半封面;已是竖版(宽高比 ≤ 1.2)则原样返回。
pub fn crop_poster(bytes: &[u8]) -> io::Result<Vec<u8>> {
    process_image(bytes.to_vec(), Some(Crop::PosterRight), None)
}

/// 下载后的图片处理:按需裁剪、画角标。两者都不需要(或竖版无需裁剪且无角标)时原样返回。
pub fn process_image(bytes: Vec<u8>, crop: Option<Crop>, stamp: Option<&Stamping>) -> io::Result<Vec<u8>> {
    match process_image_shared(&bytes, crop, stamp)? {
        std::borrow::Cow::Borrowed(_) => Ok(bytes),
        std::borrow::Cow::Owned(output) => Ok(output),
    }
}

pub(crate) fn process_image_shared<'a>(bytes: &'a [u8], crop: Option<Crop>, stamp: Option<&Stamping>) -> io::Result<std::borrow::Cow<'a, [u8]>> {
    use std::borrow::Cow;
    if crop.is_none() && stamp.is_none() {
        return Ok(Cow::Borrowed(bytes));
    }
    let bad = |e: image::ImageError| io::Error::new(io::ErrorKind::InvalidData, e);
    #[cfg(test)] crate::artwork::decode_metrics::record();
    let img = image::load_from_memory(bytes).map_err(bad)?;
    let (w, h) = (img.width(), img.height());
    let img = match crop {
        Some(Crop::PosterRight) if (w as f64) / (h as f64) > 1.2 => {
            let pw = (((h as f64) * POSTER_RATIO).round() as u32).min(w);
            img.crop_imm(w - pw, 0, pw, h)
        }
        _ if stamp.is_none() => return Ok(Cow::Borrowed(bytes)),
        _ => img,
    };
    let mut rgb = img.to_rgb8();
    if let Some(s) = stamp {
        badge::draw(&mut rgb, s);
    }
    let mut out = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92);
    rgb.write_with_encoder(enc).map_err(bad)?;
    Ok(Cow::Owned(out))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Plan {
    pub ops: Vec<Op>,
}

/// 替换非法文件名字符并裁剪长度。
pub fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { ' ' } else { c })
        .collect();
    let t = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = t.trim_matches(|c| c == '.' || c == ' ').to_string();
    // Windows 保留设备名(CON、NUL、COM1…)即使带扩展名也不能作文件/目录名
    let stem = t.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0');
    if reserved { format!("_{t}") } else { t }
}

/// 渲染目录模板(不带文件相位);语法见 [`crate::template`]。
pub fn render_template(tpl: &str, m: &Metadata) -> String {
    template::render_dir(tpl, &template::Ctx::new(m, None))
}

/// 整理计划的可选项。
#[derive(Debug, Clone, Default)]
pub struct PlanOpts {
    pub mode: crate::placement::OrganizeMode,
    pub nfo_language: crate::localization::NfoLanguage,
    /// Only explicit new-library callers may plan creation of a missing root.
    pub create_library: bool,
    /// 额外下载剧照到 `extrafanart/`(会产生较多请求,默认关)
    pub extrafanart: bool,
    /// Plex 友好命名:文件名带标题(Plex 用文件名作片名),分集用 Plex 能叠加的 ` - cd1`;开启时忽略 `file_template`
    pub plex_naming: bool,
    /// 视频文件名模板(不含扩展名);空串用 [`template::DEFAULT_FILE_TEMPLATE`]
    pub file_template: String,
    /// 封面角标(按文件名的中字/破解/流出等)
    pub badges: BadgeCfg,
    /// 用户自定义角标图目录(`<设置目录>/watermarks`);None 只用内置图
    pub stamp_dir: Option<PathBuf>,
}

/// 剧照最多下载这么多张。
pub const MAX_EXTRAFANART: usize = 12;

/// 生成整理计划(不触碰磁盘)。`dir_tpl` 决定目录;文件名按 `opts.file_template`(默认保留分集与中字/破解标记)。
/// 元数据文件(NFO/图片/kiroku.json)只随首片(无分片或 CD1)生成;需要改变归属时用 [`plan_with`]。
pub fn plan(src: &Path, code: &Code, meta: &Metadata, root: &Path, dir_tpl: &str) -> Plan {
    plan_with(src, code, meta, root, dir_tpl, code.part.map_or(true, |p| p == 1), PlanOpts::default())
}

/// `with_meta` 控制是否生成 NFO、图片与 `kiroku.json`(同一番号的多个分片里只应有一个为 true)。
pub fn plan_with(src: &Path, code: &Code, meta: &Metadata, root: &Path, dir_tpl: &str, with_meta: bool, opts: PlanOpts) -> Plan {
    let companions = subtitle::siblings(src, code);
    let mut phase = Phase::of(code);
    phase.subtitle |= opts.badges.external_subtitle && subtitle::any_chinese(&companions);
    plan_with_subtitles(src, code, meta, root, dir_tpl, with_meta, &opts, &phase, &companions)
}

/// Final concrete names, including every discovered subtitle suffix in the common stem budget.
pub(crate) fn preview_paths_with_subtitles(src: &Path, code: &Code, meta: &Metadata, dir_tpl: &str, opts: &PlanOpts, companions: &[subtitle::Sibling]) -> Result<(String,String,Vec<String>),String> {
    let ctx=template::Ctx::with_source(meta,Some(code),src);
    let ext=src.extension().and_then(|e|e.to_str()).unwrap_or("mp4");
    let suffix_bytes=companions.iter().map(|s|s.suffix.len()).max().unwrap_or(0).max(ext.len()+1).max("-fanart.jpeg".len());
    let file_tpl=if opts.plex_naming {template::PLEX_FILE_TEMPLATE} else if opts.file_template.trim().is_empty(){template::DEFAULT_FILE_TEMPLATE}else{&opts.file_template};
    template::preview_paths_with_suffix(dir_tpl,file_tpl,&ctx,suffix_bytes)
}

/// 同 [`plan_with`],角标按给定相位贴(多分片影片传入各分片聚合后的相位)。
#[allow(clippy::too_many_arguments)]
pub fn plan_with_phase(src: &Path, code: &Code, meta: &Metadata, root: &Path, dir_tpl: &str, with_meta: bool, opts: &PlanOpts, phase: &Phase) -> Plan {
    let companions = subtitle::siblings(src, code);
    plan_with_subtitles(src, code, meta, root, dir_tpl, with_meta, opts, phase, &companions)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_with_subtitles(src: &Path, code: &Code, meta: &Metadata, root: &Path, dir_tpl: &str, with_meta: bool, opts: &PlanOpts, phase: &Phase, companions: &[subtitle::Sibling]) -> Plan {
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    let Ok((directory,new_stem,_))=preview_paths_with_subtitles(src,code,meta,dir_tpl,opts,companions) else {return Plan{ops:vec![]};};
    let dir=root.join(directory);
    let target = dir.join(format!("{new_stem}.{ext}"));
    let mut ops = vec![opts.mode.operation(src.to_path_buf(), target)];
    for sub in companions {
        ops.push(opts.mode.operation(sub.from.clone(), dir.join(format!("{new_stem}{}", sub.suffix))));
    }
    if with_meta {
        let assets = crate::assets::Paths::for_stem(&dir, &new_stem).expect("validated video stem");
        ops.push(Op::WriteFile { path: assets.nfo, content: { let mut output=meta.clone();output.nfo_language=opts.nfo_language;nfo::to_nfo(&output) } });
        // 完整元数据,供之后无损重建媒体库索引
        let mut retained = meta.clone();
        retained.asset_stem = Some(new_stem);
        retained.nfo_language=opts.nfo_language;
        retained.artwork_phase = Some(phase.clone());
        if opts.mode!=crate::placement::OrganizeMode::Move {
            retained.placements.extend(ops.iter().filter_map(|op| {
                let (from,proof)=match op {Op::CopyFile{from,expected:Some(proof),..}|Op::HardlinkFile{from,expected:Some(proof),..}|Op::SymlinkFile{from,expected:Some(proof),..}=>(from,proof),_=>return None};
                crate::placement::capture_proven(opts.mode,from,op.target(),&dir,proof.clone()).ok()
            }));
        }
        if let Ok(json) = serde_json::to_string_pretty(&retained) {
            ops.push(Op::WriteFile { path: dir.join(SIDECAR), content: json });
        }
        let stamp = opts.badges.stamping(phase, opts.stamp_dir.as_deref());
        let poster_stamp = stamp.clone().filter(|_| opts.badges.poster);
        let fanart_stamp = stamp.filter(|_| opts.badges.fanart);
        // 横版封面:右侧裁出 poster,原图作 fanart;只有竖版封面时直接用作 poster
        if let Some(thumb) = meta.fields.thumb_urls.first() {
            ops.push(Op::Download { url: thumb.clone(), path: assets.poster, crop: Some(Crop::PosterRight), stamp: poster_stamp });
            ops.push(Op::Download { url: thumb.clone(), path: assets.fanart, crop: None, stamp: fanart_stamp });
        } else if let Some(poster) = meta.fields.poster_urls.first() {
            ops.push(Op::Download { url: poster.clone(), path: assets.poster, crop: None, stamp: poster_stamp });
        }
        if opts.extrafanart {
            for (i, url) in meta.fields.extrafanart.iter().take(MAX_EXTRAFANART).enumerate() {
                ops.push(Op::Download { url: url.clone(), path: dir.join("extrafanart").join(format!("fanart{}.jpg", i + 1)), crop: None, stamp: None });
            }
        }
    }
    Plan { ops }
}

pub use crate::subtitle::SUB_EXT;

/// 元数据旁路文件名(完整 Metadata 的 JSON)。
pub const SIDECAR: &str = "kiroku.json";

/// 执行结果:无论成功与否都带回已完成的操作,保证中途失败/取消仍有回滚依据。
#[derive(Debug, Default)]
pub struct Exec {
    pub done: Vec<Op>,
    /// 致命错误(移动/写文件失败),执行在此停止
    pub error: Option<io::Error>,
    /// 非致命问题:图片下载或裁剪失败不影响视频与 NFO 落盘,只记录
    pub warnings: Vec<String>,
}

/// 逐项执行;每项之前检查 `cancel`(为 true 则停止,不算错误)。
pub fn execute(plan: &Plan, fetch: &mut dyn FnMut(&str) -> io::Result<Vec<u8>>, cancel: &dyn Fn() -> bool) -> Exec {
    let mut out = Exec::default();
    if cancel() { return out; }
    if let Err(e) = preflight(&plan.ops) { out.error = Some(e); return out; }
    for op in &plan.ops {
        if cancel() {
            break;
        }
        match run_op(op, fetch) {
            Ok(()) => out.done.push(op.clone()),
            Err(e) if matches!(op, Op::Download { .. }) && e.kind() != io::ErrorKind::AlreadyExists => {
                if let Op::Download { url, .. } = op {
                    out.warnings.push(format!("图片 {url}: {e}"));
                }
            }
            Err(e) => {
                out.error = Some(e);
                break;
            }
        }
    }
    out
}

fn run_op(op: &Op, fetch: &mut dyn FnMut(&str) -> io::Result<Vec<u8>>) -> io::Result<()> {
    match op {
        Op::MoveFile { from, to } => {
            if let Some(p) = to.parent() { fs::create_dir_all(p)?; }
            move_file(from, to)
        }
        Op::CopyFile { from,to,.. } => { if let Some(p)=to.parent(){fs::create_dir_all(p)?;} copy_exclusive(from,to) }
        Op::HardlinkFile { from,to,.. } => { if let Some(p)=to.parent(){fs::create_dir_all(p)?;} fs::hard_link(from,to) }
        Op::SymlinkFile { from,to,.. } => { if let Some(p)=to.parent(){fs::create_dir_all(p)?;} crate::placement::symlink(&from.canonicalize()?,to) }
        Op::WriteFile { path, content } => {
            if let Some(p) = path.parent() { fs::create_dir_all(p)?; }
            write_new(path, content.as_bytes())
        }
        Op::Download { url, path, crop, stamp } => {
            if let Some(p) = path.parent() { fs::create_dir_all(p)?; }
            let bytes = process_image(fetch(url)?, *crop, stamp.as_ref())?;
            write_new(path, &bytes)
        }
    }
}

fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    restore_file(from, to)
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if result.is_err() { let _ = fs::remove_file(path); }
    result
}

/// 独占复制,不删除来源。用于目标盘暂存和跨盘恢复。
pub fn copy_exclusive(from: &Path, to: &Path) -> io::Result<()> {
    let mut source = fs::File::open(from)?;
    let metadata = source.metadata()?;
    let expected = metadata.len();
    let mut target = fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    let result = (|| {
        let copied=io::copy(&mut source, &mut target)?;
        #[cfg(test)] {crate::append::hash_reads::record(copied as usize);crate::append::hash_reads::record_write(copied as usize);}
        if copied != expected { return Err(io::Error::other("copy verification failed")); }
        target.set_permissions(metadata.permissions())?;
        target.sync_all()
    })();
    drop(target);
    if result.is_err() { let _ = fs::remove_file(to); }
    result
}

/// Copy and hash in one source pass. The caller still independently verifies the destination.
pub(crate) fn copy_proven(from: &Path, to: &Path, cancel: &dyn Fn()->bool) -> io::Result<String> {
    let mut target=fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    let owner=same_file::Handle::from_file(target.try_clone()?)?;
    let result=(|| {
        let proof=crate::append::fingerprint_stream(from,cancel,&|_,_|{},Some(&mut target))?;
        target.set_permissions(fs::metadata(from)?.permissions())?;
        // The durable executor performs its one sync after independent stage verification.
        Ok(proof)
    })();
    drop(target);
    // Never remove a replacement file when cleaning an interrupted stream.
    if result.is_err() && same_file::Handle::from_path(to).is_ok_and(|current|current==owner) {let _=fs::remove_file(to);}
    result
}

/// 撤销时原路径可能已被用户占用;原子创建目标,绝不替换已有文件或符号链接。
fn restore_file(from: &Path, to: &Path) -> io::Result<()> {
    let conflict = || io::Error::new(io::ErrorKind::AlreadyExists, format!("original path already exists: {}", to.display()));
    match fs::hard_link(from, to) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(conflict()),
        Err(_) => {
            copy_exclusive(from,to).map_err(|e| if e.kind() == io::ErrorKind::AlreadyExists { conflict() } else { e })?;
        }
    }
    if let Err(e) = fs::remove_file(from) {
        let _ = fs::remove_file(to);
        return Err(e);
    }
    Ok(())
}

/// 删除空目录;只含系统垃圾文件(.DS_Store / Thumbs.db / desktop.ini)的也算空。成功返回 true。
fn remove_empty_dir(dir: &Path) -> bool {
    const JUNK: [&str; 3] = [".DS_Store", "Thumbs.db", "desktop.ini"];
    let Ok(rd) = fs::read_dir(dir) else { return false };
    let names: Vec<_> = rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    if !names.iter().all(|n| JUNK.contains(&n.as_str())) {
        return false;
    }
    for n in &names {
        let _ = fs::remove_file(dir.join(n));
    }
    fs::remove_dir(dir).is_ok()
}

/// 按日志逆序回滚(移回文件,删除写出的文件),尽力而为:单项失败不中断,返回失败说明。
/// 回滚后顺手删除变空的目录,向上最多到 `root`(不含)。
pub fn rollback(done: &[Op], root: Option<&Path>) -> Vec<String> {
    let mut failures = vec![];
    let mut dirs: Vec<PathBuf> = vec![];
    for op in done.iter().rev() {
        match op {
            Op::MoveFile { from, to } => {
                let r = (|| {
                    if let Some(p) = from.parent() { fs::create_dir_all(p)?; }
                    restore_file(to, from)
                })();
                if let Err(e) = r {
                    failures.push(format!("{}: {e}", to.display()));
                }
                dirs.extend(to.parent().map(Path::to_path_buf));
            }
            Op::CopyFile { to,.. } | Op::HardlinkFile { to,.. } | Op::SymlinkFile { to,.. } => { failures.push(format!("无持久所有权证明，保留 {}",to.display())); }
            Op::WriteFile { path, .. } | Op::Download { path, .. } => {
                if path.exists() {
                    if let Err(e) = fs::remove_file(path) {
                        failures.push(format!("{}: {e}", path.display()));
                    }
                }
                dirs.extend(path.parent().map(Path::to_path_buf));
            }
        }
    }
    for d in dirs {
        let mut cur = Some(d.as_path());
        while let Some(dir) = cur {
            if root.is_some_and(|r| dir == r || !dir.starts_with(r)) || !remove_empty_dir(dir) {
                break;
            }
            cur = dir.parent();
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{code::parse_code, metadata::{Actor, Gender, PartialMetadata}};

    fn meta() -> Metadata {
        Metadata {
            code: "ABC-123".into(),
            fields: PartialMetadata {
                title: Some("T: x/y".into()),
                actors: vec![Actor::new("Foo", Gender::Female)],
                release_date: Some("2024-01-02".into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn execute_rejects_any_existing_output_before_moving_video() {
        for name in ["movie.nfo", "kiroku.json", "poster.jpg", "fanart.jpg", "ABC-123.srt", "extrafanart/fanart1.jpg"] {
            let d = tempfile::tempdir().unwrap();
            let src = d.path().join("ABC-123.mp4");
            let to = d.path().join("lib/ABC-123.mp4");
            let path = d.path().join("lib").join(name);
            fs::write(&src, b"video").unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"old content").unwrap();
            let output = if name.ends_with(".jpg") {
                Op::Download { path: path.clone(), url: "https://example.invalid/image".into(), crop: None, stamp: None }
            } else { Op::WriteFile { path: path.clone(), content: "replacement".into() } };
            let ex = execute(&Plan { ops: vec![Op::MoveFile { from: src.clone(), to: to.clone() }, output] }, &mut |_| Ok(b"image".to_vec()), &|| false);
            assert!(ex.error.is_some(), "must reject occupied {name}");
            assert!(ex.done.is_empty(), "preflight must stop before moving the video for {name}");
            assert_eq!(fs::read(src).unwrap(), b"video");
            assert!(!to.exists());
            assert_eq!(fs::read(path).unwrap(), b"old content");
        }
    }

    #[test]
    fn execute_preserves_image_created_during_download() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("poster.jpg");
        let p = Plan { ops: vec![Op::Download { url: "u".into(), path: path.clone(), crop: None, stamp: None }] };
        let ex = execute(&p, &mut |_| { fs::write(&path, b"new user file")?; Ok(b"image".to_vec()) }, &|| false);
        assert_eq!(ex.error.unwrap().kind(), io::ErrorKind::AlreadyExists);
        assert!(ex.done.is_empty());
        assert_eq!(fs::read(&path).unwrap(), b"new user file");
    }

    #[cfg(unix)]
    #[test]
    fn execute_rejects_dangling_symlink_output() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("movie.nfo");
        let missing = d.path().join("missing");
        std::os::unix::fs::symlink(&missing, &path).unwrap();
        let ex = execute(&Plan { ops: vec![Op::WriteFile { path: path.clone(), content: "x".into() }] }, &mut |_| unreachable!(), &|| false);
        assert!(ex.error.is_some() && ex.done.is_empty());
        assert_eq!(fs::read_link(path).unwrap(), missing);
        assert!(!missing.exists());
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_copy_preserves_permissions_and_never_replaces_existing_files() {
        use std::os::unix::fs::PermissionsExt;
        for mode in [0o600, 0o755] {
            let d = tempfile::tempdir().unwrap();
            let from = d.path().join("source"); let to = d.path().join("target");
            fs::write(&from,b"video").unwrap(); fs::set_permissions(&from,fs::Permissions::from_mode(mode)).unwrap();
            copy_exclusive(&from,&to).unwrap();
            assert_eq!(fs::metadata(&to).unwrap().permissions().mode() & 0o777,mode);
            assert_eq!(fs::read(&to).unwrap(),b"video");
            fs::write(&to,b"new user file").unwrap();
            assert!(copy_exclusive(&from,&to).is_err());
            assert_eq!(fs::read(&to).unwrap(),b"new user file");
            assert_eq!(fs::read(&from).unwrap(),b"video");
        }
    }

    #[test]
    fn template_sanitizes() {
        assert_eq!(render_template("{actor}/{code} {title}", &meta()), "Foo/ABC-123 T x y");
    }

    #[test]
    fn optional_groups_drop_when_a_value_is_missing() {
        let mut m = meta();
        // 没有系列:整个可选组(含分隔文字)消失
        assert_eq!(render_template("{actor}/{code}[ - {series}]", &m), "Foo/ABC-123");
        m.fields.series = Some("Ser/ies".into());
        assert_eq!(render_template("{actor}/{code}[ - {series}]", &m), "Foo/ABC-123 - Ser ies");
        // 组内多个占位符:任一缺失都省略
        assert_eq!(render_template("{code}[ ({studio} {year})]", &m), "ABC-123");
        m.fields.studio = Some("S1".into());
        assert_eq!(render_template("{code}[ ({studio} {year})]", &m), "ABC-123 (S1 2024)");
        // 组外缺失用兜底;括号不配对按普通文字;未知占位符原样保留
        m.fields.actors.clear();
        assert_eq!(render_template("{actor}/{code}", &m), "Unknown/ABC-123");
        assert_eq!(render_template("{code} [x", &m), "ABC-123 [x");
        assert_eq!(render_template("{code}{nope}", &m), "ABC-123{nope}");
    }

    #[test]
    fn plan_execute_rollback() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("in/abc123-cd2.mp4");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, b"video").unwrap();
        let code = parse_code("abc123-cd2.mp4").unwrap();
        let p = plan(&src, &code, &meta(), &tmp.path().join("lib"), "{actor}/{code}");
        assert_eq!(p.ops.len(), 1); // 非首片不写 nfo
        let ex = execute(&p, &mut |_| Err(io::Error::other("no network")), &|| false);
        assert!(ex.error.is_none());
        assert!(tmp.path().join("lib/Foo/ABC-123/ABC-123-CD2.mp4").exists());
        assert!(rollback(&ex.done, Some(&tmp.path().join("lib"))).is_empty());
        assert!(src.exists());
        // 变空的目录被清理,库根保留
        assert!(!tmp.path().join("lib/Foo").exists());
        assert!(tmp.path().join("lib").exists());
    }

    fn png(w: u32, h: u32, f: impl Fn(u32) -> [u8; 3]) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, _| image::Rgb(f(x)));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    #[test]
    fn crops_right_cover() {
        // 左半红(封底)、右半蓝(封面)
        let src = png(800, 438, |x| if x < 400 { [255, 0, 0] } else { [0, 0, 255] });
        let out = image::load_from_memory(&crop_poster(&src).unwrap()).unwrap().to_rgb8();
        assert_eq!((out.width(), out.height()), (309, 438));
        let px = out.get_pixel(150, 200);
        assert!(px[2] > 200 && px[0] < 60, "应为蓝色封面: {px:?}");
    }

    #[test]
    fn portrait_untouched() {
        let src = png(300, 420, |_| [0, 255, 0]);
        assert_eq!(crop_poster(&src).unwrap(), src);
    }

    #[test]
    fn execute_keeps_completed_ops_on_error_and_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a.mp4"), tmp.path().join("b.mp4"));
        fs::write(&a, b"a").unwrap();
        fs::write(&b, b"b").unwrap();
        let existing = tmp.path().join("out/x.mp4");
        fs::create_dir_all(existing.parent().unwrap()).unwrap();
        fs::remove_file(&b).unwrap();
        let p = Plan { ops: vec![
            Op::MoveFile { from: a.clone(), to: tmp.path().join("out/a.mp4") },
            Op::MoveFile { from: b.clone(), to: existing.clone() }, // 执行时源文件已消失 → 失败
            Op::WriteFile { path: tmp.path().join("out/never"), content: "x".into() },
        ]};
        let ex = execute(&p, &mut |_| unreachable!(), &|| false);
        assert_eq!(ex.done.len(), 1, "失败前完成的操作必须保留");
        assert_eq!(ex.error.unwrap().kind(), io::ErrorKind::NotFound);
        assert!(!tmp.path().join("out/never").exists());
        // 图片失败只记警告,不中断
        let p = Plan { ops: vec![Op::Download { url: "u".into(), path: tmp.path().join("p.jpg"), crop: None, stamp: None }, Op::WriteFile { path: tmp.path().join("ok"), content: "1".into() }] };
        let ex = execute(&p, &mut |_| Err(io::Error::other("offline")), &|| false);
        assert!(ex.error.is_none() && ex.warnings.len() == 1 && ex.done.len() == 1);
        // 取消:一项都不执行,也不算错误
        let ex = execute(&p, &mut |_| unreachable!(), &|| true);
        assert!(ex.done.is_empty() && ex.error.is_none());
    }

    #[test]
    fn rollback_cleans_dirs_that_only_hold_system_junk() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, to) = (tmp.path().join("in/a.mp4"), tmp.path().join("lib/Actor/A/a.mp4"));
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, b"v").unwrap();
        let ex = execute(&Plan { ops: vec![Op::MoveFile { from: src.clone(), to: to.clone() }] }, &mut |_| unreachable!(), &|| false);
        // Finder 在演员目录与影片目录里留下的垃圾文件
        fs::write(tmp.path().join("lib/Actor/.DS_Store"), b"x").unwrap();
        fs::write(tmp.path().join("lib/Actor/A/.DS_Store"), b"x").unwrap();
        assert!(rollback(&ex.done, Some(&tmp.path().join("lib"))).is_empty());
        assert!(!tmp.path().join("lib/Actor").exists(), "只剩垃圾文件的目录应被清理");
        // 有真实文件的目录不动
        let keep = tmp.path().join("lib/Keep");
        fs::create_dir_all(&keep).unwrap();
        fs::write(keep.join("movie.mp4"), b"v").unwrap();
        assert!(!remove_empty_dir(&keep) && keep.exists());
    }

    #[test]
    fn rollback_is_best_effort() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, to) = (tmp.path().join("in/a.mp4"), tmp.path().join("lib/A/a.mp4"));
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, b"v").unwrap();
        let ex = execute(&Plan { ops: vec![Op::MoveFile { from: src.clone(), to: to.clone() }] }, &mut |_| unreachable!(), &|| false);
        // 日志里多一项早已不存在的文件:不应中断其余回滚
        let mut done = ex.done;
        done.insert(0, Op::MoveFile { from: tmp.path().join("in/ghost.mp4"), to: tmp.path().join("lib/ghost.mp4") });
        let failures = rollback(&done, Some(&tmp.path().join("lib")));
        assert_eq!(failures.len(), 1);
        assert!(src.exists());
    }

    #[test]
    fn rollback_preserves_occupied_original_path_and_can_retry() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, to) = (tmp.path().join("in/a.mp4"), tmp.path().join("lib/A/a.mp4"));
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, b"original video").unwrap();
        let ex = execute(&Plan { ops: vec![Op::MoveFile { from: src.clone(), to: to.clone() }] }, &mut |_| unreachable!(), &|| false);
        fs::write(&src, b"new user file").unwrap();

        let failures = rollback(&ex.done, Some(&tmp.path().join("lib")));
        assert_eq!(failures.len(), 1, "原路径被占用时应报告冲突");
        assert_eq!(fs::read(&src).unwrap(), b"new user file", "不得覆盖用户的新文件");
        assert_eq!(fs::read(&to).unwrap(), b"original video", "保留库内视频以便重试");

        fs::remove_file(&src).unwrap();
        assert!(rollback(&ex.done, Some(&tmp.path().join("lib"))).is_empty());
        assert_eq!(fs::read(&src).unwrap(), b"original video");
        assert!(!to.exists());
    }

    #[cfg(unix)]
    #[test]
    fn rollback_preserves_dangling_symlink_at_original_path() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, to) = (tmp.path().join("in/a.mp4"), tmp.path().join("lib/A/a.mp4"));
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, b"original video").unwrap();
        let ex = execute(&Plan { ops: vec![Op::MoveFile { from: src.clone(), to: to.clone() }] }, &mut |_| unreachable!(), &|| false);
        let missing = tmp.path().join("missing.mp4");
        std::os::unix::fs::symlink(&missing, &src).unwrap();

        let failures = rollback(&ex.done, Some(&tmp.path().join("lib")));
        assert_eq!(failures.len(), 1, "悬空符号链接也占用了原路径");
        assert_eq!(fs::read_link(&src).unwrap(), missing);
        assert_eq!(fs::read(&to).unwrap(), b"original video");
        assert!(!missing.exists());

        fs::remove_file(&src).unwrap();
        assert!(rollback(&ex.done, Some(&tmp.path().join("lib"))).is_empty());
        assert_eq!(fs::read(&src).unwrap(), b"original video");
    }

    #[test]
    fn plex_naming_puts_title_in_filename_and_stacks_parts() {
        let code = parse_code("abc123-cd2.mp4").unwrap();
        let plex = PlanOpts { plex_naming: true, ..Default::default() };
        let p = plan_with(Path::new("/in/abc123-cd2.mp4"), &code, &meta(), Path::new("/lib"), "{code}", false, plex);
        let Op::MoveFile { to, .. } = &p.ops[0] else { panic!() };
        assert_eq!(to.file_name().unwrap(), "ABC-123 T x y - cd2.mp4");
        // 关闭时保持原命名
        let p = plan_with(Path::new("/in/abc123-cd2.mp4"), &code, &meta(), Path::new("/lib"), "{code}", false, PlanOpts::default());
        let Op::MoveFile { to, .. } = &p.ops[0] else { panic!() };
        assert_eq!(to.file_name().unwrap(), "ABC-123-CD2.mp4");
        // 没有标题时只用番号
        let mut m = meta();
        m.fields.title = None;
        let p = plan_with(Path::new("/in/abc123.mp4"), &parse_code("abc123.mp4").unwrap(), &m, Path::new("/lib"), "{code}", false, PlanOpts { plex_naming: true, ..Default::default() });
        let Op::MoveFile { to, .. } = &p.ops[0] else { panic!() };
        assert_eq!(to.file_name().unwrap(), "ABC-123.mp4");
    }

    #[test]
    fn file_names_keep_markers_and_follow_the_template() {
        let name = |src: &str, opts: PlanOpts| {
            let p = plan_with(Path::new(src), &parse_code(src).unwrap(), &meta(), Path::new("/lib"), "{code}", false, opts);
            let Op::MoveFile { to, .. } = &p.ops[0] else { panic!() };
            to.file_name().unwrap().to_string_lossy().into_owned()
        };
        assert_eq!(name("/in/ABC-123-UC.mkv", PlanOpts::default()), "ABC-123-UC.mkv");
        assert_eq!(name("/in/ABC-123-C-CD1.mp4", PlanOpts::default()), "ABC-123-CD1-C.mp4");
        assert_eq!(name("/in/ABC-123-UC.mp4", PlanOpts { plex_naming: true, ..Default::default() }), "ABC-123-UC T x y.mp4");
        let custom = PlanOpts { file_template: "{code}[[{def?}]]".into(), ..Default::default() };
        assert_eq!(name("/in/ABC-123-UC 4K.mp4", custom), "ABC-123[4K].mp4");
        // 同一部片的两个版本不再争同一个文件名
        assert_ne!(name("/in/ABC-123-C.mp4", PlanOpts::default()), name("/in/ABC-123-UC.mp4", PlanOpts::default()));
    }

    #[test]
    fn subtitles_follow_the_video() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("in/abc123-cd2.mp4");
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        for f in ["abc123-cd2.mp4", "abc123-cd2.chs.srt", "ABC123-CD2.ass", "abc1234-cd2.srt", "abc12.srt", "abc123-cd2.txt"] {
            fs::write(tmp.path().join("in").join(f), b"x").unwrap();
        }
        let code = parse_code("abc123-cd2.mp4").unwrap();
        let p = plan(&src, &code, &meta(), &tmp.path().join("lib"), "{code}");
        let moved: Vec<String> = p.ops.iter().filter_map(|o| if let Op::MoveFile { to, .. } = o { to.file_name().map(|n| n.to_string_lossy().into_owned()) } else { None }).collect();
        // 视频 + 两个同名字幕(大小写不敏感);相近但不同的番号(ABC-12、ABC-1234)和 txt 不动
        assert_eq!(moved, vec!["ABC-123-CD2.mp4", "ABC-123-CD2.ass", "ABC-123-CD2.chs.srt"]);
    }

    #[test]
    fn sanitize_windows_reserved_and_trailing() {
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("nul.txt"), "_nul.txt");
        assert_eq!(sanitize("COM1"), "_COM1");
        assert_eq!(sanitize("COM0"), "COM0");
        assert_eq!(sanitize("name. "), "name");
        assert_eq!(sanitize("Console"), "Console");
    }

    #[test]
    fn plan_writes_sidecar_with_meta_only() {
        let code = parse_code("abc123.mp4").unwrap();
        let p = plan(Path::new("/in/abc123.mp4"), &code, &meta(), Path::new("/lib"), "{code}");
        assert!(p.ops.iter().any(|o| matches!(o, Op::WriteFile { path, .. } if path.ends_with(SIDECAR))));
        let p = plan_with(Path::new("/in/abc123.mp4"), &code, &meta(), Path::new("/lib"), "{code}", false, PlanOpts::default());
        assert_eq!(p.ops.len(), 1);
        // 剧照:默认不下载;开启后最多 12 张,放进 extrafanart/ 且按序编号
        let mut m = meta();
        m.fields.extrafanart = (1..=20).map(|i| format!("http://x/{i}.jpg")).collect();
        let off = plan(Path::new("/in/abc123.mp4"), &code, &m, Path::new("/lib"), "{code}");
        assert!(!off.ops.iter().any(|o| matches!(o, Op::Download { path, .. } if path.to_string_lossy().contains("extrafanart"))));
        let on = plan_with(Path::new("/in/abc123.mp4"), &code, &m, Path::new("/lib"), "{code}", true, PlanOpts { extrafanart: true, ..Default::default() });
        let ef: Vec<_> = on.ops.iter().filter_map(|o| if let Op::Download { path, .. } = o { path.to_str().filter(|p| p.contains("extrafanart")) } else { None }).collect();
        assert_eq!(ef.len(), MAX_EXTRAFANART);
        assert!(ef[0].ends_with("extrafanart/fanart1.jpg") && ef[11].ends_with("fanart12.jpg"));
    }

    #[test]
    fn badges_go_on_poster_by_default_and_survive_the_journal() {
        use crate::badge::{BadgeCfg, Corner};
        let mut m = meta();
        m.fields.thumb_urls = vec!["http://x/pl.jpg".into()];
        let code = parse_code("/in/ABC-123-UC.mp4").unwrap();
        let imgs = |opts: PlanOpts| -> Vec<(String, Option<Stamping>)> {
            plan_with(Path::new("/in/ABC-123-UC.mp4"), &code, &m, Path::new("/lib"), "{code}", true, opts).ops.into_iter()
                .filter_map(|o| if let Op::Download { path, stamp, .. } = o { Some((path.file_name()?.to_string_lossy().into_owned(), stamp)) } else { None })
                .collect()
        };
        // 默认关闭
        assert!(imgs(PlanOpts::default()).iter().all(|(_, s)| s.is_none()));
        let on = PlanOpts { badges: BadgeCfg { enabled: true, ..Default::default() }, stamp_dir: Some("/cfg/watermarks".into()), ..Default::default() };
        let got = imgs(on);
        assert_eq!(got[0].0, "ABC-123-UC-poster.jpg");
        let s = got[0].1.as_ref().unwrap();
        assert_eq!(s.marks.iter().map(|m| (m.stem.as_str(), m.corner)).collect::<Vec<_>>(), [("subtitle", Corner::TopLeft), ("cracked", Corner::TopLeft)]);
        assert_eq!(s.dir.as_deref(), Some(Path::new("/cfg/watermarks")));
        assert!(got[1].0 == "ABC-123-UC-fanart.jpg" && got[1].1.is_none(), "fanart 默认不加");
        let both = PlanOpts { badges: BadgeCfg { enabled: true, fanart: true, ..Default::default() }, ..Default::default() };
        assert!(imgs(both)[1].1.is_some());
        // 贴图指令随操作日志持久化;没有该字段的旧日志仍能读回
        let op = Op::Download { url: "u".into(), path: "/p".into(), crop: None, stamp: Some(s.clone()) };
        assert_eq!(serde_json::from_str::<Op>(&serde_json::to_string(&op).unwrap()).unwrap(), op);
        let old: Op = serde_json::from_str(r#"{"Download":{"url":"u","path":"/p","crop":"PosterRight"}}"#).unwrap();
        assert!(matches!(old, Op::Download { stamp: None, .. }));
    }

    #[test]
    fn process_image_crops_then_stamps() {
        use crate::badge::{Corner, Mark};
        let src = png(800, 538, |_| [10, 10, 10]);
        let stamp = Stamping { marks: vec![Mark { stem: "cracked".into(), corner: Corner::TopLeft }], scale: 0.08, dir: None };
        let out = image::load_from_memory(&process_image(src.clone(), Some(Crop::PosterRight), Some(&stamp)).unwrap()).unwrap().to_rgb8();
        assert_eq!(out.dimensions(), (379, 538));
        let red = out.pixels().filter(|p| p.0[0] > 180 && p.0[1] < 90).count();
        assert!(red > 500, "左上角有红色「破解」角标");
        // 竖版且无角标:原样返回
        let portrait = png(300, 450, |_| [1, 2, 3]);
        assert_eq!(process_image(portrait.clone(), Some(Crop::PosterRight), None).unwrap(), portrait);
        assert_ne!(process_image(portrait.clone(), None, Some(&stamp)).unwrap(), portrait);
    }
}
