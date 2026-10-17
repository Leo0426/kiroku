//! JavLibrary selectors adapted from amane 3006c80 (GPL-3.0-or-later).
//! Live normal responses currently blocked by Cloudflare; see fixture manifest.
use super::common::*;
use crate::{
    code::{Code, ContentType},
    metadata::{Actor, Gender, PartialMetadata},
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use scraper::Html;
pub struct JavLibrary {
    pub base: String,
    pub cookie: String,
}
pub fn parse_search(html: &str, base: &str, code: &Code) -> Result<Vec<String>, SourceError> {
    validate_body(html)?;
    let d = Html::parse_document(html);
    if let Some(id) = text(&d, "#video_id .text") {
        return if equal_code(&id, code) {
            Ok(vec![base.into()])
        } else {
            Ok(vec![])
        };
    }
    if d.select(&sel(".videothumblist, #rightcolumn, #video_search"))
        .next()
        .is_none()
    {
        return Err(SourceError::Parse("JavLibrary 未返回可识别的搜索页".into()));
    }
    let mut out = vec![];
    for video in d.select(&sel("div.video")) {
        let Some(a) = video.select(&sel("a[href]")).next() else {
            continue;
        };
        let id = video
            .select(&sel(".id"))
            .next()
            .map(|e| e.text().collect::<String>())
            .or_else(|| {
                a.value()
                    .attr("title")
                    .and_then(|s| s.split_whitespace().next())
                    .map(String::from)
            });
        if id.is_some_and(|v| equal_code(&v, code)) {
            if let Some(u) = a.value().attr("href").and_then(|s| same_origin(base, s)) {
                if !out.contains(&u) {
                    out.push(u)
                }
            }
        }
    }
    Ok(out)
}
pub fn parse_detail(html: &str, base: &str, code: &Code) -> Result<PartialMetadata, SourceError> {
    parse_page(html, base, Some(code))
}

/// `code` 为 None 时不核对身份(用户指定的详情页,只提示不拒绝)。
fn parse_page(html: &str, base: &str, code: Option<&Code>) -> Result<PartialMetadata, SourceError> {
    validate_body(html)?;
    let d = Html::parse_document(html);
    let id = text(&d, "#video_id .text")
        .ok_or_else(|| SourceError::Parse("JavLibrary 详情缺少番号；站点结构可能已变化".into()))?;
    if code.is_some_and(|code| !equal_code(&id, code)) {
        return Err(SourceError::NotFound);
    }
    let full = text(&d, "h3.post-title a, #video_title a")
        .ok_or_else(|| SourceError::Parse("JavLibrary 详情缺少标题".into()))?;
    // 标题以番号开头(与 JavBus 一致,去掉番号前缀)
    let title = match full.split_once(char::is_whitespace) {
        Some((head, rest)) if head.eq_ignore_ascii_case(&id) && !rest.trim().is_empty() => rest.trim().to_string(),
        _ => full,
    };
    let score = text(&d, "span.score")
        .and_then(|s| {
            regex::Regex::new(r"\(([\d.]+)\)")
                .unwrap()
                .captures(&s)
                .and_then(|c| c[1].parse::<f32>().ok())
        })
        .filter(|v| v.is_finite() && (0.0..=10.0).contains(v))
        .map(|v| v * 10.0);
    Ok(PartialMetadata {
        title: Some(title),
        release_date: text(&d, "#video_date .text"),
        runtime_min: text(&d, "#video_length .text").and_then(|s| {
            regex::Regex::new(r"\d+")
                .unwrap()
                .find(&s)
                .and_then(|v| v.as_str().parse().ok())
        }),
        studio: text(&d, "#video_maker .text a"),
        publisher: text(&d, "#video_label .text a"),
        directors: links(&d, "#video_director .text a"),
        actors: links(&d, ".cast .star a")
            .into_iter()
            .map(|n| Actor::new(n, Gender::Female))
            .collect(),
        tags: links(&d, "#video_genres .genre a"),
        thumb_urls: attrs(&d, "#video_jacket_img", "src")
            .into_iter()
            .filter_map(|s| absolute(base, &s))
            .collect(),
        score,
        source_url: Some(base.into()),
        external_id: reqwest::Url::parse(base).ok().and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "v")
                .map(|(_, v)| v.to_string())
        }),
        ..Default::default()
    })
}
#[async_trait]
impl Source for JavLibrary {
    fn id(&self) -> SourceId {
        "javlibrary"
    }
    fn supports(&self, c: &Code) -> bool {
        c.content_type == ContentType::Censored
    }
    async fn check_connectivity(&self, h: &Http) -> crate::connectivity::ConnectivityOutcome {
        let u = format!("{}/cn/", self.base.trim_end_matches('/'));
        crate::connectivity::probe_html(
            h,
            &u,
            &self.cookie,
            "",
            &["video_id", "videothumblist", "javlibrary"],
        )
        .await
    }
    async fn fetch_detail(&self, url: &str, _code: &Code, h: &Http) -> Option<Result<(PartialMetadata, String), SourceError>> {
        Some(async {
            h.seed_cookies(url, &super::javdb::clean_cookie(&self.cookie));
            let p = h.get_text(url).await.map_err(|e| SourceError::Network(e.to_string()))?;
            check(&p)?;
            let mut m = parse_page(&p.body, &p.url, None)?;
            m.source_url = Some(p.url);
            Ok((m, p.body))
        }.await)
    }
    async fn fetch(&self, c: &Code, h: &Http) -> Result<PartialMetadata, SourceError> {
        if !self.supports(c) {
            return Err(SourceError::NotFound);
        }
        let mut url = reqwest::Url::parse(&format!(
            "{}/cn/vl_searchbyid.php",
            self.base.trim_end_matches('/')
        ))
        .map_err(|e| SourceError::Parse(e.to_string()))?;
        url.query_pairs_mut()
            .append_pair("keyword", &c.search_key());
        h.seed_cookies(url.as_str(), &super::javdb::clean_cookie(&self.cookie));
        let p = h
            .get_text(url.as_str())
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        if Html::parse_document(&p.body)
            .select(&sel("#video_id"))
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
}
