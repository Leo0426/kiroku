//! 字幕随迁:找出跟当前视频一起整理的字幕,以及整理后的文件名后缀。
//!
//! 对齐 amane `organize/subtitles.py`:只看视频所在的那一层目录;字幕只解析文件名(不看目录);
//! 解析出番号时必须与视频同号,再按分集配对;解析不出番号的跟无分集或 CD1 的视频。
//! kiroku 的补充:
//! - 仍保留「文件名以视频名开头」的旧规则(其后缀原样保留);
//! - 无番号字幕只在目录里**没有其他番号的视频**时才认领,否则合集目录里每部片都会来抢同一个字幕;
//! - 媒体服务器按「视频名开头」找字幕,所以整理后改名为 新视频名 + 语言标记(`.chs`)+ 扩展名,重名时加 `.2`。

use crate::{
    code::{parse_text, search_key, Code},
    pipeline::VIDEO_EXT,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

/// 字幕扩展名。
pub const SUB_EXT: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "idx", "sup"];

/// 一个随迁的字幕:原路径 + 接在新视频名之后的部分(如 `.chs.srt`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sibling {
    pub from: PathBuf,
    pub suffix: String,
}

fn ext_of(name: &str) -> Option<String> {
    Path::new(name).extension()?.to_str().map(str::to_lowercase)
}

fn same_number(a: &str, b: &str) -> bool {
    search_key(&a.to_uppercase()) == search_key(&b.to_uppercase())
}

/// 分集配对:同号同集;视频是 CD1 时也认没有分集标记的字幕。
fn cd_belongs(video: Option<u8>, sub: Option<u8>) -> bool {
    sub == video || (video == Some(1) && sub.is_none())
}

/// 文件名尾部的语言标记,如 `x.chs.srt` → `.chs`、`x.zh-CN.ass` → `.zh-CN`;只取末尾连续的「字母」段。
fn lang_tags(name: &str) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let mut segs: Vec<&str> = stem.split('.').skip(1).collect();
    let keep = segs.iter().rev().take_while(|s| !s.is_empty() && s.chars().count() <= 10 && s.chars().all(|c| c.is_alphabetic() || c == '-' || c == '_')).count();
    segs.drain(..segs.len() - keep);
    segs.iter().map(|s| format!(".{s}")).collect()
}

/// 中文字幕的语言标记(小写,`_` 视同 `-`)。
const CHINESE_TAGS: &[&str] = &["chs", "cht", "sc", "tc", "zh", "zh-cn", "zh-tw", "zh-hk", "zh-sg", "zh-hans", "zh-hant", "chi", "zho", "chinese", "cn", "big5", "gb"];
/// 文件名里表示中文字幕的词。
const CHINESE_WORDS: &[&str] = &["中文", "中字", "简体", "繁体", "简中", "繁中", "簡體", "繁體"];

/// 字幕文件名是否表明为中文字幕:末尾语言标记或文件名中的中文词。只看名字,不读内容。
pub fn is_chinese(name: &str) -> bool {
    if !ext_of(name).is_some_and(|e| SUB_EXT.contains(&e.as_str())) { return false }
    let tags = lang_tags(name).to_lowercase().replace('_', "-");
    tags.split('.').any(|t| CHINESE_TAGS.contains(&t)) || CHINESE_WORDS.iter().any(|w| name.contains(w))
}

/// 随迁字幕里是否有中文字幕。
pub fn any_chinese(companions: &[Sibling]) -> bool {
    companions.iter().any(|s| s.from.file_name().is_some_and(|n| is_chinese(&n.to_string_lossy())))
}

/// 纯判断:该字幕是否属于视频,属于则返回新文件名的后缀。`shared_dir` 为目录里还有其他番号的视频。
fn suffix_for(video_stem: &str, code: &Code, sub_name: &str, shared_dir: bool) -> Option<String> {
    let ext = ext_of(sub_name)?;
    // 旧规则:以视频名开头且紧跟分隔符(避免 ABC-12 误配 ABC-123.srt),后缀原样保留
    let (lc, stem_lc) = (sub_name.to_lowercase(), video_stem.to_lowercase());
    if lc.starts_with(&stem_lc) && lc[stem_lc.len()..].starts_with(['.', '-', '_', ' ']) {
        return sub_name.get(video_stem.len()..).map(String::from);
    }
    let info = parse_text(sub_name);
    let belongs = match info.number.as_deref() {
        Some(n) => same_number(n, &code.id),
        None => !shared_dir,
    } && cd_belongs(code.part, info.cd);
    belongs.then(|| format!("{}.{ext}", lang_tags(sub_name)))
}

/// 与视频同目录、属于该视频的字幕,按文件名排序;重名的后缀加序号。
pub fn siblings(video: &Path, code: &Code) -> Vec<Sibling> {
    let Some(dir) = video.parent() else {
        return vec![];
    };
    Directory::read(dir).siblings(video, code)
}

/// A plan-batch snapshot. Only subtitle candidates and numbered video names
/// are retained; all other directory entries are discarded during traversal.
pub(crate) struct Directory {
    names: Vec<String>,
    by_number: HashMap<String, Vec<usize>>,
    unnumbered: Vec<usize>,
    by_prefix: BTreeMap<String, Vec<usize>>,
    videos: HashMap<std::ffi::OsString, String>,
    video_counts: HashMap<String, usize>,
}

fn normalized_number(number: &str) -> String {
    search_key(&number.to_uppercase())
}

impl Directory {
    pub(crate) fn read(dir: &Path) -> Self {
        #[cfg(test)]
        directory_reads::record();
        let mut snapshot = Self {
            names: vec![],
            by_number: HashMap::new(),
            unnumbered: vec![],
            by_prefix: BTreeMap::new(),
            videos: HashMap::new(),
            video_counts: HashMap::new(),
        };
        for entry in fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_ok_and(|t| t.is_file()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(ext) = ext_of(&name) else { continue };
            if SUB_EXT.contains(&ext.as_str()) {
                snapshot.names.push(name);
            } else if VIDEO_EXT.contains(&ext.as_str()) {
                if let Some(number) = parse_text(&name).number {
                    let key = normalized_number(&number);
                    *snapshot.video_counts.entry(key.clone()).or_default() += 1;
                    snapshot.videos.insert(entry.file_name(), key);
                }
            }
        }
        snapshot.names.sort();
        for (index, name) in snapshot.names.iter().enumerate() {
            if let Some(number) = parse_text(name).number {
                snapshot
                    .by_number
                    .entry(normalized_number(&number))
                    .or_default()
                    .push(index);
            } else {
                snapshot.unnumbered.push(index);
            }
            snapshot
                .by_prefix
                .entry(name.to_lowercase())
                .or_default()
                .push(index);
        }
        snapshot
    }

    pub(crate) fn siblings(&self, video: &Path, code: &Code) -> Vec<Sibling> {
        let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str()))
        else {
            return vec![];
        };
        let key = normalized_number(&code.id);
        let own = video.file_name().and_then(|name| self.videos.get(name));
        // The source itself is excluded, including when its number was manually overridden.
        let others = self.videos.len()
            - self.video_counts.get(&key).copied().unwrap_or(0)
            - usize::from(own.is_some_and(|number| number != &key));
        let shared_dir = others > 0;
        let mut candidates = self.by_number.get(&key).cloned().unwrap_or_default();
        if !shared_dir {
            candidates.extend(&self.unnumbered);
        }
        let prefix = stem.to_lowercase();
        for (name, indexes) in self.by_prefix.range(prefix.clone()..) {
            if !name.starts_with(&prefix) {
                break;
            }
            if name[prefix.len()..].starts_with(['.', '-', '_', ' ']) {
                candidates.extend(indexes);
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        let mut used = HashSet::new();
        candidates
            .into_iter()
            .filter_map(|index| {
                let name = &self.names[index];
                let from = dir.join(name);
                if from == video {
                    return None;
                }
                let mut suffix = suffix_for(stem, code, name, shared_dir)?;
                if !used.insert(suffix.to_lowercase()) {
                    let (base, ext) = suffix
                        .rsplit_once('.')
                        .map(|(b, e)| (b.to_string(), e.to_string()))
                        .unwrap_or_default();
                    suffix = (2..)
                        .map(|i| format!("{base}.{i}.{ext}"))
                        .find(|s| used.insert(s.to_lowercase()))
                        .unwrap_or(suffix);
                }
                Some(Sibling { from, suffix })
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod directory_reads {
    thread_local! { static COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
    pub(super) fn record() {
        COUNT.with(|count| count.set(count.get() + 1));
    }
    pub(crate) fn measure<T>(f: impl FnOnce() -> T) -> (T, usize) {
        COUNT.with(|count| count.set(0));
        let value = f();
        (value, COUNT.with(std::cell::Cell::get))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_subtitles_are_recognised_by_language_tag_or_name() {
        for name in ["SMPL-001.chs.srt", "SMPL-001.CHT.ass", "SMPL-001.sc.srt", "SMPL-001.tc.srt", "SMPL-001.zh.srt", "SMPL-001.zh-CN.ass", "SMPL-001.zh-TW.srt", "SMPL-001.zh-Hans.vtt", "SMPL-001.zh_Hant.srt", "SMPL-001.chi.srt", "SMPL-001.chinese.srt", "中文字幕.ass", "SMPL-001 简体.srt", "SMPL-001-繁体.srt", "简中.srt"] {
            assert!(is_chinese(name), "{name}");
        }
        for name in ["SMPL-001.srt", "SMPL-001.en.srt", "SMPL-001.jp.srt", "SMPL-001.ja.ass", "SMPL-001.eng.srt", "SMPL-001.chs.mp4", "zhang.srt"] {
            assert!(!is_chinese(name), "{name}");
        }
    }
    use crate::code::parse_code;

    fn dir_with(files: &[&str]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for f in files {
            fs::write(d.path().join(f), b"x").unwrap();
        }
        d
    }

    fn found(d: &tempfile::TempDir, video: &str) -> Vec<(String, String)> {
        siblings(&d.path().join(video), &parse_code(video).unwrap())
            .into_iter()
            .map(|s| (s.from.file_name().unwrap().to_string_lossy().into_owned(), s.suffix))
            .collect()
    }

    fn s(a: &str, b: &str) -> (String, String) {
        (a.into(), b.into())
    }

    /// 复现:以前只有 `[HD]SMPL-001-C.srt` 会跟着视频走,其余三个留在原文件夹,随后被清理进废纸篓。
    #[test]
    fn matches_by_number_and_adopts_unnumbered_subtitles() {
        let d = dir_with(&["[HD]SMPL-001-C.mp4", "SMPL-001.chs.srt", "中文字幕.ass", "smpl001.srt", "[HD]SMPL-001-C.srt", "notes.txt"]);
        assert_eq!(
            found(&d, "[HD]SMPL-001-C.mp4"),
            [s("SMPL-001.chs.srt", ".chs.srt"), s("[HD]SMPL-001-C.srt", ".srt"), s("smpl001.srt", ".2.srt"), s("中文字幕.ass", ".ass")]
        );
    }

    #[test]
    fn other_numbers_and_parts_are_left_alone() {
        let d = dir_with(&["SMPL-001-CD1.mp4", "SMPL-001-CD2.mp4", "SMPL-001-CD2.srt", "SMPL-002.srt", "SMPL-001.srt"]);
        assert_eq!(found(&d, "SMPL-001-CD1.mp4"), [s("SMPL-001.srt", ".srt")], "无分集字幕归 CD1;别的番号、别的分集不要");
        assert_eq!(found(&d, "SMPL-001-CD2.mp4"), [s("SMPL-001-CD2.srt", ".srt")]);
    }

    #[test]
    fn unnumbered_subtitles_need_a_single_movie_folder() {
        // 合集目录:无番号字幕不知道属于谁,谁都不认领;有番号的照常
        let d = dir_with(&["SMPL-001.mp4", "SMPL-002.mp4", "中文字幕.srt", "SMPL-002.zh-CN.ass"]);
        assert!(found(&d, "SMPL-001.mp4").is_empty());
        assert_eq!(found(&d, "SMPL-002.mp4"), [s("SMPL-002.zh-CN.ass", ".zh-CN.ass")]);
        // 同号的样片、无番号的广告不算「其他番号」
        let d = dir_with(&["SMPL-001.mp4", "SMPL-001-trailer.mp4", "最新地址.mp4", "中文字幕.srt"]);
        assert_eq!(found(&d, "SMPL-001.mp4"), [s("中文字幕.srt", ".srt")]);
    }

    #[test]
    fn snapshot_excludes_overridden_source_number_and_keeps_prefix_matches() {
        let d = dir_with(&["SMPL-001.mp4", "SMPL-001.chs.srt", "中文字幕.ass"]);
        let video = d.path().join("SMPL-001.mp4");
        let override_code = parse_code("ABC-123.mp4").unwrap();
        let found: Vec<_> = siblings(&video, &override_code)
            .into_iter()
            .map(|s| {
                (
                    s.from.file_name().unwrap().to_string_lossy().into_owned(),
                    s.suffix,
                )
            })
            .collect();
        assert_eq!(found, [s("SMPL-001.chs.srt", ".chs.srt"), s("中文字幕.ass", ".ass")], "the manually overridden video must not count itself as another number; prefix subtitles still belong");
        fs::write(d.path().join("DEF-456.mp4"), b"v").unwrap();
        let found: Vec<_> = siblings(&video, &override_code)
            .into_iter()
            .map(|s| {
                (
                    s.from.file_name().unwrap().to_string_lossy().into_owned(),
                    s.suffix,
                )
            })
            .collect();
        assert_eq!(
            found,
            [s("SMPL-001.chs.srt", ".chs.srt")],
            "standalone discovery reads fresh disk and rejects ambiguous unnumbered subtitles"
        );
    }

    #[test]
    fn discovery_never_returns_its_source_as_a_companion() {
        let d = dir_with(&["SMPL-001.srt"]);
        assert!(siblings(
            &d.path().join("SMPL-001.srt"),
            &parse_code("SMPL-001").unwrap()
        )
        .is_empty());
    }

    #[test]
    fn language_tags() {
        assert_eq!(lang_tags("SMPL-001.chs.srt"), ".chs");
        assert_eq!(lang_tags("Vixen.23.04.15.eng.srt"), ".eng");
        assert_eq!(lang_tags("SMPL-001.简体.ass"), ".简体");
        assert_eq!(lang_tags("smpl001.srt"), "");
        assert_eq!(lang_tags("a.b.1080p.srt"), "", "带数字的段不是语言标记");
    }
}
