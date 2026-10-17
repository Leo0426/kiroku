//! Avsox anonymous CSRF JSON API. Field semantics: amane 3006c80, GPL-3.0.
use super::common::*;
use crate::{
    code::{Code, ContentType},
    metadata::{Actor, Gender, PartialMetadata},
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use scraper::Html;
use serde_json::{json, Value};
pub struct Avsox {
    pub base: String,
    pub cookie: String,
}
pub fn csrf_token(body: &str) -> Result<String, SourceError> {
    body_error(body)?;
    attrs(
        &Html::parse_document(body),
        "meta[name=csrf-token]",
        "content",
    )
    .into_iter()
    .next()
    .ok_or_else(|| SourceError::Parse("Avsox 页面缺少 CSRF token；检查域名或登录/拦截页".into()))
}
pub fn parse_search(body: &str, code: &str) -> Result<Option<String>, SourceError> {
    let v = envelope(body)?;
    let rows = v
        .as_array()
        .ok_or_else(|| SourceError::Parse("search data 不是数组".into()))?;
    for exact in [true, false] {
        for row in rows {
            let n = nonempty(row, "movieFanHao")
                .ok_or_else(|| SourceError::Parse("搜索项缺少 movieFanHao".into()))?;
            let matched = if exact {
                n.eq_ignore_ascii_case(code)
            } else {
                fold(&n) == fold(code)
            };
            if matched {
                return nonempty(row, "movieId")
                    .map(Some)
                    .ok_or_else(|| SourceError::Parse("搜索项缺少 movieId".into()));
            }
        }
    }
    Ok(None)
}
fn entity(v: &Value, key: &str, name: &str) -> Option<String> {
    let v = v.get(key)?;
    nonempty(
        if let Some(a) = v.as_array() {
            a.first()?
        } else {
            v
        },
        name,
    )
}
pub fn parse_detail(body: &str, code: &str) -> Result<PartialMetadata, SourceError> {
    let v = envelope(body)?;
    if v.is_null() {
        return Err(SourceError::NotFound);
    }
    if !v.is_object() {
        return Err(SourceError::Parse("getMovie data 不是对象".into()));
    }
    if nonempty(&v, "movieFanHao").is_none_or(|n| fold(&n) != fold(code)) {
        return Err(SourceError::Parse("Avsox 返回了不同番号".into()));
    }
    let title = nonempty(&v, "title")
        .or_else(|| nonempty(&v, "title_ja"))
        .ok_or_else(|| SourceError::Parse("详情缺少标题".into()))?;
    let mut m = PartialMetadata {
        title: Some(title),
        original_title: nonempty(&v, "title_ja"),
        plot: nonempty(&v, "description_cn").or_else(|| nonempty(&v, "description_ja")),
        release_date: nonempty(&v, "releaseDate"),
        runtime_min: v["length"].as_u64().and_then(|n| u32::try_from(n).ok()),
        studio: entity(&v, "studio", "studioName"),
        publisher: entity(&v, "label", "labelName"),
        series: entity(&v, "series", "seriesName"),
        external_id: nonempty(&v, "movieId"),
        ..Default::default()
    };
    for (key, name) in [
        ("star", "starName"),
        ("genre", "genreName"),
        ("director", "directorName"),
    ] {
        if let Some(a) = v[key].as_array() {
            for e in a {
                if let Some(n) = nonempty(e, name) {
                    match key {
                        "star" => m.actors.push(Actor::new(n, Gender::Female)),
                        "genre" => m.tags.push(n),
                        _ => m.directors.push(n),
                    }
                }
            }
        } else if let Some(n) = entity(&v, key, name) {
            match key {
                "star" => m.actors.push(Actor::new(n, Gender::Female)),
                "genre" => m.tags.push(n),
                _ => m.directors.push(n),
            }
        }
    }
    m.poster_urls = nonempty(&v, "posterSmall").into_iter().collect();
    m.thumb_urls = nonempty(&v, "posterLarge").into_iter().collect();
    m.extrafanart = v["sampleLarge"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(m)
}
#[async_trait]
impl Source for Avsox {
    async fn check_connectivity(&self,http:&Http)->crate::connectivity::ConnectivityOutcome {crate::connectivity::probe_html(http,&self.base,self.cookie.as_str(),"",&["csrf-token", "avsox"]).await}
    fn id(&self) -> SourceId {
        "avsox"
    }
    fn supports(&self, c: &Code) -> bool {
        c.content_type == ContentType::Uncensored
    }
    async fn fetch(&self, c: &Code, h: &Http) -> Result<PartialMetadata, SourceError> {
        let base = self.base.trim_end_matches('/');
        let shell = format!("{base}/cn/");
        h.seed_cookies(&shell, &self.cookie);
        let p = h
            .get_text(&shell)
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        let token = csrf_token(&p.body)?;
        let origin = reqwest::Url::parse(base)
            .map_err(|e| SourceError::Parse(e.to_string()))?
            .origin()
            .ascii_serialization();
        let referer = format!("{base}/cn/search/{}", c.id);
        let headers = [
            ("Origin", origin.as_str()),
            ("Referer", referer.as_str()),
            ("X-Requested-With", "XMLHttpRequest"),
            ("X-CSRF-Token", token.as_str()),
            ("Accept", "application/json, text/plain, */*"),
        ];
        let p = h
            .post_json_text(
                &format!("{base}/javu/data/api/search"),
                &json!([{"search":c.id,"lang":"cn"},60,1]),
                &headers,
            )
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        let id = parse_search(&p.body, &c.id)?.ok_or(SourceError::NotFound)?;
        let p = h
            .post_json_text(
                &format!("{base}/javu/data/api/getMovie"),
                &json!([id, "cn"]),
                &headers,
            )
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        let mut m = parse_detail(&p.body, &c.id)?;
        if m.external_id.as_deref() != Some(&id) {
            return Err(SourceError::Parse("Avsox 详情 movieId 不匹配".into()));
        }
        m.source_url = Some(format!("{base}/cn/movies/{id}"));
        Ok(m)
    }
}
