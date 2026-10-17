//! JavBus 数据源:番号直达详情页 `/{番号}`,不存在返回 404。
//! 需要年龄验证 Cookie;图片需同源 Referer(由 `Http` 统一补)。不提供简介。

use crate::{
    code::{Code, CodeKind},
    metadata::{Actor, Gender, PartialMetadata},
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use scraper::{Html, Selector};

pub struct JavBus {
    pub base: String,
}

impl Default for JavBus {
    fn default() -> Self {
        Self { base: "https://www.javbus.com".into() }
    }
}

const COOKIE: &str = "existmag=all; age=verified; dv=1";

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn absolute(base: &str, href: &str) -> String {
    if href.starts_with("http") {
        href.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), href.trim_start_matches('/'))
    }
}

/// 详情页 → 部分元数据。`base` 用于把相对图片路径补全。
pub fn parse_detail(html: &str, base: &str) -> Result<PartialMetadata, SourceError> {
    let doc = Html::parse_document(html);
    let h3 = doc.select(&sel("div.container h3")).next().ok_or(SourceError::NotFound)?;
    let full = squash(&h3.text().collect::<String>());
    // h3 形如「SSIS-001 标题…」,去掉开头的番号
    let title = full.split_once(' ').map(|(_, t)| t.to_string()).filter(|t| !t.is_empty());
    let Some(title) = title else { return Err(SourceError::Parse("title not found".into())) };

    let mut m = PartialMetadata { title: Some(title.clone()), original_title: Some(title), ..Default::default() };

    for p in doc.select(&sel("div.info > p")) {
        let Some(label) = p.select(&sel("span.header")).next() else { continue };
        let name = squash(&label.text().collect::<String>());
        let name = name.trim_end_matches([':', ':']).trim();
        let link_text = || p.select(&sel("a")).next().map(|a| squash(&a.text().collect::<String>()));
        let plain = || squash(&p.text().collect::<String>()).replacen(&squash(&label.text().collect::<String>()), "", 1).trim().to_string();
        match name {
            "發行日期" | "发行日期" => m.release_date = Some(plain()).filter(|s| !s.is_empty()),
            "長度" | "长度" => m.runtime_min = plain().chars().filter(char::is_ascii_digit).collect::<String>().parse().ok(),
            "導演" | "导演" => m.directors = link_text().into_iter().collect(),
            "製作商" | "制作商" => m.studio = link_text(),
            "發行商" | "发行商" => m.publisher = link_text(),
            "系列" => m.series = link_text(),
            _ => {}
        }
    }

    m.tags = doc
        .select(&sel("span.genre label a"))
        .map(|a| squash(&a.text().collect::<String>()))
        .filter(|t| !t.is_empty())
        .collect();
    // 演员头像区;JavBus 的演员栏是女优名单
    let (name_sel, img_sel) = (sel("div.star-name a"), sel("img"));
    m.actors = doc
        .select(&sel("div.star-box li"))
        .filter_map(|li| {
            let name = squash(&li.select(&name_sel).next()?.text().collect::<String>());
            let thumb = li.select(&img_sel).next().and_then(|i| i.value().attr("src")).filter(|u| Actor::real_thumb(u)).map(|u| absolute(base, u));
            Some(Actor { thumb, ..Actor::new(name, Gender::Female) })
        })
        .filter(|a| !a.name.is_empty())
        .collect();

    // 大图是横版封面(封底|封面);剧照在 DMM 图床
    m.thumb_urls = doc.select(&sel("a.bigImage")).filter_map(|a| a.value().attr("href")).map(|h| absolute(base, h)).take(1).collect();
    m.extrafanart = doc.select(&sel("a.sample-box")).filter_map(|a| a.value().attr("href")).map(|h| absolute(base, h)).collect();
    Ok(m)
}

#[async_trait]
impl Source for JavBus {
    async fn check_connectivity(&self,http:&Http)->crate::connectivity::ConnectivityOutcome {crate::connectivity::probe_html(http,&self.base,"","existmag=all",&["javbus", "movie-box"]).await}
    fn id(&self) -> SourceId {
        "javbus"
    }

    fn supports(&self, code: &Code) -> bool {
        // FC2 不在 JavBus;有码、无码(日期型也有 /{番号} 直达)、素人都可尝试
        code.kind != CodeKind::Fc2
    }

    async fn fetch(&self, code: &Code, http: &Http) -> Result<PartialMetadata, SourceError> {
        let url = format!("{}/{}", self.base.trim_end_matches('/'), code.search_key());
        let page = http.get_text_with(&url, &[("Cookie", COOKIE)]).await.map_err(|e| SourceError::Network(e.to_string()))?;
        match page.status {
            200 => {
                // 年龄验证页会 200 重定向;没有 h3 标题就当作未命中/被拦
                if page.url.contains("driver-verify") {
                    return Err(SourceError::Blocked("age verification".into()));
                }
                let mut m = parse_detail(&page.body, &self.base)?;
                m.source_url = Some(page.url);
                m.external_id = Some(code.search_key());
                Ok(m)
            }
            404 => Err(SourceError::NotFound),
            403 | 429 | 503 => Err(SourceError::Blocked(format!("status {}", page.status))),
            s => Err(SourceError::Network(format!("status {s}"))),
        }
    }

    async fn fetch_detail(&self, url: &str, _code: &Code, http: &Http) -> Option<Result<(PartialMetadata, String), SourceError>> {
        Some(async {
            let page = http.get_text_with(url, &[("Cookie", COOKIE)]).await.map_err(|e| SourceError::Network(e.to_string()))?;
            match page.status {
                200 if page.url.contains("driver-verify") => Err(SourceError::Blocked("age verification".into())),
                200 => { let mut m = parse_detail(&page.body, &self.base)?; m.source_url = Some(page.url); Ok((m, page.body)) }
                404 => Err(SourceError::NotFound),
                403 | 429 | 503 => Err(SourceError::Blocked(format!("status {}", page.status))),
                s => Err(SourceError::Network(format!("status {s}"))),
            }
        }.await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DETAIL: &str = include_str!("../../tests/fixtures/javbus/detail_ABC-001.html");
    const MISSING: &str = include_str!("../../tests/fixtures/javbus/missing.html");

    #[test]
    fn detail_fields() {
        let m = parse_detail(DETAIL, "https://www.javbus.com").unwrap();
        assert!(m.title.as_deref().unwrap().starts_with("Sample Title 001"));
        assert!(!m.title.as_deref().unwrap().starts_with("ABC-001"), "不应带番号前缀");
        assert_eq!(m.release_date.as_deref(), Some("2021-02-18"));
        assert_eq!(m.runtime_min, Some(150));
        assert_eq!(m.directors, vec!["Director A"]);
        assert_eq!(m.studio.as_deref(), Some("Studio One"));
        assert_eq!(m.publisher.as_deref(), Some("Label One"));
        assert_eq!(m.actors, vec![
            Actor::new("Performer A", Gender::Female).with_thumb("https://www.javbus.com/pics/actress/p1_a.jpg"),
            Actor::new("Performer B", Gender::Female).with_thumb("https://www.javbus.com/pics/actress/p2_a.jpg"),
        ]);
        assert!(m.tags.contains(&"劇情".to_string()) && m.tags.len() >= 6, "{:?}", m.tags);
        assert_eq!(m.thumb_urls, vec!["https://www.javbus.com/pics/cover/aaa1_b.jpg"]);
        assert!(m.extrafanart.len() >= 3 && m.extrafanart[0].contains("abc00001-1.jpg"));
    }

    #[test]
    fn placeholder_avatars_are_dropped() {
        let html = DETAIL.replace("/pics/actress/p1_a.jpg", "/pics/actress/nowprinting.gif");
        let m = parse_detail(&html, "https://www.javbus.com").unwrap();
        assert_eq!(m.actors[0].thumb, None);
        assert!(m.actors[1].thumb.is_some());
    }

    #[test]
    fn missing_page_is_not_found() {
        assert!(parse_detail(MISSING, "https://www.javbus.com").is_err());
    }
}
