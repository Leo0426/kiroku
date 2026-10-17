//! Jav321 数据源(数据来自 DMM,提供日文标题与简介,补足 JavDB 缺少的 plot)。
//! 搜索为表单 POST(`sn=<番号>`),命中时 301 到详情页;未命中返回 200 的列表页。

use crate::{code::{Code, CodeKind}, metadata::{Actor, Gender, PartialMetadata}, net::Http, source::{Source, SourceError, SourceId}};
use async_trait::async_trait;
use scraper::{ElementRef, Html, Selector};

pub struct Jav321 {
    pub base: String,
}

impl Default for Jav321 {
    fn default() -> Self {
        Self { base: "https://www.jav321.com".into() }
    }
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 详情页 → 部分元数据;非详情页(未命中的列表页)返回 NotFound。
pub fn parse_detail(html: &str) -> Result<PartialMetadata, SourceError> {
    let doc = Html::parse_document(html);
    let panel = doc.select(&sel("div.panel-info")).next().ok_or(SourceError::NotFound)?;
    let h3 = panel.select(&sel("div.panel-heading h3")).next().ok_or(SourceError::NotFound)?;
    // 只取 h3 的直接文本(<small> 内是番号和演员名)
    let title: String = squash(&h3.children().filter_map(|n| n.value().as_text().map(|t| t.to_string())).collect::<String>());
    if title.is_empty() {
        return Err(SourceError::Parse("title empty".into()));
    }

    let mut m = PartialMetadata { title: Some(title.clone()), original_title: Some(title), ..Default::default() };

    // 字段区:`<b>标签</b>: 值…<br>` 重复
    if let Some(info) = panel.select(&sel("div.col-md-9")).next() {
        let mut label = String::new();
        let (mut txt, mut links) = (String::new(), Vec::<String>::new());
        let flush = |label: &str, txt: &str, links: &mut Vec<String>, m: &mut PartialMetadata| {
            let v = squash(txt.trim_start_matches(|c: char| c == ':' || c.is_whitespace()));
            match label {
                // 出演栏默认女优(与 amane 一致)
                "出演者" => m.actors = links.drain(..).map(|n| Actor::new(n, Gender::Female)).collect(),
                "ジャンル" => m.tags = std::mem::take(links),
                "メーカー" => m.studio = links.first().cloned().or(Some(v).filter(|v| !v.is_empty())),
                "レーベル" => m.publisher = links.first().cloned().or(Some(v).filter(|v| !v.is_empty())),
                "平均評価" => m.score = v.parse::<f32>().ok().map(|x| x * 20.0), // 5 分制 → 0–100
                "シリーズ" => m.series = links.first().cloned().or(Some(v).filter(|v| !v.is_empty())),
                "配信開始日" | "発売日" => m.release_date = Some(v).filter(|v| !v.is_empty()),
                "収録時間" => m.runtime_min = v.split_whitespace().next().and_then(|n| n.parse().ok()),
                _ => {}
            }
            links.clear();
        };
        for node in info.children() {
            if let Some(e) = ElementRef::wrap(node) {
                match e.value().name() {
                    "b" => {
                        flush(&label, &txt, &mut links, &mut m);
                        label = squash(&e.text().collect::<String>());
                        txt.clear();
                    }
                    "br" => {
                        flush(&label, &txt, &mut links, &mut m);
                        label.clear();
                        txt.clear();
                    }
                    "a" => links.push(squash(&e.text().collect::<String>())),
                    _ => {}
                }
            } else if let Some(t) = node.value().as_text() {
                txt.push_str(t);
            }
        }
        flush(&label, &txt, &mut links, &mut m);
    }

    // 简介:面板内不含子元素的 col-md-12 文本块
    m.plot = panel
        .select(&sel("div.col-md-12"))
        .filter(|d| d.children().all(|c| c.value().is_text()))
        .map(|d| squash(&d.text().collect::<String>()))
        .find(|t| t.chars().count() > 20);

    // 封面:大图 pl.jpg(横版,由 organize 裁 poster);样张:/snapshot/ 下的 jp-N.jpg
    let imgs: Vec<String> = doc
        .select(&sel("a[href^='/snapshot/'] img"))
        .filter_map(|i| i.value().attr("src").map(String::from))
        .collect();
    m.thumb_urls = imgs.iter().filter(|u| u.ends_with("pl.jpg")).take(1).cloned().collect();
    // 左栏小图 ps.jpg 是竖版封面
    m.poster_urls = panel
        .select(&sel("div.col-md-3 img.img-responsive"))
        .filter_map(|i| i.value().attr("src"))
        .filter(|u| u.ends_with("ps.jpg"))
        .take(1)
        .map(String::from)
        .collect();
    m.extrafanart = imgs.into_iter().filter(|u| !u.ends_with("pl.jpg")).collect();
    // DMM cid(如 ssis00001)取自图片路径
    m.external_id = m.thumb_urls.first().and_then(|u| u.split("/video/").nth(1)).and_then(|r| r.split('/').next()).map(String::from);
    Ok(m)
}

#[async_trait]
impl Source for Jav321 {
    async fn check_connectivity(&self,http:&Http)->crate::connectivity::ConnectivityOutcome {crate::connectivity::probe_html(http,&self.base,"","",&["jav321", "action=\"/search\""]).await}
    fn id(&self) -> SourceId {
        "jav321"
    }

    fn supports(&self, code: &Code) -> bool {
        code.kind == CodeKind::Standard
    }

    async fn fetch(&self, code: &Code, http: &Http) -> Result<PartialMetadata, SourceError> {
        let page = http
            .post_form_text(&format!("{}/search", self.base), &[("sn", &code.search_key())])
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        match page.status {
            200 => {
                let mut m = parse_detail(&page.body)?;
                m.source_url = Some(page.url);
                Ok(m)
            }
            403 | 429 | 503 => Err(SourceError::Blocked(format!("status {}", page.status))),
            s => Err(SourceError::Network(format!("status {s}"))),
        }
    }

    async fn fetch_detail(&self, url: &str, _code: &Code, http: &Http) -> Option<Result<(PartialMetadata, String), SourceError>> {
        Some(async {
            let page = http.get_text(url).await.map_err(|e| SourceError::Network(e.to_string()))?;
            match page.status {
                200 => { let mut m = parse_detail(&page.body)?; m.source_url = Some(page.url); Ok((m, page.body)) }
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

    const DETAIL: &str = include_str!("../../tests/fixtures/jav321/detail_ABC-001.html");
    const NONE: &str = include_str!("../../tests/fixtures/jav321/search_none.html");

    #[test]
    fn detail_fields() {
        let m = parse_detail(DETAIL).unwrap();
        assert!(m.title.as_deref().unwrap().starts_with("Sample Title 001"));
        assert!(!m.title.as_deref().unwrap().contains("abc-001"), "不应混入 <small> 内容");
        assert_eq!(m.actors, vec![Actor::new("Performer A", Gender::Female), Actor::new("Performer B", Gender::Female)]);
        assert_eq!(m.studio.as_deref(), Some("Studio One"));
        assert_eq!(m.external_id.as_deref(), Some("abc00001"));
        assert!((m.score.unwrap() - 90.0).abs() < 0.01);
        assert!(m.poster_urls[0].ends_with("abc00001ps.jpg"));
        assert_eq!(m.release_date.as_deref(), Some("2021-02-19"));
        assert_eq!(m.runtime_min, Some(147));
        assert!(m.plot.as_deref().unwrap().starts_with("Synthetic plot"));
        assert!(m.thumb_urls[0].ends_with("abc00001pl.jpg"));
        assert!(m.extrafanart.len() >= 2 && m.extrafanart[0].contains("jp-1"));
    }

    #[test]
    fn miss_is_not_found() {
        assert!(matches!(parse_detail(NONE), Err(SourceError::NotFound)));
    }
}
