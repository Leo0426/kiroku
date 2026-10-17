//! FC2 official public detail; selectors adapted from amane 3006c80 (GPL-3.0).
use super::common::*;
use crate::{
    code::{Code, ContentType},
    metadata::PartialMetadata,
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use scraper::Html;
pub struct Fc2 {
    pub base: String,
    pub cookie: String,
}
pub fn parse_detail(body: &str, id: &str) -> Result<PartialMetadata, SourceError> {
    body_error(body)?;
    if body.trim().is_empty() {
        return Err(SourceError::Parse("FC2 空响应".into()));
    }
    let d = Html::parse_document(body);
    let title = text(&d, "div[data-section=userInfo] h3").ok_or_else(|| {
        if text(&d, "title").is_some_and(|t| t.contains("お探しの商品が見つかりませんでした"))
        {
            SourceError::NotFound
        } else {
            SourceError::Parse("FC2 响应不是作品详情或站点结构已变化".into())
        }
    })?;
    let url = attrs(&d, "meta[property='og:url']", "content")
        .into_iter()
        .next()
        .ok_or_else(|| SourceError::Parse("FC2 缺少作品身份 og:url".into()))?;
    if reqwest::Url::parse(&url)
        .ok()
        .and_then(|u| {
            u.path()
                .strip_prefix("/article/")
                .map(|s| s.trim_end_matches('/').to_string())
        })
        .as_deref()
        .is_none_or(|value| !numeric_equal(value, id))
    {
        return Err(SourceError::Parse("FC2 返回了不同商品".into()));
    }
    let mut m = PartialMetadata {
        original_title: Some(title.clone()),
        title: Some(title),
        studio: text(&d, ".items_article_headerInfo ul li:last-child a"),
        plot: attrs(&d, "meta[name=description]", "content")
            .into_iter()
            .next(),
        external_id: Some(id.into()),
        ..Default::default()
    };
    m.poster_urls = attrs(&d, ".items_article_MainitemThumb span img", "src")
        .into_iter()
        .filter_map(|s| absolute(&url, &s))
        .collect();
    m.extrafanart = attrs(&d, ".items_article_SampleImagesArea li a", "href")
        .into_iter()
        .filter_map(|s| absolute(&url, &s))
        .collect();
    m.thumb_urls = m.extrafanart.first().cloned().into_iter().collect();
    m.tags = d
        .select(&sel("a.tagTag"))
        .map(|e| e.text().collect::<String>().trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if let Some(t) = text(&d, ".items_article_Releasedate p") {
        if let Some(c) = regex::Regex::new(r"(\d{4})/(\d{1,2})/(\d{1,2})")
            .unwrap()
            .captures(&t)
        {
            m.release_date = Some(format!(
                "{}-{:02}-{:02}",
                &c[1],
                c[2].parse::<u32>().unwrap(),
                c[3].parse::<u32>().unwrap()
            ))
        }
    }
    Ok(m)
}
#[async_trait]
impl Source for Fc2 {
    async fn check_connectivity(&self,http:&Http)->crate::connectivity::ConnectivityOutcome {crate::connectivity::probe_html(http,&self.base,self.cookie.as_str(),"",&["fc2", "items_article"]).await}
    fn id(&self) -> SourceId {
        "fc2"
    }
    fn supports(&self, c: &Code) -> bool {
        c.content_type == ContentType::Fc2
    }
    async fn fetch(&self, c: &Code, h: &Http) -> Result<PartialMetadata, SourceError> {
        let id = fc2_id(&c.id)?;
        let url = format!("{}/article/{id}/", self.base.trim_end_matches('/'));
        h.seed_cookies(&url, &self.cookie);
        let p = h
            .get_text(&url)
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        let mut m = parse_detail(&p.body, &id)?;
        m.source_url = Some(p.url);
        Ok(m)
    }

    async fn fetch_detail(&self, url: &str, c: &Code, h: &Http) -> Option<Result<(PartialMetadata, String), SourceError>> {
        Some(async {
            let id = fc2_id(&c.id)?;
            h.seed_cookies(url, &self.cookie);
            let p = h.get_text(url).await.map_err(|e| SourceError::Network(e.to_string()))?;
            check(&p)?;
            let mut m = parse_detail(&p.body, &id)?;
            m.source_url = Some(p.url);
            Ok((m, p.body))
        }.await)
    }
}
