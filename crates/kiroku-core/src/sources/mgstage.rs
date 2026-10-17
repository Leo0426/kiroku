//! MGStage selectors adapted from amane 3006c80 (GPL-3.0). Positive path
//! remains externally unverified: captured public responses are CloudFront 403.
use super::common::*;
use crate::{
    code::{Code, ContentType},
    metadata::{Actor, Gender, PartialMetadata},
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use scraper::Html;
pub struct Mgstage {
    pub base: String,
    pub cookie: String,
}
pub fn matches_product(url: &str, code: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path()
                .split("/product_detail/")
                .nth(1)
                .map(|s| s.trim_end_matches('/').to_string())
        })
        .is_some_and(|s| fold(&s) == fold(code))
}
pub fn parse_search(body: &str, code: &str, base: &str) -> Result<Option<String>, SourceError> {
    body_error(body)?;
    let d = Html::parse_document(body);
    if d.select(&sel("#center_column")).next().is_none() {
        return Err(SourceError::Parse("MGStage 未返回可识别的搜索页".into()));
    }
    Ok(attrs(&d, "a[href*='/product/product_detail/']", "href")
        .into_iter()
        .filter_map(|s| absolute(base, &s))
        .find(|s| matches_product(s, code)))
}
pub fn parse_detail(body: &str, code: &str, url: &str) -> Result<PartialMetadata, SourceError> {
    body_error(body)?;
    if body.trim().is_empty() {
        return Err(SourceError::Parse("MGStage 空响应".into()));
    }
    if !matches_product(url, code) {
        return Err(SourceError::Parse("MGStage 返回了不同商品 URL".into()));
    }
    let d = Html::parse_document(body);
    let title = text(&d, "#center_column > div:first-child h1")
        .ok_or_else(|| SourceError::Parse("MGStage 响应不是作品详情或站点结构已变化".into()))?;
    let mut m = PartialMetadata {
        title: Some(title.clone()),
        original_title: Some(title),
        external_id: Some(code.into()),
        plot: text(&d, "#introduction dd p.introduction"),
        ..Default::default()
    };
    for row in d.select(&sel("tr")) {
        let label = row
            .select(&sel("th"))
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();
        let Some(td) = row.select(&sel("td")).next() else {
            continue;
        };
        let raw = td.text().collect::<String>().trim().to_string();
        let links: Vec<String> = td
            .select(&sel("a"))
            .map(|e| e.text().collect::<String>().trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let value = links
            .first()
            .cloned()
            .or_else(|| (!raw.is_empty()).then_some(raw.clone()));
        if label.contains("出演") {
            m.actors = links
                .into_iter()
                .map(|n| Actor::new(n, Gender::Female))
                .collect()
        } else if label.contains("メーカー") {
            m.studio = value
        } else if label.contains("レーベル") {
            m.publisher = value
        } else if label.contains("シリーズ") {
            m.series = value
        } else if label.contains("収録時間") {
            m.runtime_min = regex::Regex::new(r"\d+")
                .unwrap()
                .find(&raw)
                .and_then(|n| n.as_str().parse().ok())
        } else if label.contains("配信開始日") {
            m.release_date = value.map(|v| v.replace('/', "-"))
        } else if label.contains("ジャンル") {
            m.tags = links
        } else if label.contains("品番") && !raw.is_empty() && fold(&raw) != fold(code) {
            return Err(SourceError::Parse("MGStage 详情品番不匹配".into()));
        }
    }
    m.thumb_urls = attrs(&d, "a#EnlargeImage", "href");
    if m.thumb_urls.is_empty() {
        m.thumb_urls = attrs(&d, "img.enlarge_image", "src")
    }
    m.thumb_urls = m
        .thumb_urls
        .into_iter()
        .filter_map(|s| absolute(url, &s))
        .collect();
    m.extrafanart = attrs(&d, "#sample-photo li a", "href")
        .into_iter()
        .filter_map(|s| absolute(url, &s))
        .collect();
    Ok(m)
}
#[async_trait]
impl Source for Mgstage {
    async fn check_connectivity(&self,http:&Http)->crate::connectivity::ConnectivityOutcome {crate::connectivity::probe_html(http,&self.base,self.cookie.as_str(),"adc=1",&["mgstage", "product_detail"]).await}
    fn id(&self) -> SourceId {
        "mgstage"
    }
    fn supports(&self, c: &Code) -> bool {
        c.content_type == ContentType::Amateur
    }
    async fn fetch(&self, c: &Code, h: &Http) -> Result<PartialMetadata, SourceError> {
        let base = self.base.trim_end_matches('/');
        let url = format!("{base}/product/product_detail/{}/", c.id);
        h.seed_cookies(&url, "adc=1");
        h.seed_cookies(&url, &self.cookie);
        let p = h
            .get_text(&url)
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        match check(&p).and_then(|_| parse_detail(&p.body, &c.id, &p.url)) {
            Ok(mut m) => {
                m.source_url = Some(p.url);
                return Ok(m);
            }
            Err(SourceError::NotFound) => {}
            Err(e) => return Err(e),
        }
        let mut search = reqwest::Url::parse(&format!("{base}/search/cSearch.php"))
            .map_err(|e| SourceError::Parse(e.to_string()))?;
        search.query_pairs_mut().append_pair("search_word", &c.id);
        let p = h
            .get_text(search.as_str())
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        let url = parse_search(&p.body, &c.id, base)?.ok_or(SourceError::NotFound)?;
        let p = h
            .get_text(&url)
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        let mut m = parse_detail(&p.body, &c.id, &p.url)?;
        m.source_url = Some(p.url);
        Ok(m)
    }
}
