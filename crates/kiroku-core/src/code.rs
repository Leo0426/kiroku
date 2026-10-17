//! 番号与文件相位解析:纯函数,无 I/O。
//!
//! 忠实移植自 amane `parsing/file_info.py`(GPL-3.0),以 `tests/golden/amane_file_info.json`
//! 做差分测试。正则大量使用环视,故用 fancy-regex;模式本身与 amane 逐条对应,改动前先看 golden。

use fancy_regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Censored,
    Uncensored,
    Chinese,
    Western,
    Fc2,
    Amateur,
    Hentai,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mosaic {
    Censored,
    Uncensored,
    Cracked,
    Leaked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileInfo {
    pub number: Option<String>,
    pub content_type: ContentType,
    pub prefix: String,
    pub cd: Option<u8>,
    pub has_subtitle: bool,
    pub mosaic: Option<Mosaic>,
    pub definition: Option<&'static str>,
    /// 番号是否由已知规则命中(false = 路径模式下清理文件名得到的兜底值,不应拿去爬)
    pub matched: bool,
}

macro_rules! rx {
    ($p:expr) => {{
        static R: LazyLock<Regex> = LazyLock::new(|| Regex::new($p).expect("static regex"));
        &*R
    }};
}

// ---------------------------------------------------------------------------
// 常量表
// ---------------------------------------------------------------------------

const UNCENSORED_PREFIXES: &[&str] = &[
    "BT-", "CT-", "EMP-", "CCDV-", "CWP-", "CWPBD-", "DSAM-", "DRC-", "DRG-", "GACHI-", "HEYDOUGA", "JAV-",
    "LAF-", "LAFBD-", "HEYZO-", "KTG-", "KP-", "KG-", "LLDV-", "MCDV-", "MKD-", "MKBD-", "MMDV-", "NIP-",
    "PB-", "PT-", "QE-", "RED-", "RHJ-", "S2M-", "SKY-", "SKYHD-", "SMD-", "SSDV-", "SSKP-", "TRG-", "TS-",
    "XXX-AV-", "YKB-", "BIRD", "BOUGA", "H4610-", "C0930-", "H0930-", "S2M", "MCB3D",
];

/// 顺序有意义(与 amane 的 dict 迭代顺序一致)。
const SUREN_PREFIXES: &[(&str, &str)] = &[
    ("LUXU-", "259LUXU-"),
    ("ARA-", "200GANA-"),
    ("MIUM-", "300MIUM-"),
    ("NTK-", "300NTK-"),
    ("MAAN-", "300MAAN-"),
];

const WESTERN_NAMES: &[(&str, &str)] = &[
    ("vixen", "Vixen"),
    ("blacked", "Blacked"),
    ("tushy", "Tushy"),
    ("sexart", "SexArt"),
    ("x-art", "X-Art"),
    ("nubilefilms", "NubileFilms"),
    ("babes", "Babes"),
    ("wgp", "WhenGirlsPlay"),
    ("twistys", "Twistys"),
    ("deeper", "Deeper"),
    ("slayed", "Slayed"),
    ("tushy raw", "TushyRaw"),
    ("blackedraw", "BlackedRaw"),
];

/// 数字分辨率须逐个列出,否则其数字片段会被当作番号。
const ESCAPE_MARKERS: &[&str] = &[
    "4K", "4KS", "8K", "2160P", "1440P", "1080P", "720P", "480P", "HD", "LR", "VR", "DVD", "FULL", "HEVC",
    "H264", "H265", "X264", "X265", "AAC", "XXX", "PRT",
];

static MARKER_RES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    ESCAPE_MARKERS
        .iter()
        .map(|m| Regex::new(&format!(r"[-_ .\[]{m}[-_ .\]]")).expect("marker regex"))
        .collect()
});

const CATALOG_NUMBER: &str = r"[A-Z]{2,}-\d{2,}[Z]?";

// ---------------------------------------------------------------------------
// 小工具
// ---------------------------------------------------------------------------

fn find(re: &Regex, s: &str) -> Option<String> {
    re.find(s).ok().flatten().map(|m| m.as_str().to_string())
}

fn caps<'a>(re: &Regex, s: &'a str) -> Option<fancy_regex::Captures<'a>> {
    re.captures(s).ok().flatten()
}

fn is_match(re: &Regex, s: &str) -> bool {
    re.is_match(s).unwrap_or(false)
}

fn sub(re: &Regex, s: &str, rep: &str) -> String {
    re.replace_all(s, rep).into_owned()
}

fn trim_set<'a>(s: &'a str, set: &[char]) -> &'a str {
    s.trim_matches(|c| set.contains(&c))
}

fn capitalize(s: &str) -> String {
    let mut it = s.chars();
    match it.next() {
        Some(f) => f.to_uppercase().collect::<String>() + &it.as_str().to_lowercase(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// 预处理
// ---------------------------------------------------------------------------

struct Prepared {
    /// 只去掉分辨率/逃逸串。国产 MD 必须用此阶段(再剥尾部会删掉 MD0165-1 的 -1)
    escaped: String,
    /// 再剥 CD/字幕尾符,日期仍在。欧美 studio.YY.MM.DD 用此阶段
    dated: String,
    /// 再剥日期并归一 FC2。其余规则用此阶段
    catalog: String,
}

fn remove_escape_strings(filename: &str, escape: &[String]) -> String {
    let mut upper = filename.to_uppercase();
    for s in escape.iter().filter(|s| !s.is_empty()) {
        upper = upper.replace(&s.to_uppercase(), "");
    }
    for re in MARKER_RES.iter() {
        upper = sub(re, &upper, "-");
    }
    trim_set(&upper.replace("--", "-"), &['-', '_', ' ', '.']).to_string()
}

fn prepare(basename: &str, escape: &[String]) -> Prepared {
    let real_name = format!("{}.", basename.trim());
    let escaped = format!("{}.", remove_escape_strings(&real_name, escape));
    let mut dated = escaped
        .replace("-C.", ".")
        .replace(".PART", "-CD")
        .replace("-PART", "-CD")
        .replace(" EP.", ".EP")
        .replace("-CD-", "");
    dated = sub(rx!(r"[-_ .]CD\d{1,2}"), &dated, "");
    dated = sub(rx!(r"[-_ .][A-Z0-9]\.$"), &dated, "");
    let dated = trim_set(&dated.replace(' ', "-"), &['-', '_', '.', ' ']).to_string();
    let mut catalog = sub(rx!(r"\d{4}[-_.]\d{1,2}[-_.]\d{1,2}"), &dated, "");
    catalog = sub(rx!(r"[-\[]\d{2}[-_.]\d{2}[-_.]\d{2}\]?"), &catalog, "");
    let catalog = catalog
        .replace("FC2-PPV", "FC2-")
        .replace("FC2PPV", "FC2-")
        .replace("--", "-")
        .replace("GACHIPPV", "GACHI");
    Prepared { escaped, dated, catalog }
}

// ---------------------------------------------------------------------------
// 番号 + 类型
// ---------------------------------------------------------------------------

fn type_of_catalog_id(number: &str) -> ContentType {
    use ContentType::*;
    if is_match(rx!(r"^\d{6}[-_]\d{2,4}$"), number) {
        return Uncensored;
    }
    if is_match(rx!(r"^n\d{4}"), number) || is_match(rx!(r"[^.]+\.\d{2}\.\d{2}\.\d{2}"), number) {
        return Uncensored;
    }
    let upper = number.to_uppercase();
    if UNCENSORED_PREFIXES.iter().any(|p| upper.starts_with(&p.to_uppercase())) {
        return Uncensored;
    }
    if upper.contains("SIRO") || is_match(rx!(r"\d{3,}[A-Z]+-\d{2}"), &upper) {
        return Amateur;
    }
    if SUREN_PREFIXES.iter().any(|(k, _)| upper.starts_with(k)) {
        return Amateur;
    }
    if is_match(rx!(r"([^A-Z]|^)MD[A-Z-]*\d{4,}"), &upper) && !upper.contains("MDVR") {
        return Chinese;
    }
    if is_match(rx!(r"MKY-[A-Z]+-\d{3,}"), &upper) {
        return Chinese;
    }
    if is_match(rx!(CATALOG_NUMBER), &upper) {
        return Censored;
    }
    Western
}

/// 命中已知形态则 (番号, 类型),否则 None。不含文件名回退。
fn match_number(basename: &str, escape: &[String], generic: bool) -> Option<(String, ContentType)> {
    use ContentType::*;
    let t = prepare(basename, escape);
    let (c, escaped, dated) = (t.catalog.as_str(), t.escaped.as_str(), t.dated.as_str());

    if c.contains("MYWIFE") && is_match(rx!(r"NO\.\d*"), c) {
        let n = caps(rx!(r"NO\.(\d*)"), c).map(|m| m[1].to_string()).unwrap_or_default();
        return Some((format!("Mywife No.{n}"), Censored));
    }
    if let Some(m) = find(rx!(r"CW3D2D?BD-?\d{2,}"), c) {
        let ty = type_of_catalog_id(&m);
        return Some((m, ty));
    }
    if let Some(m) = find(rx!(r"MMR-?[A-Z]{2,}-?\d+[A-Z]*"), c) {
        let n = m.replace("MMR-", "MMR");
        let ty = type_of_catalog_id(&n);
        return Some((n, ty));
    }
    // 国产 MD 必须用 escaped
    if !escaped.contains("MDVR") {
        if let Some(m) = caps(rx!(r"([^A-Z]|^)(MD[A-Z-]*\d{4,}(-\d)?)"), escaped) {
            return Some((m[2].to_string(), Chinese));
        }
    }
    // 欧美日期号用 dated(日期尚未剥除)
    if is_match(rx!(r"([A-Z0-9_]{2,})[-.]2?0?(\d{2}[-.]\d{2}[-.]\d{2})"), dated) {
        if let Some(m) = caps(rx!(r"([A-Z0-9-]{2,})[-_.]2?0?(\d{2}[-.]\d{2}[-.]\d{2})"), dated) {
            let short = m[1].trim_matches('-').to_lowercase();
            let full = WESTERN_NAMES.iter().find(|(k, _)| *k == short).map(|(_, v)| v.to_string()).unwrap_or(short);
            let number = capitalize(&format!(
                "{}.{}",
                full.to_lowercase().replace(['-', '.'], ""),
                m[2].replace('-', ".")
            ));
            return Some((number, Western));
        }
    }
    // 其余规则用 catalog
    if let Some(m) = find(rx!(r"XXX-AV-\d{4,}"), c) {
        return Some((m, Uncensored));
    }
    if let Some(m) = find(rx!(r"MKY-[A-Z]+-\d{3,}"), c) {
        return Some((m, Chinese));
    }
    if c.contains("FC2") {
        let fc2 = c.replace("PPV", "").replace('_', "-").replace("--", "-");
        if let Some(m) = find(rx!(r"FC2-\d{5,}"), &fc2) {
            return Some((m, Fc2));
        }
        if let Some(m) = find(rx!(r"FC2\d{5,}"), &fc2) {
            return Some((m.replace("FC2", "FC2-"), Fc2));
        }
    }
    if c.contains("HEYZO") {
        let hz = c.replace('_', "-").replace("--", "-");
        if let Some(m) = find(rx!(r"HEYZO-\d{3,}"), &hz) {
            return Some((m, Uncensored));
        }
        if let Some(m) = find(rx!(r"HEYZO\d{3,}"), &hz) {
            return Some((m.replace("HEYZO", "HEYZO-"), Uncensored));
        }
    }
    if let Some(m) = find(rx!(r"(H4610|C0930|H0930)-[A-Z]+\d{4,}"), c) {
        return Some((m, Uncensored));
    }
    if let Some(m) = find(rx!(r"KIN8(TENGOKU)?-?\d{3,}"), c) {
        return Some((m.replace("TENGOKU", "-").replace("--", "-"), Uncensored));
    }
    if let Some(m) = find(rx!(r"S2M[BD]*-\d{3,}"), c).or_else(|| find(rx!(r"MCB3D[BD]*-\d{2,}"), c)) {
        return Some((m, Uncensored));
    }
    if let Some(m) = find(rx!(r"T\d{2}-?\d{3,}"), c) {
        // T28/T38 等:DMM 连写 Txx00nnn → Txx-nnn;已有短横线则原样
        return Some((sub(rx!(r"T(\d{2})00"), &m, "T${1}-"), Censored));
    }
    if let Some(m) = find(rx!(r"TH101-\d{3,}-\d{5,}"), c) {
        return Some((m.to_lowercase(), Uncensored));
    }
    if let Some(m) = caps(rx!(r"([A-Z]{2,})00(\d{3})"), c) {
        let n = format!("{}-{}", &m[1], &m[2]);
        let ty = type_of_catalog_id(&n);
        return Some((n, ty));
    }
    if let Some(m) = find(rx!(r"\d{2,}[A-Z]{2,}-\d{2,}[A-Z]?"), c) {
        return Some((m, Amateur));
    }
    if let Some(mut n) = find(rx!(CATALOG_NUMBER), c) {
        for (key, value) in SUREN_PREFIXES {
            if n.contains(key) {
                n = format!("{}{}", value.replace(key, ""), n);
                break;
            }
        }
        let ty = type_of_catalog_id(&n);
        return Some((n, ty));
    }
    // 六位数字 + 短横线/下划线 + 序号(010115-001 / 010115_001 是两部)
    if let Some(m) = find(rx!(r"(?<!\d)\d{6}[-_]\d{2,4}(?!\d)"), c) {
        return Some((m, Uncensored));
    }
    // 过宽回退仅对文件名
    if generic {
        let wide = find(rx!(r"[A-Z]+-[A-Z]\d+"), c)
            .or_else(|| find(rx!(r"\d{2,}[-_]\d{2,}"), c))
            .or_else(|| find(rx!(r"\d{3,}-[A-Z]{3,}"), c));
        if let Some(m) = wide {
            let ty = type_of_catalog_id(&m);
            return Some((m, ty));
        }
    }
    if let Some(m) = caps(rx!(r"([^A-Z]|^)(N\d{4})(\D|$)"), c) {
        return Some((m[2].to_lowercase(), Uncensored));
    }
    if let Some(m) = caps(rx!(r"H_\d{3,}([A-Z]{2,})(\d{2,})"), c) {
        let n = format!("{}-{}", &m[1], &m[2]);
        let ty = type_of_catalog_id(&n);
        return Some((n, ty));
    }
    if generic {
        if let Some(m) = caps(rx!(r"([A-Z]{3,}).*?(\d{2,})"), c) {
            let n = format!("{}-{}", &m[1], &m[2]);
            let ty = type_of_catalog_id(&n);
            return Some((n, ty));
        }
        if let Some(m) = caps(rx!(r"([A-Z]{2,}).*?(\d{3,})"), c) {
            let n = format!("{}-{}", &m[1], &m[2]);
            let ty = type_of_catalog_id(&n);
            return Some((n, ty));
        }
    }
    None
}

/// 路径仍未命中:FC2/HEYZO 关键字视为该族,否则清理后的原文 + 欧美。
fn fallback(stem: &str, escape: &[String]) -> (String, ContentType) {
    let t = prepare(stem, escape);
    if t.catalog.contains("FC2") {
        return (t.catalog.replace("PPV", "").replace('_', "-").replace("--", "-"), ContentType::Fc2);
    }
    if t.catalog.contains("HEYZO") {
        return (t.catalog.replace('_', "-").replace("--", "-"), ContentType::Uncensored);
    }
    let temp = sub(rx!(r"[【(（\[].+?[\]）)】]"), &t.escaped, "");
    let temp: String = trim_set(&temp, &['@', '.', ' ']).nfc().collect();
    let result = trim_set(&temp, &['-', '_', '.', ' ']).to_string();
    if let Some(rest) = result.strip_prefix("FC-") {
        return (format!("FC2-{rest}"), ContentType::Fc2);
    }
    (result, ContentType::Western)
}

/// 文件名 → 父目录(目录不用过宽拼接)→ 可选的文件名回退。返回 (番号, 类型, 是否规则命中)。
fn identify(stem: &str, dirs: &[&str], escape: &[String], use_fallback: bool) -> (Option<String>, ContentType, bool) {
    if let Some((n, t)) = match_number(stem, escape, true) {
        return (Some(n), t, true);
    }
    for name in dirs.iter().rev() {
        if let Some((n, t)) = match_number(name, escape, false) {
            return (Some(n), t, true);
        }
    }
    if !use_fallback {
        return (None, ContentType::Western, false);
    }
    let (n, t) = fallback(stem, escape);
    (Some(n), t, false)
}

fn prefix_of(number: &str) -> String {
    let upper = number.to_uppercase();
    if let Some(m) = caps(rx!(r"([A-Za-z0-9\-.]{3,})[-_. ]\d{2}\.\d{2}\.\d{2}"), number) {
        return m[1].to_uppercase();
    }
    if let Some(m) = caps(rx!(r"^(T\d{2})"), &upper) {
        return m[1].to_string();
    }
    for p in ["FC2", "MYWIFE", "KIN8", "S2M", "TH101", "XXX-AV"] {
        if upper.starts_with(p) {
            return p.to_string();
        }
    }
    if let Some(m) = caps(rx!(r"(MKY-[A-Z]+)-\d{3,}"), &upper) {
        return m[1].to_string();
    }
    if let Some(m) = caps(rx!(r"(H4610|C0930|H0930)"), &upper) {
        return m[1].to_string();
    }
    if let Some(m) = caps(rx!(r"(\d*[A-Za-z]+)\d*"), number) {
        return m[1].to_uppercase();
    }
    upper
}

// ---------------------------------------------------------------------------
// 路径关键词 + 文件相位
// ---------------------------------------------------------------------------

fn classify_from_path(stem: &str, dirs: &[&str]) -> Option<ContentType> {
    for name in std::iter::once(&stem).chain(dirs.iter()) {
        if name.to_lowercase() == "getchu" || name.starts_with("里番") || name.starts_with("裏番") {
            return Some(ContentType::Hentai);
        }
        if name.starts_with("欧美") {
            return Some(ContentType::Western);
        }
    }
    None
}

fn detect_cd(b: &str) -> Option<u8> {
    // 空格分隔与 DISC/DISK/PT 写法是 kiroku 在 amane 之外补充的
    for re in [rx!(r"[-_. ]CD(\d{1,2})"), rx!(r"[-_.]PART(\d{1,2})"), rx!(r"[-_. ](?:DIS[CK]|PT)(\d{1,2})(?![0-9])")] {
        if let Some(m) = caps(re, b) {
            return m[1].parse().ok();
        }
    }
    if let Some(m) = caps(rx!(r"-([AB])(?:-(?:UC|C|U))?(?:\.|$)"), b) {
        return Some(m[1].as_bytes()[0] - b'A' + 1);
    }
    // amane 只认结尾 `-N`;kiroku 补 `_N` 和分段号后跟字幕/破解尾符(`-1-C`、`_2-UC`)。
    caps(rx!(r"[-_]([1-9])(?:-(?:UC|C|U))?$"), b).and_then(|m| m[1].parse().ok())
}

fn detect_cd_from_parent(parent: &str) -> Option<u8> {
    let m = caps(rx!(r"(?i)^(?:CD|PART)(\d{1,2})$"), parent.trim())?;
    m[1].parse().ok().filter(|n| *n >= 1)
}

/// `-CH`、`-LEAKC`(流出 + 中字,kiroku 默认文件名模板的产物)是 kiroku 在 amane 之外补充的写法。
fn detect_subtitle(b: &str) -> bool {
    is_match(rx!(r"-U?C(?![A-Z0-9])"), b) || is_match(rx!(r"-(CH|LEAKC)(?![A-Z0-9])"), b) || b.chars().any(|c| "字幕中文".contains(c))
}

fn fill_mosaic(m: Option<Mosaic>, ty: ContentType) -> Option<Mosaic> {
    m.or(match ty {
        ContentType::Censored => Some(Mosaic::Censored),
        ContentType::Uncensored => Some(Mosaic::Uncensored),
        _ => None,
    })
}

/// 破解优先于流出,流出优先于无码。`-UMR`、`-LEAK` 是 kiroku 在 amane 之外补充的常见后缀。
fn detect_mosaic(b: &str) -> Option<Mosaic> {
    if is_match(rx!(r"破解|克破|CRACKED"), b) || is_match(rx!(r"-U(C|MR)?(?![A-Z0-9])"), b) {
        return Some(Mosaic::Cracked);
    }
    if is_match(rx!(r"流出|LEAKED"), b) || is_match(rx!(r"[-_ .]LEAKC?(?![A-Z0-9])"), b) {
        return Some(Mosaic::Leaked);
    }
    if is_match(rx!(r"無碼|无码|UNCENSORED"), b) {
        return Some(Mosaic::Uncensored);
    }
    None
}

fn detect_mosaic_from_dirs(dirs: &[&str]) -> Option<Mosaic> {
    for name in dirs.iter().rev() {
        let token = trim_set(name.trim(), &['[', ']', '【', '】', '(', ')', '（', '）']).to_lowercase();
        match token.as_str() {
            "uncensored" | "無碼" | "无码" => return Some(Mosaic::Uncensored),
            "cracked" | "破解" => return Some(Mosaic::Cracked),
            "leaked" | "流出" => return Some(Mosaic::Leaked),
            _ => {}
        }
    }
    None
}

fn detect_definition(b: &str) -> Option<&'static str> {
    let n = sub(rx!(r"_|[^\x00-\x7F]"), b, ".");
    let table: [(&'static str, &Regex); 9] = [
        ("8K", rx!(r"\b8K(?:\d+)?\b")),
        ("4K", rx!(r"\b4K(?:\d+)?\b")),
        ("4K", rx!(r"\b2160P(?:\d+)?\b")),
        ("1440p", rx!(r"\b1440P(?:\d+)?\b")),
        ("1080p", rx!(r"\b1080P(?:\d+)?\b")),
        ("720p", rx!(r"\b720P(?:\d+)?\b")),
        ("480p", rx!(r"\b480P(?:\d+)?\b")),
        ("HD", rx!(r"\bHD\b(?![.-]\d)")),
        ("SD", rx!(r"\bSD\b(?![.-]\d)")),
    ];
    table.iter().find(|(_, re)| is_match(re, &n)).map(|(v, _)| *v)
}

// ---------------------------------------------------------------------------
// 公共入口
// ---------------------------------------------------------------------------

fn components(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".").collect()
}

/// Python `Path.stem`:去掉最后一个后缀(隐藏文件的前导点不算)。
fn stem_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

/// 解析路径:文件名优先,未命中回退父目录;仍未命中用清理后的文件名(`matched = false`)。
pub fn parse_file_info(path: &str) -> FileInfo {
    parse_file_info_with(path, &[])
}

pub fn parse_file_info_with(path: &str, escape: &[String]) -> FileInfo {
    let mut parts = components(path);
    let name = parts.pop().unwrap_or("");
    let stem = stem_of(name);
    let dirs = parts;
    let (number, ty, matched) = identify(stem.trim(), &dirs, escape, true);
    let ty = classify_from_path(stem, &dirs).unwrap_or(ty);
    let basename = stem.to_uppercase();
    let cd = detect_cd(&basename).or_else(|| dirs.last().and_then(|d| detect_cd_from_parent(d)));
    let mosaic = detect_mosaic(&basename).or_else(|| detect_mosaic_from_dirs(&dirs));
    finish(number, ty, matched, cd, &basename, mosaic)
}

/// 解析自由文本(RSS 标题、已有番号):只执行番号规则,未命中则 `number` 为 None。
pub fn parse_text(text: &str) -> FileInfo {
    let (number, ty, matched) = identify(text, &[], &[], false);
    let name = components(text).pop().unwrap_or("");
    let basename = stem_of(name).to_uppercase();
    let cd = detect_cd(&basename);
    let mosaic = detect_mosaic(&basename);
    finish(number, ty, matched, cd, &basename, mosaic)
}

fn finish(number: Option<String>, ty: ContentType, matched: bool, cd: Option<u8>, basename: &str, mosaic: Option<Mosaic>) -> FileInfo {
    FileInfo {
        prefix: number.as_deref().map(prefix_of).unwrap_or_default(),
        number,
        content_type: ty,
        cd,
        has_subtitle: detect_subtitle(basename),
        mosaic: fill_mosaic(mosaic, ty),
        definition: detect_definition(basename),
        matched,
    }
}

/// 未命中返回 None,不会把原文冒充番号。
pub fn extract_number(text: &str) -> Option<String> {
    parse_text(text).number
}

// ---------------------------------------------------------------------------
// 供数据源使用的精简视图
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodeKind {
    /// ABC-123
    Standard,
    /// FC2-1234567
    Fc2,
    /// 010120_001(六位日期型)
    Dated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Code {
    pub kind: CodeKind,
    /// amane 口径的番号,如 `ABC-123`、`FC2-1234567`、`010115-001`
    pub id: String,
    pub content_type: ContentType,
    /// 分片序号(CD1/part2/-A…),从 1 开始
    pub part: Option<u8>,
    pub subtitled: bool,
    /// 文件名/路径给出的马赛克相位(破解/流出/无码),用于封面角标
    #[serde(default)]
    pub mosaic: Option<Mosaic>,
    /// 文件名里的清晰度(`4K`、`1080p`…),用于封面角标
    #[serde(default)]
    pub definition: Option<String>,
}

impl Code {
    /// 站点检索用的键:去掉番号数字部分多余的前导零(至少保留 3 位),如 `ABC-00123` → `ABC-123`。
    pub fn search_key(&self) -> String {
        search_key(&self.id)
    }
}

pub fn search_key(id: &str) -> String {
    match caps(rx!(r"^([A-Za-z]+)-(\d+)$"), id) {
        Some(m) => format!("{}-{:0>3}", &m[1], m[2].trim_start_matches('0')),
        None => id.to_string(),
    }
}

impl Code {
    /// 由解析结果构造可用于爬取的番号;兜底值(`matched = false`)返回 None。
    pub fn from_info(info: &FileInfo) -> Option<Code> {
        if !info.matched {
            return None;
        }
        let id = info.number.clone()?;
        let kind = if info.content_type == ContentType::Fc2 {
            CodeKind::Fc2
        } else if is_match(rx!(r"^\d{6}[-_]\d{2,4}$"), &id) {
            CodeKind::Dated
        } else {
            CodeKind::Standard
        };
        Some(Code { kind, id, content_type: info.content_type, part: info.cd, subtitled: info.has_subtitle, mosaic: info.mosaic, definition: info.definition.map(String::from) })
    }
}

/// 从路径解析出可用于爬取的番号;兜底值(`matched = false`)返回 None。
pub fn parse_code(input: &str) -> Option<Code> {
    Code::from_info(&parse_file_info(input))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> String {
        parse_code(s).map(|c| c.id).unwrap_or_default()
    }

    #[test]
    fn standard_variants() {
        assert_eq!(id("ABC-123.mp4"), "ABC-123");
        assert_eq!(id("abc123.mkv"), "ABC-123");
        assert_eq!(id("/media/dl/SMPL-001-C.mp4"), "SMPL-001");
        // amane 口径:不去零,检索时再用 search_key 归一
        assert_eq!(id("[HD] abc_00123 1080p.mp4"), "ABC-00123");
        assert_eq!(search_key("ABC-00123"), "ABC-123");
        assert_eq!(search_key("ABC-1234"), "ABC-1234");
    }

    #[test]
    fn fc2_and_dated() {
        assert_eq!(id("FC2-PPV-1234567.mp4"), "FC2-1234567");
        assert_eq!(parse_code("fc2ppv_1234567.mp4").unwrap().kind, CodeKind::Fc2);
        assert_eq!(id("010120_001.mp4"), "010120_001");
        assert_eq!(parse_code("010120-001.mp4").unwrap().kind, CodeKind::Dated);
    }

    #[test]
    fn parts_and_subtitles() {
        assert_eq!(parse_code("ABC-123-CD2.mp4").unwrap().part, Some(2));
        let c = parse_code("ABC-123-C.mp4").unwrap();
        assert!(c.subtitled && c.part.is_none());
        assert_eq!(parse_code("ABC-123-B.mp4").unwrap().part, Some(2));
        assert_eq!(parse_code("ABC-123.part3.mp4").unwrap().part, Some(3));
        let c = parse_code("hhd800.com@FC2-PPV-4817594_2.mp4").unwrap();
        assert_eq!((c.id.as_str(), c.part), ("FC2-4817594", Some(2)));
        let c = parse_code("489155.com@FC2PPV-3253595-1-C.mp4").unwrap();
        assert_eq!((c.id.as_str(), c.part, c.subtitled), ("FC2-3253595", Some(1), true));
        assert_eq!(parse_code("KLMN-377-UC.mp4").unwrap().part, None);
    }

    #[test]
    fn part_marker_variants() {
        let part = |name: &str| parse_code(name).map(|c| (c.id, c.part));
        assert_eq!(part("SMPL-003 CD2.mp4"), Some(("SMPL-003".into(), Some(2))));
        assert_eq!(part("SMPL-010-disc2.mp4"), Some(("SMPL-010".into(), Some(2))));
        assert_eq!(part("SMPL-010.DISK1.mp4"), Some(("SMPL-010".into(), Some(1))));
        assert_eq!(part("SMPL-012-pt2.mp4"), Some(("SMPL-012".into(), Some(2))));
        let c = parse_code("SMPL-020-B-C.mp4").unwrap();
        assert_eq!((c.part, c.subtitled), (Some(2), true));
        // 浏览器/Finder 的重复下载副本,不是分片
        assert_eq!(part("SMPL-011 (2).mp4"), Some(("SMPL-011".into(), None)));
        // 10musume 番号本身以 `_NN` 结尾
        assert_eq!(part("010120_01.mp4").and_then(|c| c.1), None);
    }

    #[test]
    fn unmatched_names_are_not_codes() {
        // 路径模式有兜底值(与 amane 一致),但 parse_code 不把兜底值当番号
        assert!(parse_file_info("random.mp4").number.is_some());
        assert_eq!(parse_code("random.mp4"), None);
        assert_eq!(parse_code("movie x264 1080p.mp4"), None);
        assert_eq!(extract_number("Weekly Update"), None);
    }

    #[test]
    fn windows_paths() {
        let i = parse_file_info(r"D:\Downloads\新番\SMPL-001-CD2.mkv");
        assert_eq!((i.number.as_deref(), i.cd, i.matched), (Some("SMPL-001"), Some(2), true));
        let i = parse_file_info(r"C:\Media\MOVX-123\video.mp4"); // 目录回退
        assert_eq!(i.number.as_deref(), Some("MOVX-123"));
    }

    #[test]
    fn phases() {
        let i = parse_file_info("[4K無碼]MOVX-123-UC-CD1.mp4");
        assert_eq!((i.cd, i.has_subtitle, i.mosaic, i.definition), (Some(1), true, Some(Mosaic::Cracked), Some("4K")));
        let i = parse_file_info("/media/[無碼]/ABC-123.mp4");
        assert_eq!(i.mosaic, Some(Mosaic::Uncensored));
    }

    /// 常见下载后缀 → (番号, 中字, 马赛克相位)
    #[test]
    fn common_suffixes() {
        use Mosaic::*;
        let cases = [
            ("SMPL-001.mp4", false, Censored),
            ("SMPL-001-C.mp4", true, Censored),
            ("smpl-001-ch.mp4", true, Censored),
            ("SMPL-001-U.mp4", false, Cracked),
            ("SMPL-001-UC.mp4", true, Cracked),
            ("SMPL-001-UMR.mp4", false, Cracked),
            ("SMPL-001-UC-CD1.mp4", true, Cracked),
            ("SMPL-001-leak.mp4", false, Leaked),
            ("SMPL-001-LEAKED.mp4", false, Leaked),
            ("SMPL-001-LEAKC.mp4", true, Leaked),
            ("SMPL-001 流出.mp4", false, Leaked),
            ("SMPL-001-C 流出.mp4", true, Leaked),
            ("[破解]SMPL-001.mp4", false, Cracked),
            ("SMPL-001-无码破解.mp4", false, Cracked),
            ("HEYZO-1234.mp4", false, Uncensored),
            ("HEYZO-1234-C.mp4", true, Uncensored),
            ("/dl/流出/SMPL-001.mp4", false, Leaked),
        ];
        for (name, sub, mosaic) in cases {
            let i = parse_file_info(name);
            let n = if name.contains("HEYZO") { "HEYZO-1234" } else { "SMPL-001" };
            assert_eq!((i.number.as_deref(), i.has_subtitle, i.mosaic), (Some(n), sub, Some(mosaic)), "{name}");
            let c = parse_code(name).unwrap();
            assert_eq!((c.subtitled, c.mosaic), (sub, Some(mosaic)), "{name}");
        }
        // 不能把番号里的字母误判为后缀
        assert_eq!(parse_file_info("CHN-123.mp4").has_subtitle, false);
        assert_eq!(parse_file_info("UVW-123.mp4").mosaic, Some(Censored));
        assert_eq!(parse_file_info("ABC-123-CHS.mp4").has_subtitle, false);
    }
}
