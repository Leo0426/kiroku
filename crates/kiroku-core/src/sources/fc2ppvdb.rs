//! FC2PPVDB legacy public HTML selectors (amane 3006c80, GPL-3.0).
//! No positive live fixture available: current public host returns HTTP 526.
use super::common::*;
use crate::{
    code::{Code, ContentType},
    metadata::{Actor, Gender, PartialMetadata},
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use scraper::Html;
pub struct Fc2PpvDb {
    pub base: String,
    pub cookie: String,
}
pub fn parse_detail(body: &str, id: &str, url: &str) -> Result<PartialMetadata, SourceError> {
    body_error(body)?;
    if body.trim().is_empty() {
        return Err(SourceError::Parse("FC2PPVDB 空响应；不能判为未命中".into()));
    }
    let d = Html::parse_document(body);
    let title = text(&d, "h1.title, h2.title")
        .ok_or_else(|| SourceError::Parse("FC2PPVDB 响应不是作品详情或站点结构已变化".into()))?;
    let identity = attrs(&d, "link[rel=canonical]", "href")
        .into_iter()
        .next()
        .or_else(|| {
            attrs(&d, "meta[property='og:url']", "content")
                .into_iter()
                .next()
        })
        .unwrap_or_else(|| url.into());
    if reqwest::Url::parse(&identity)
        .ok()
        .and_then(|u| {
            u.path()
                .strip_prefix("/articles/")
                .map(|v| v.trim_end_matches('/').to_string())
        })
        .as_deref()
        .is_none_or(|value| !numeric_equal(value, id))
    {
        return Err(SourceError::Parse("FC2PPVDB 返回了不同商品".into()));
    }
    let mut m = PartialMetadata {
        title: Some(title.clone()),
        original_title: Some(title),
        external_id: Some(id.into()),
        ..Default::default()
    };
    m.actors = d
        .select(&sel("a[href*='/actresses/']"))
        .map(|e| Actor::new(e.text().collect::<String>().trim(), Gender::Female))
        .filter(|a| !a.name.is_empty())
        .collect();
    m.tags = d
        .select(&sel("a[href*='/tags/']"))
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();
    m.thumb_urls = attrs(&d, ".article-image img", "src")
        .into_iter()
        .filter_map(|s| absolute(url, &s))
        .collect();
    let raw = attrs(&d, "time", "datetime")
        .into_iter()
        .next()
        .or_else(|| text(&d, "time"));
    if let Some(raw) = raw {
        m.release_date = regex::Regex::new(r"\d{4}[-/]\d{2}[-/]\d{2}")
            .unwrap()
            .find(&raw)
            .map(|m| m.as_str().replace('/', "-"))
    }
    Ok(m)
}
#[async_trait]
impl Source for Fc2PpvDb {
    async fn check_connectivity(&self,http:&Http)->crate::connectivity::ConnectivityOutcome {crate::connectivity::probe_html(http,&self.base,self.cookie.as_str(),"",&["fc2ppvdb", "/articles/"]).await}
    fn id(&self) -> SourceId {
        "fc2ppvdb"
    }
    fn supports(&self, c: &Code) -> bool {
        c.content_type == ContentType::Fc2
    }
    async fn fetch(&self, c: &Code, h: &Http) -> Result<PartialMetadata, SourceError> {
        let id = fc2_id(&c.id)?;
        let url = format!("{}/articles/{id}", self.base.trim_end_matches('/'));
        h.seed_cookies(&url, &self.cookie);
        let p = h
            .get_text(&url)
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        let mut m = parse_detail(&p.body, &id, &p.url)?;
        m.source_url = Some(p.url);
        Ok(m)
    }
}
