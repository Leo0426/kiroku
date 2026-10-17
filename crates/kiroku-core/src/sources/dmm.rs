//! FANZA/DMM digital AV: public legacySearchPPV -> exact CID -> ppvContent.
//! Detail fields/search fallback derive from amane 3006c80 (GPL-3.0).
//! TV, rental, mono and monthly are intentionally outside this digital adapter.
use super::common::*;
use crate::{
    code::{Code, ContentType},
    metadata::{Actor, Gender, PartialMetadata},
    net::Http,
    source::{Source, SourceError, SourceId},
};
use async_trait::async_trait;
use serde_json::{json, Value};
pub const DEFAULT_API: &str = "https://api.video.dmm.co.jp/graphql";
pub struct Dmm {
    pub base: String,
    pub api_base: String,
    pub cookie: String,
}
fn number(code: &str) -> Option<(String, String)> {
    let re = regex::Regex::new(r"(?i)^([a-z]+)[-_]?(\d+)$").unwrap();
    let c = re.captures(code)?;
    let n = c[2].trim_start_matches('0');
    Some((
        c[1].to_lowercase(),
        if n.is_empty() { "0".into() } else { n.into() },
    ))
}
pub fn search_keys(code: &str) -> Vec<String> {
    let Some((p, n)) = number(code) else {
        return vec![];
    };
    let mut out = vec![format!("{p}{n:0>5}")];
    let plain = format!("{p}{n}");
    if out[0] != plain {
        out.push(plain)
    }
    out
}
pub fn cid_matches(cid: &str, code: &str) -> bool {
    let Some((p, n)) = number(code) else {
        return false;
    };
    let re = regex::Regex::new(&format!(
        r"(?i)^(?:(?:h|n)_\d+|\d+)?{}0*{}(?:so|sp|tk|hd|hhb|hib)?$",
        regex::escape(&p),
        regex::escape(&n)
    ))
    .unwrap();
    re.is_match(cid)
}
fn graphql(body: &str) -> Result<Value, SourceError> {
    let v = json(body)?;
    if v.get("errors")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Err(SourceError::Parse(
            "DMM GraphQL errors；公开 API 可能变更或请求被拒绝".into(),
        ));
    }
    let data = v
        .get("data")
        .filter(|d| d.is_object())
        .ok_or_else(|| SourceError::Parse("DMM GraphQL 缺少 data 对象".into()))?;
    Ok(data.clone())
}
pub fn parse_search(body: &str, code: &str) -> Result<Vec<String>, SourceError> {
    let data = graphql(body)?;
    let rows = data
        .pointer("/legacySearchPPV/result/contents")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Parse("DMM 搜索缺少 contents 数组".into()))?;
    let mut ids = Vec::new();
    for row in rows {
        let id =
            nonempty(row, "id").ok_or_else(|| SourceError::Parse("DMM 搜索项缺少 CID".into()))?;
        if cid_matches(&id, code) && !ids.contains(&id) {
            ids.push(id)
        }
    }
    Ok(ids)
}
pub fn parse_detail(body: &str, cid: &str) -> Result<PartialMetadata, SourceError> {
    let data = graphql(body)?;
    let v = data
        .get("ppvContent")
        .ok_or_else(|| SourceError::Parse("DMM 缺少 ppvContent".into()))?;
    if v.is_null() {
        return Err(SourceError::NotFound);
    }
    if !v.is_object() {
        return Err(SourceError::Parse("DMM ppvContent 不是对象".into()));
    }
    let title =
        nonempty(v, "title").ok_or_else(|| SourceError::Parse("DMM 详情缺少标题".into()))?;
    // Query returns no id: validate the authoritative CID present in package image paths,
    // when provided. Request CID is still retained for image-less legitimate details.
    for key in ["largeUrl", "mediumUrl"] {
        if let Some(u) = nonempty(&v["packageImage"], key) {
            if let Some(image_cid) = u.split("/video/").nth(1).and_then(|s| s.split('/').next()) {
                if image_cid != cid {
                    return Err(SourceError::Parse("DMM 详情封面 CID 不匹配".into()));
                }
            }
        }
    }
    let mut m = PartialMetadata {
        title: Some(title.clone()),
        original_title: Some(title),
        plot: nonempty(v, "description"),
        release_date: nonempty(v, "deliveryStartDate")
            .map(|s| s.split('T').next().unwrap_or(&s).into()),
        runtime_min: v["duration"]
            .as_u64()
            .and_then(|n| u32::try_from(n / 60).ok()),
        studio: nonempty(&v["maker"], "name"),
        publisher: nonempty(&v["label"], "name"),
        series: nonempty(&v["series"], "name"),
        external_id: Some(cid.into()),
        score: data["reviewSummary"]["average"]
            .as_f64()
            .filter(|n| n.is_finite() && (0.0..=5.0).contains(n))
            .map(|n| (n * 20.) as f32),
        ..Default::default()
    };
    for (key, kind) in [("actresses", 0), ("directors", 1), ("genres", 2)] {
        if let Some(rows) = v[key].as_array() {
            for row in rows {
                if let Some(n) = nonempty(row, "name") {
                    match kind {
                        0 => m.actors.push(Actor::new(n, Gender::Female)),
                        1 => m.directors.push(n),
                        _ => m.tags.push(n),
                    }
                }
            }
        }
    }
    m.thumb_urls = nonempty(&v["packageImage"], "largeUrl")
        .into_iter()
        .collect();
    m.poster_urls = nonempty(&v["packageImage"], "mediumUrl")
        .into_iter()
        .collect();
    m.trailer_urls = nonempty(&v["sample2DMovie"], "highestMovieUrl")
        .into_iter()
        .collect();
    m.extrafanart = v["sampleImages"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|e| nonempty(e, "largeImageUrl"))
                .collect()
        })
        .unwrap_or_default();
    Ok(m)
}
pub fn parse_html_search(body: &str, code: &str, base: &str) -> Result<Vec<String>, SourceError> {
    body_error(body)?;
    let unescaped = body.replace("\\/", "/").replace("\\\"", "\"");
    let re = regex::Regex::new(r#""detailUrl"\s*:\s*"([^"]+)""#).unwrap();
    if !re.is_match(&unescaped) {
        return Err(SourceError::Parse("DMM 网页未返回可解析的数字影片搜索目录；公开 API 搜索有目录缺口，网页回退通常需要日本出口".into()));
    }
    let mut out = Vec::new();
    for c in re.captures_iter(&unescaped) {
        if let Some(u) = absolute(base, &c[1]) {
            let Ok(url) = reqwest::Url::parse(&u) else {
                continue;
            };
            if !url.path().contains("/digital/") {
                continue;
            }
            for (k, v) in url.query_pairs() {
                if (k == "cid" || k == "id")
                    && cid_matches(&v, code)
                    && !out.contains(&v.to_string())
                {
                    out.push(v.into_owned())
                }
            } // Legacy path cid=.../ uses no query string.
            if let Some(cid) = regex::Regex::new(r"(?:cid|id)=([^/&?#]+)")
                .unwrap()
                .captures(url.path())
                .map(|c| c[1].to_string())
            {
                if cid_matches(&cid, code) && !out.contains(&cid) {
                    out.push(cid)
                }
            }
        }
    }
    Ok(out)
}
impl Dmm {
    async fn detail(&self, cid: &str, h: &Http) -> Result<PartialMetadata, SourceError> {
        let body = json!({"operationName":"FetchDigitalContent","variables":{"id":cid},"query":include_str!("dmm_digital.graphql")});
        let p = h
            .post_json_text(&self.api_base, &body, &[])
            .await
            .map_err(|e| SourceError::Network(e.to_string()))?;
        check(&p)?;
        let mut m = parse_detail(&p.body, cid)?;
        m.source_url = Some(format!(
            "{}/digital/videoa/-/detail/=/cid={cid}/",
            self.base.trim_end_matches('/')
        ));
        Ok(m)
    }
}
#[async_trait]
impl Source for Dmm {
    async fn check_connectivity(&self,http:&Http)->crate::connectivity::ConnectivityOutcome {crate::connectivity::probe_dmm(http,&self.api_base,&self.cookie).await}
    fn id(&self) -> SourceId {
        "dmm"
    }
    fn supports(&self, c: &Code) -> bool {
        c.content_type == ContentType::Censored && number(&c.id).is_some()
    }
    async fn fetch(&self, c: &Code, h: &Http) -> Result<PartialMetadata, SourceError> {
        h.seed_cookies(&self.api_base, &self.cookie);
        let keys = search_keys(&c.id);
        if keys.is_empty() {
            return Err(SourceError::NotFound);
        }
        let mut tried = Vec::new();
        let mut last_error = None;
        for key in &keys {
            let q=format!("query PublicNumberSearch {{ legacySearchPPV(limit: 5, offset: 0, sort: SALES_RANK_SCORE, floor: AV, queryWord: {}) {{ result {{ contents {{ id title packageImage {{ largeUrl }} }} pageInfo {{ totalCount }} }} }} }}",serde_json::to_string(key).unwrap());
            let p = h
                .post_json_text(
                    &self.api_base,
                    &json!({"operationName":"PublicNumberSearch","query":q}),
                    &[],
                )
                .await
                .map_err(|e| SourceError::Network(e.to_string()));
            match p.and_then(|p| {
                check(&p)?;
                parse_search(&p.body, &c.id)
            }) {
                Ok(ids) => {
                    for id in ids {
                        if tried.contains(&id) {
                            continue;
                        }
                        tried.push(id.clone());
                        match self.detail(&id, h).await {
                            Ok(m) => return Ok(m),
                            Err(SourceError::NotFound) => {}
                            Err(e) => last_error = Some(e),
                        }
                    }
                }
                Err(e) => last_error = Some(e),
            }
        }
        // Search index gaps are real (MIDV-124): try precise bare CID variants as well.
        for cid in &keys {
            if tried.contains(cid) {
                continue;
            }
            tried.push(cid.clone());
            match self.detail(cid, h).await {
                Ok(m) => return Ok(m),
                Err(SourceError::NotFound) => {}
                Err(e) => last_error = Some(e),
            }
        }
        let base = self.base.trim_end_matches('/');
        h.seed_cookies(base, "age_check_done=1");
        h.seed_cookies(base, &self.cookie);
        for key in &keys {
            let url = format!("{base}/search/=/searchstr={key}/sort=ranking/");
            let p = h
                .get_text(&url)
                .await
                .map_err(|e| SourceError::Network(e.to_string()));
            match p.and_then(|p| {
                check(&p)?;
                parse_html_search(&p.body, &c.id, base)
            }) {
                Ok(ids) => {
                    for cid in ids {
                        if tried.contains(&cid) {
                            continue;
                        }
                        tried.push(cid.clone());
                        match self.detail(&cid, h).await {
                            Ok(m) => return Ok(m),
                            Err(SourceError::NotFound) => {}
                            Err(e) => last_error = Some(e),
                        }
                    }
                }
                Err(e) => last_error = Some(e),
            }
        }
        Err(last_error.unwrap_or(SourceError::NotFound))
    }
}
