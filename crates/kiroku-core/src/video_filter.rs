//! 哪些视频不算正片:样片/预告片、广告(用户黑名单)、过小的文件。
//!
//! 对齐 amane `library/rules.py`:规则只匹配文件名(含扩展名),不看目录段;
//! 每条正则单独编译(用户写的 `(?i)` 之类全局旗标拼进 `|` 中间会失效);体积阈值 0 为关闭。
//! 与 amane 的差异:预告片默认规则更宽(amane 只有 `(?i)trailer`),把样片、预览也算进来。
//! 扫描时跳过命中的文件;整理后清理原文件夹时,它们也不算「还有正片」。

use regex::Regex;
use serde::{Deserialize, Serialize};

pub const DEFAULT_EXTRA_PATTERN: &str = r"(?i)sample|trailer|preview|预告|預告|样片|樣本|样本";

/// 用户设置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanCfg {
    /// 样片/预告片文件名正则;空串关闭
    pub extra_pattern: String,
    /// 广告等黑名单,每条一个正则
    pub blacklist: Vec<String>,
    /// 小于此值(MB)的视频不算正片;0 关闭
    pub min_video_mb: u64,
}

impl Default for ScanCfg {
    fn default() -> Self {
        Self { extra_pattern: DEFAULT_EXTRA_PATTERN.into(), blacklist: vec![], min_video_mb: 0 }
    }
}

impl ScanCfg {
    /// 逐条校验正则,报出第一条非法的。
    pub fn validate(&self) -> Result<(), String> {
        let check = |p: &str, what: &str| {
            if p.trim().is_empty() { return Ok(()) }
            Regex::new(p).map(|_| ()).map_err(|e| format!("{what}「{p}」不是有效的正则:{e}"))
        };
        check(&self.extra_pattern, "样片/预告片规则")?;
        self.blacklist.iter().try_for_each(|p| check(p, "黑名单规则"))
    }
}

/// 不算正片的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SkipReason {
    /// 样片 / 预告片
    Extra,
    /// 命中黑名单(广告等)
    Blacklisted,
    /// 小于最小体积
    TooSmall,
}

/// 编译好的过滤器。非法正则跳过该条(设置保存时已校验,这里只做兜底)。
#[derive(Debug, Clone)]
pub struct VideoFilter {
    extra: Option<Regex>,
    blacklist: Vec<Regex>,
    min_bytes: u64,
}

impl Default for VideoFilter {
    fn default() -> Self {
        Self::new(&ScanCfg::default())
    }
}

impl VideoFilter {
    pub fn new(cfg: &ScanCfg) -> Self {
        let compile = |p: &str| Some(p).filter(|p| !p.trim().is_empty()).and_then(|p| Regex::new(p).ok());
        Self {
            extra: compile(&cfg.extra_pattern),
            blacklist: cfg.blacklist.iter().filter_map(|p| compile(p)).collect(),
            min_bytes: cfg.min_video_mb.saturating_mul(1024 * 1024),
        }
    }

    /// 不过滤任何文件。
    pub fn none() -> Self {
        Self { extra: None, blacklist: vec![], min_bytes: 0 }
    }

    /// `name` 为文件名(含扩展名);`size` 取不到时传 None,此时不按体积过滤(读不到的正片不能当广告丢)。
    pub fn skip(&self, name: &str, size: Option<u64>) -> Option<SkipReason> {
        if self.blacklist.iter().any(|r| r.is_match(name)) {
            return Some(SkipReason::Blacklisted);
        }
        if self.extra.as_ref().is_some_and(|r| r.is_match(name)) {
            return Some(SkipReason::Extra);
        }
        if self.min_bytes > 0 && size.is_some_and(|s| s < self.min_bytes) {
            return Some(SkipReason::TooSmall);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_skips_samples_and_trailers_only() {
        let f = VideoFilter::default();
        for n in ["SMPL-001-trailer.mp4", "SMPL-001 sample.mp4", "smpl001_Preview.mkv", "预告.mp4", "SMPL-001 樣本.mp4"] {
            assert_eq!(f.skip(n, Some(1)), Some(SkipReason::Extra), "{n}");
        }
        for n in ["SMPL-001.mp4", "SMPL-001-C.mp4", "最新地址.mp4"] {
            assert_eq!(f.skip(n, Some(1)), None, "{n}");
        }
    }

    #[test]
    fn blacklist_size_and_disabling() {
        let cfg = ScanCfg { blacklist: vec!["最新地址".into(), "(?i)^ads?[-_. ]".into(), "[".into()], min_video_mb: 100, ..Default::default() };
        let f = VideoFilter::new(&cfg);
        assert_eq!(f.skip("最新地址.mp4", Some(u64::MAX)), Some(SkipReason::Blacklisted));
        assert_eq!(f.skip("AD_xxx.mp4", Some(u64::MAX)), Some(SkipReason::Blacklisted), "每条单独编译,(?i) 生效");
        assert_eq!(f.skip("SMPL-001.mp4", Some(50 * 1024 * 1024)), Some(SkipReason::TooSmall));
        assert_eq!(f.skip("SMPL-001.mp4", Some(200 * 1024 * 1024)), None);
        assert_eq!(f.skip("SMPL-001.mp4", None), None, "取不到大小不按体积过滤");
        assert!(cfg.validate().unwrap_err().contains("「[」"));
        let off = VideoFilter::new(&ScanCfg { extra_pattern: String::new(), ..Default::default() });
        assert_eq!(off.skip("trailer.mp4", Some(1)), None);
        assert_eq!(VideoFilter::none().skip("sample.mp4", Some(0)), None);
    }
}
