//! XCity selectors from actual 2026-10-09 responses and amane 3006c80 (GPL-3.0).
use super::common::*;
use crate::{
    code::{Code, ContentType},
    metadata::{Actor, Gender, PartialMetadata},
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use scraper::Html;
pub struct XCity {
    pub base: String,
    pub cookie: String,
}
/// Search listings omit manufacturer codes. Return only film candidates, then
/// fetch MUST verify each detail's manufacturer identity; no first-hit fallback.
pub fn parse_search(html: &str, base: &str, code: &Code) -> Result<Vec<String>, SourceError> {
    validate_body(html)?;
    let d = Html::parse_document(html);
    if let Some(id) = text(&d, "#hinban, span.kopin") {
        return Ok(if equal_code(&id, code) {
            vec![base.into()]
        } else {
            vec![]
        });
    }
    if d.select(&sel("#searchResult, .resultList, #centerCol h4"))
        .next()
        .is_none()
    {
        return Err(SourceError::Parse("XCity 未返回可识别的搜索页".into()));
    }
    let mut out = vec![];
    for s in attrs(
        &d,
        "table.resultList a[href*='/avod/detail/'], table.resultList a[href*='/titles/']",
        "href",
    ) {
        if let Some(u) = same_origin(base, &s) {
            if !out.contains(&u) {
                out.push(u)
            }
        }
    }
    Ok(out)
}
pub fn parse_detail(html: &str, base: &str, code: &Code) -> Result<PartialMetadata, SourceError> {
    validate_body(html)?;
    let d = Html::parse_document(html);
    let id = text(&d, "#hinban, span.kopin")
        .ok_or_else(|| SourceError::Parse("XCity 详情缺少メーカー品番；不能确认作品身份".into()))?;
    if !equal_code(&id, code) {
        return Err(SourceError::NotFound);
    }
    let title = text(&d, "#program_detail_title, h1")
        .ok_or_else(|| SourceError::Parse("XCity 详情缺少标题".into()))?;
    let mut m = PartialMetadata {
        title: Some(title.clone()),
        original_title: Some(title),
        studio: text(&d, "#program_detail_maker_name, a[href*='/studios/']"),
        actors: links(
            &d,
            ".credit-links a[href*='/idol/'], .profileCL a[href*='/idol/'], .profile a[href*='/idol/']",
        )
        .into_iter()
        .map(|n| Actor::new(n, Gender::Female))
        .collect(),
        tags: links(&d, ".profileCL a.genre, .profile a[href*='/genres/']"),
        source_url: Some(base.into()),
        external_id: reqwest::Url::parse(base).ok().and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "id")
                .map(|(_, v)| v.to_string())
        }),
        ..Default::default()
    };
    for li in d.select(&sel("li")) {
        let label = li
            .select(&sel("span.koumoku, span"))
            .next()
            .map(|e| e.text().collect::<String>())
            .unwrap_or_default();
        let raw = li.text().collect::<String>();
        if label == "発売日" {
            m.release_date = regex::Regex::new(r"\d{4}[/\-]\d{2}[/\-]\d{2}")
                .unwrap()
                .find(&raw)
                .map(|v| v.as_str().replace('/', "-"))
        } else if label == "収録時間" {
            m.runtime_min = regex::Regex::new(r"\d+")
                .unwrap()
                .find(&raw)
                .and_then(|v| v.as_str().parse().ok())
        }
    }
    let mut covers = attrs(&d, ".photo a[rel='lightbox']", "href");
    if covers.is_empty() {
        covers = attrs(&d, "img.photo, .photo img.packageThumb", "src")
    }
    if covers.is_empty() {
        covers = attrs(&d, "a.fancy_image", "href")
    }
    m.thumb_urls = covers
        .into_iter()
        .filter_map(|s| absolute(base, &s))
        .collect();
    Ok(m)
}
#[async_trait]
impl Source for XCity {
    fn id(&self) -> SourceId {
        "xcity"
    }
    fn supports(&self, c: &Code) -> bool {
        c.content_type == ContentType::Censored
    }
    async fn check_connectivity(&self, h: &Http) -> crate::connectivity::ConnectivityOutcome {
        let u = format!("{}/result/?q=ADZ335", self.base.trim_end_matches('/'));
        crate::connectivity::probe_html(
            h,
            &u,
            &self.cookie,
            "adc=1; over18=1",
            &["resultlist", "searchresult"],
        )
        .await
    }
    async fn fetch(&self, c: &Code, h: &Http) -> Result<PartialMetadata, SourceError> {
        if !self.supports(c) {
            return Err(SourceError::NotFound);
        }
        let mut url = reqwest::Url::parse(&format!("{}/result/", self.base.trim_end_matches('/')))
            .map_err(|e| SourceError::Parse(e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("q", &c.search_key().replace('-', ""));
        h.seed_cookies(url.as_str(), "adc=1; over18=1");
        h.seed_cookies(url.as_str(), &super::javdb::clean_cookie(&self.cookie));
        let p = h
            .get_text(url.as_str())
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        if Html::parse_document(&p.body)
            .select(&sel("#hinban, span.kopin"))
            .next()
            .is_some()
        {
            return parse_detail(&p.body, &p.url, c);
        }
        for url in parse_search(&p.body, &p.url, c)? {
            let p = h
                .get_text(&url)
                .await
                .map_err(|e| SourceError::Network(e.to_string()))?;
            check(&p)?;
            match parse_detail(&p.body, &p.url, c) {
                Ok(m) => return Ok(m),
                Err(SourceError::NotFound) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(SourceError::NotFound)
    }

    async fn fetch_detail(&self, url: &str, c: &Code, h: &Http) -> Option<Result<(PartialMetadata, String), SourceError>> {
        Some(async {
            h.seed_cookies(url, "adc=1; over18=1");
            h.seed_cookies(url, &super::javdb::clean_cookie(&self.cookie));
            let p = h.get_text(url).await.map_err(|e| SourceError::Network(e.to_string()))?;
            check(&p)?;
            let m = parse_detail(&p.body, &p.url, c)?;
            Ok((m, p.body))
        }.await)
    }
}
