use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Gender {
    Female,
    Male,
    /// 栏位无法判定性别时保持 unknown;名单语义能判定(如出演栏默认女优)时必须写出
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub name: String,
    #[serde(default)]
    pub gender: Gender,
    /// 演员头像 URL(ADR-0009);旧 sidecar 没有该键。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
}

impl Actor {
    pub fn with_thumb(mut self, url: impl Into<String>) -> Self {
        self.thumb = Some(url.into());
        self
    }

    /// 站点「暂无头像」占位图不算头像。
    pub fn real_thumb(url: &str) -> bool {
        let lower = url.to_ascii_lowercase();
        !["nowprinting", "noimage", "no_image", "printing.gif"].iter().any(|p| lower.contains(p))
    }
    pub fn new(name: impl Into<String>, gender: Gender) -> Self {
        Self {
            name: name.into(),
            gender,
            thumb: None,
        }
    }
}

/// 单个数据源返回的部分元数据;缺失字段为 None / 空。字段口径对齐 amane `MediaMetadata`。
/// 外部站点 schema 不受控,所以一律可空,解析时单个字段失败不应让整条丢失。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartialMetadata {
    pub title: Option<String>,
    /// kiroku 额外字段:站点给出的日文原题(amane 靠 field_language 选语言)
    pub original_title: Option<String>,
    pub plot: Option<String>,
    /// YYYY-MM-DD
    pub release_date: Option<String>,
    pub runtime_min: Option<u32>,
    /// 片商(メーカー)
    pub studio: Option<String>,
    /// 发行商(レーベル);集团多厂牌站必须写真实发行商,不能用站点名顶替
    pub publisher: Option<String>,
    pub series: Option<String>,
    pub actors: Vec<Actor>,
    pub tags: Vec<String>,
    pub directors: Vec<String>,
    /// 竖版封面
    pub poster_urls: Vec<String>,
    /// 横版封面(封底|封面),poster 可由此右侧裁出
    pub thumb_urls: Vec<String>,
    pub trailer_urls: Vec<String>,
    /// 剧照/样张
    pub extrafanart: Vec<String>,
    /// 0–100;各站评分体系不同,由数据源负责归一
    pub score: Option<f32>,
    pub external_id: Option<String>,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourcedScore {
    pub site: String,
    pub score: f32,
}

/// Successful pre-AI source data. No credentials are persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceSnapshot {
    pub code: String,
    pub content_type: crate::code::ContentType,
    pub base: String,
    pub fetched_at: u64,
    #[serde(default)]
    pub base_identity: String,
    #[serde(default)]
    pub cache_identity: String,
    pub fields: PartialMetadata,
}
impl SourceSnapshot {
    pub fn new(code: &crate::code::Code, base: &str, fields: &PartialMetadata) -> Self {
        let mut fields = fields.clone();
        fields.source_url = fields.source_url.map(|s| public_url(&s));
        for urls in [
            &mut fields.poster_urls,
            &mut fields.thumb_urls,
            &mut fields.trailer_urls,
            &mut fields.extrafanart,
        ] {
            for url in urls {
                *url = public_url(url);
            }
        }
        Self {
            code: code.id.clone(),
            content_type: code.content_type,
            base: public_url(base),
            base_identity: base_digest(base),
            cache_identity: config_digest(base, "", None, ""),
            fetched_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            fields,
        }
    }
    pub fn configured(
        code: &crate::code::Code,
        cfg: &crate::settings::SourceCfg,
        proxy: Option<&str>,
        fields: &PartialMetadata,
    ) -> Self {
        let mut snapshot = Self::new(code, &cfg.base, fields);
        snapshot.cache_identity = config_digest(&cfg.base, &cfg.cookie, proxy, cfg.effective_api_base());
        snapshot
    }
    pub fn matches_config(
        &self,
        code: &crate::code::Code,
        cfg: &crate::settings::SourceCfg,
        proxy: Option<&str>,
    ) -> bool {
        self.matches(code, &cfg.base)
            && self.cache_identity == config_digest(&cfg.base, &cfg.cookie, proxy, cfg.effective_api_base())
    }
    pub fn matches(&self, code: &crate::code::Code, base: &str) -> bool {
        self.code == code.id
            && self.content_type == code.content_type
            && self.base == public_url(base)
            && self.base_identity == base_digest(base)
            && usable_partial(&self.fields)
    }
}
pub fn public_url(value: &str) -> String {
    if let Ok(mut url) = reqwest::Url::parse(value) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.to_string()
    } else {
        value.into()
    }
}
fn config_digest(base: &str, cookie: &str, proxy: Option<&str>, api_base: &str) -> String {
    // Preserve Task3 identities for sources with no second origin.
    let value = if api_base.is_empty() { serde_json::to_string(&(base, cookie, proxy)) }
        else { serde_json::to_string(&(base, cookie, proxy, api_base)) };
    base_digest(&value.expect("string tuple"))
}
fn base_digest(value: &str) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(value.as_bytes()))
}
pub fn usable_partial(fields: &PartialMetadata) -> bool {
    serde_json::to_value(fields)
        .ok()
        .and_then(|v| {
            v.as_object().map(|o| {
                o.iter().any(|(k, v)| {
                    !matches!(k.as_str(), "source_url" | "external_id")
                        && crate::refresh::nonempty(v)
                })
            })
        })
        .unwrap_or(false)
}

/// 聚合后的最终元数据。
/// `fields` 中:标量字段为链上第一个非空值;poster/thumb/trailer 为沿链拼接去重;
/// `extrafanart`/`score`/`external_id`/`source_url` 不在 `fields` 内聚合,见下方专用字段。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    /// Import-time concrete video stem for NFO/poster/fanart. None is legacy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_stem: Option<String>,
    #[serde(default)]
    pub placements: Vec<crate::placement::Placement>,
    #[serde(default)]
    pub text_versions: crate::localization::TextVersions,
    #[serde(default)]
    pub nfo_language: crate::localization::NfoLanguage,
    /// Original and applied name fields, independent from raw source snapshots.
    #[serde(default)]
    pub name_projection: BTreeMap<String, crate::names::NameProjectionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork_phase: Option<crate::badge::Phase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artwork: Option<crate::artwork::Artwork>,
    pub code: String,
    #[serde(flatten)]
    pub fields: PartialMetadata,
    /// 剧照按来源分组保留(不跨站混合)
    pub extrafanart_by_source: BTreeMap<String, Vec<String>>,
    /// 各站评分体系不同,必须保留来源
    pub scores: Vec<SourcedScore>,
    pub external_ids: BTreeMap<String, String>,
    pub source_urls: BTreeMap<String, String>,
    /// 每个字段最终采用的数据源(用于审计)
    pub provenance: BTreeMap<String, String>,
    /// Locked values include explicitly empty manual edits.
    #[serde(default)]
    pub locked_fields: std::collections::BTreeSet<String>,
    /// 用户收藏;只存 sidecar,未收藏时不写出
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub favorite: bool,
    /// 本次刮削的提示(如来源结果不一致),只随刮削事件展示,不落盘
    #[serde(skip)]
    pub scrape_notes: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_snapshots")]
    pub raw_sources: BTreeMap<String, SourceSnapshot>,
}

impl Metadata {
    pub fn year(&self) -> Option<&str> {
        self.fields.release_date.as_deref().and_then(|d| d.get(..4))
    }

    /// 多源评分的算术平均(0–100)。
    pub fn mean_score(&self) -> Option<f32> {
        (!self.scores.is_empty())
            .then(|| self.scores.iter().map(|s| s.score).sum::<f32>() / self.scores.len() as f32)
    }
}

fn deserialize_snapshots<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, SourceSnapshot>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_object()
        .into_iter()
        .flat_map(|map| map.iter())
        .filter_map(|(site, value)| {
            serde_json::from_value(value.clone())
                .ok()
                .map(|snapshot| (site.clone(), snapshot))
        })
        .collect())
}
