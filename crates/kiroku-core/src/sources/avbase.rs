//! AVBase(www.avbase.net)。页面是 Next.js,作品数据在 `__NEXT_DATA__` 的 pageProps 里,
//! 不解析 HTML 结构。普通请求会被 Cloudflare 挑战拦住,需要经用户的求解器访问(ADR-0007)。
//! fixture 为 2026-10-15 经 FlareSolverr 取得的真实页面。
use super::common::*;
use crate::{
    code::{Code, ContentType},
    metadata::{Actor, Gender, PartialMetadata},
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use serde_json::Value;

pub struct AvBase {
    pub base: String,
}

impl Default for AvBase {
    fn default() -> Self {
        Self { base: "https://www.avbase.net".into() }
    }
}

fn page_props(html: &str) -> Result<Value, SourceError> {
    body_error(html)?;
    let start = html.find("<script id=\"__NEXT_DATA__\"")
        .and_then(|i| html[i..].find('>').map(|j| i + j + 1))
        .ok_or_else(|| SourceError::Parse("AVBase 页面缺少 __NEXT_DATA__;站点结构可能已变化".into()))?;
    let end = html[start..].find("</script>").map(|j| start + j)
        .ok_or_else(|| SourceError::Parse("AVBase __NEXT_DATA__ 不完整".into()))?;
    let v: Value = serde_json::from_str(&html[start..end]).map_err(|e| SourceError::Parse(format!("AVBase 数据不是 JSON: {e}")))?;
    Ok(v["props"]["pageProps"].clone())
}

/// 番号比较:忽略大小写和分隔符,数字部分忽略前导零;素人番号的数字前缀(300MIUM)也算同一部。
fn same_code(raw: &str, code: &Code) -> bool {
    let norm = |s: &str| -> Option<(String, u64)> {
        let s: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
        let digits = s.len() - s.chars().rev().take_while(|c| c.is_ascii_digit()).count();
        let (head, num) = s.split_at(digits);
        Some((head.to_string(), num.parse().ok()?))
    };
    let (Some((a, n)), Some((b, m))) = (norm(raw), norm(&code.id)) else { return false };
    let strip = |s: &str| s.trim_start_matches(|c: char| c.is_ascii_digit()).to_string();
    n == m && !a.is_empty() && (a == b || strip(&a) == strip(&b))
}

fn matches(work: &Value, code: &Code) -> bool {
    work["work_id"].as_str().is_some_and(|w| same_code(w, code))
        || work["products"].as_array().into_iter().flatten()
            .any(|p| p["product_id"].as_str().is_some_and(|w| same_code(w, code)))
}

fn work_path(work: &Value) -> Option<String> {
    let id = work["work_id"].as_str()?;
    Some(match work["prefix"].as_str().filter(|p| !p.is_empty()) {
        Some(prefix) => format!("/works/{prefix}:{id}"),
        None => format!("/works/{id}"),
    })
}

/// 搜索页:返回与番号一致的作品详情地址。没有厂牌前缀的作品排在前面;
/// 同番号不同作品(如 SSIS-001 与 secondface:SSIS-001)都返回,由详情的身份核对和跨站日期核对决定。
pub fn parse_search(html: &str, base: &str, code: &Code) -> Result<Vec<String>, SourceError> {
    let props = page_props(html)?;
    let works = props["works"].as_array().ok_or_else(|| SourceError::Parse("AVBase 搜索页缺少 works".into()))?;
    let mut hits: Vec<&Value> = works.iter().filter(|w| matches(w, code)).collect();
    hits.sort_by_key(|w| w["prefix"].as_str().is_some_and(|p| !p.is_empty()));
    Ok(hits.into_iter().filter_map(work_path).map(|p| format!("{}{p}", base.trim_end_matches('/'))).collect())
}

/// "Thu Feb 18 2021 09:00:00 GMT+0900 (Japan Standard Time)" → 2021-02-18(站点已按日本时间给出)
fn date(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let month = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .iter().position(|m| Some(m) == parts.get(1))? + 1;
    let day: u32 = parts.get(2)?.parse().ok()?;
    let year: u32 = parts.get(3)?.parse().ok()?;
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

fn name(v: &Value) -> Option<String> {
    v["name"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn push_unique(list: &mut Vec<String>, value: Option<String>) {
    if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
        if !list.contains(&v) { list.push(v) }
    }
}

/// 作品详情页。页面里的作品番号与要找的不一致时返回 NotFound。
pub fn parse_detail(html: &str, code: &Code) -> Result<PartialMetadata, SourceError> {
    parse_work(html, Some(code))
}

/// `code` 为 None 时不核对身份(用户指定的详情页,与其他来源一致只提示不拒绝)。
fn parse_work(html: &str, code: Option<&Code>) -> Result<PartialMetadata, SourceError> {
    let props = page_props(html)?;
    let work = &props["work"];
    if work.is_null() || code.is_some_and(|c| !matches(work, c)) {
        return Err(SourceError::NotFound);
    }
    let title = work["title"].as_str().map(str::trim).filter(|t| !t.is_empty())
        .ok_or_else(|| SourceError::Parse("AVBase 作品缺少标题".into()))?.to_string();
    let products: Vec<&Value> = work["products"].as_array().map(|p| p.iter().collect()).unwrap_or_default();
    // 简介、时长、导演取第一个有简介的商品(通常是 FANZA 配信),没有就取第一个
    let main = products.iter().find(|p| nonempty(&p["iteminfo"], "description").is_some()).or(products.first());
    let info = main.map(|p| &p["iteminfo"]);
    let mut m = PartialMetadata {
        title: Some(title.clone()),
        original_title: Some(title),
        plot: info.and_then(|i| nonempty(i, "description")),
        release_date: work["min_date"].as_str().and_then(date),
        // FANZA 写 "147",MGStage 写 "69分"
        runtime_min: info.and_then(|i| i["volume"].as_str()).and_then(|v| v.trim().split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()).filter(|v| *v > 0),
        studio: products.iter().find_map(|p| name(&p["maker"])),
        publisher: products.iter().find_map(|p| name(&p["label"])),
        series: products.iter().find_map(|p| name(&p["series"])),
        external_id: work["work_id"].as_str().map(String::from),
        ..Default::default()
    };
    for cast in work["casts"].as_array().into_iter().flatten() {
        if let Some(n) = name(&cast["actor"]) {
            if !m.actors.iter().any(|a| a.name == n) {
                let thumb = nonempty(&cast["actor"], "image_url").filter(|u| Actor::real_thumb(u));
                m.actors.push(Actor { thumb, ..Actor::new(n, Gender::Female) })
            }
        }
    }
    for g in work["genres"].as_array().into_iter().flatten().chain(work["tags"].as_array().into_iter().flatten()) {
        push_unique(&mut m.tags, name(g));
    }
    push_unique(&mut m.directors, info.and_then(|i| nonempty(i, "director")));
    for p in &products {
        push_unique(&mut m.thumb_urls, nonempty(p, "image_url"));
        push_unique(&mut m.trailer_urls, nonempty(p, "sample_movie_url"));
    }
    if let Some(p) = main {
        for s in p["sample_image_urls"].as_array().into_iter().flatten() {
            push_unique(&mut m.extrafanart, nonempty(s, "l").or_else(|| nonempty(s, "s")));
        }
    }
    Ok(m)
}

#[async_trait]
impl Source for AvBase {
    fn id(&self) -> SourceId {
        "avbase"
    }
    fn supports(&self, c: &Code) -> bool {
        matches!(c.content_type, ContentType::Censored | ContentType::Amateur)
    }
    async fn check_connectivity(&self, h: &Http) -> crate::connectivity::ConnectivityOutcome {
        let u = format!("{}/works?q=SSIS-001", self.base.trim_end_matches('/'));
        crate::connectivity::probe_html(h, &u, "", "", &["__next_data__"]).await
    }
    async fn fetch(&self, c: &Code, h: &Http) -> Result<PartialMetadata, SourceError> {
        if !self.supports(c) {
            return Err(SourceError::NotFound);
        }
        let mut url = reqwest::Url::parse(&format!("{}/works", self.base.trim_end_matches('/')))
            .map_err(|e| SourceError::Parse(e.to_string()))?;
        url.query_pairs_mut().append_pair("q", &c.search_key());
        let p = h.get_text(url.as_str()).await.map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        for url in parse_search(&p.body, &self.base, c)? {
            let p = h.get_text(&url).await.map_err(|e| SourceError::Network(e.to_string()))?;
            check(&p)?;
            match parse_detail(&p.body, c) {
                Ok(mut m) => {
                    m.source_url = Some(p.url);
                    return Ok(m);
                }
                Err(SourceError::NotFound) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(SourceError::NotFound)
    }
    async fn fetch_detail(&self, url: &str, _code: &Code, http: &Http) -> Option<Result<(PartialMetadata, String), SourceError>> {
        Some(async {
            let p = http.get_text(url).await.map_err(|e| SourceError::Network(e.to_string()))?;
            check(&p)?;
            let mut m = parse_work(&p.body, None)?;
            m.source_url = Some(p.url);
            Ok((m, p.body))
        }.await)
    }
}
