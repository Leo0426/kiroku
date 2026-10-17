//! Shared pure response/selector helpers. Provider rules derive from amane 3006c80 (GPL-3.0).
use crate::{net::Page, source::SourceError};
use scraper::{Html, Selector};
pub fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}
pub fn text(d: &Html, s: &str) -> Option<String> {
    d.select(&sel(s))
        .map(|e| {
            e.text()
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .find(|v| !v.is_empty())
}
pub fn attrs(d: &Html, s: &str, a: &str) -> Vec<String> {
    d.select(&sel(s))
        .filter_map(|e| e.value().attr(a))
        .filter(|v| !v.is_empty())
        .map(String::from)
        .collect()
}
pub fn absolute(base: &str, u: &str) -> Option<String> {
    reqwest::Url::parse(base)
        .ok()?
        .join(u)
        .ok()
        .map(|u| u.to_string())
}
pub fn fold(v: &str) -> String {
    v.to_lowercase().replace(['-', ' '], "")
}
pub fn body_error(body: &str) -> Result<(), SourceError> {
    let b = body.to_lowercase();
    if b.contains("not available in your region")
        || b.contains("not-available-in-your-region")
        || body.contains("お住まいの地域からはご利用になれません")
    {
        return Err(SourceError::Blocked(
            "地区限制；DMM 网页搜索需要日本出口，可在设置中配置代理".into(),
        ));
    }
    if b.contains("request blocked")
        || b.contains("_cf_chl_opt")
        || b.contains("just a moment...")
        || b.contains("access denied")
    {
        return Err(SourceError::Blocked(
            "站点拦截请求；请检查出口与请求间隔".into(),
        ));
    }
    Ok(())
}
fn authentication_required() -> SourceError {
    SourceError::NeedLogin(
        "HTTP/API 401：站点拒绝认证；请在数据源设置填写或更新 Cookie，并确认该账号有访问权限"
            .into(),
    )
}
pub fn check(p: &Page) -> Result<(), SourceError> {
    body_error(&p.body)?;
    if p.url.contains("/login") || p.url.contains("/signin") {
        return Err(SourceError::NeedLogin(
            "站点要求登录；请在数据源设置填写有效 Cookie".into(),
        ));
    }
    match p.status {
        200..=299 => Ok(()),
        401 => Err(authentication_required()),
        404 => Err(SourceError::NotFound),
        403 | 429 | 503 => Err(SourceError::Blocked(format!(
            "HTTP {}；检查出口与请求间隔",
            p.status
        ))),
        s => Err(SourceError::Network(format!(
            "HTTP {s}{}",
            if s == 526 {
                "；站点上游 TLS 异常（非验证码）"
            } else {
                ""
            }
        ))),
    }
}
pub fn nonempty(v: &serde_json::Value, k: &str) -> Option<String> {
    v.get(k)?
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(String::from)
}
pub fn fc2_id(code: &str) -> Result<String, SourceError> {
    let re = regex::Regex::new(r"(?i)^FC2[-_]?(?:PPV[-_]?)?(\d+)$").unwrap();
    re.captures(code)
        .map(|c| c[1].to_string())
        .ok_or_else(|| SourceError::Parse("无效 FC2 番号".into()))
}
pub fn json(body: &str) -> Result<serde_json::Value, SourceError> {
    body_error(body)?;
    serde_json::from_str(body).map_err(|e| SourceError::Parse(format!("JSON: {e}")))
}
pub fn envelope(body: &str) -> Result<serde_json::Value, SourceError> {
    let v = json(body)?;
    if v["code"].as_u64() == Some(401) {
        return Err(authentication_required());
    }
    if v["code"].as_u64() != Some(200) {
        return Err(SourceError::Parse(format!(
            "API 响应失败 code={}",
            v["code"]
        )));
    }
    v.get("data")
        .cloned()
        .ok_or_else(|| SourceError::Parse("API 缺少 data".into()))
}

/// Numeric provider identity normalization only; Code.id remains untouched.
pub fn numeric_equal(a: &str, b: &str) -> bool {
    !a.is_empty()
        && !b.is_empty()
        && a.bytes().all(|c| c.is_ascii_digit())
        && b.bytes().all(|c| c.is_ascii_digit())
        && a.trim_start_matches('0') == b.trim_start_matches('0')
}

/// Provider identity normalization: optional separator, case and numeric padding.
/// Never alters the canonical Code stored on disk.
pub(crate) fn equal_code(raw: &str, code: &crate::code::Code) -> bool {
    let r = regex::Regex::new(r"(?i)^([a-z]+)[-_]?(\d+)$").unwrap();
    let Some(c) = r.captures(raw.trim()) else {
        return false;
    };
    crate::code::search_key(&format!("{}-{}", c[1].to_uppercase(), &c[2]))
        == code.search_key().to_uppercase()
}
pub(crate) fn validate_body(body: &str) -> Result<(), SourceError> {
    body_error(body)?;
    let d = Html::parse_document(body);
    if d.select(&sel(
        "#video_id, .videothumblist, #hinban, .resultList, #centerCol",
    ))
    .next()
    .is_none()
        && d.select(&sel("form[action*='login'], input[type='password']"))
            .next()
            .is_some()
    {
        return Err(SourceError::NeedLogin(
            "站点要求登录；请在数据源设置填写有效 Cookie".into(),
        ));
    }
    Ok(())
}
pub(crate) fn links(d: &Html, selector: &str) -> Vec<String> {
    let mut out = vec![];
    for e in d.select(&sel(selector)) {
        let s = e
            .text()
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !s.is_empty() && !out.contains(&s) {
            out.push(s)
        }
    }
    out
}
pub(crate) fn same_origin(base: &str, href: &str) -> Option<String> {
    let b = reqwest::Url::parse(base).ok()?;
    let u = b.join(href).ok()?;
    (b.origin() == u.origin()).then(|| u.to_string())
}
