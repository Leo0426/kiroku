//! JavDB 数据源:搜索页定位详情页 → 详情页解析。
//! 解析函数为纯函数(见 tests/fixtures/javdb),网络部分只在 `Source::fetch`。

use crate::{code::{search_key, Code}, metadata::{Actor, Gender, PartialMetadata}, net::Http, source::{Source, SourceError, SourceId}};
use async_trait::async_trait;
use scraper::{ElementRef, Html, Selector};

pub struct JavDb {
    /// 域名写进配置:镜像/域名漂移时只改这里
    pub base: String,
    /// 登录后的 Cookie;FC2 等详情页未登录会被重定向到 `/login`
    pub cookie: String,
}

impl Default for JavDb {
    fn default() -> Self {
        Self { base: "https://javdb.com".into(), cookie: String::new() }
    }
}

/// 整理用户粘贴的 Cookie:去掉开头的 `Cookie:`、换行和首尾空白(换行会让请求头非法)。
pub fn clean_cookie(raw: &str) -> String {
    let s = raw.trim();
    let s = s.strip_prefix("Cookie:").or_else(|| s.strip_prefix("cookie:")).unwrap_or(s);
    s.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ")
}

/// 跟随重定向后的最终地址是登录页 → 该页需要登录。
pub fn is_login_redirect(final_url: &str) -> bool {
    reqwest::Url::parse(final_url).map(|u| u.path().trim_end_matches('/') == "/login").unwrap_or(false)
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn text(e: ElementRef) -> String {
    e.text().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 搜索页 → 番号完全匹配的详情页路径(如 `/v/ZY5eq`)。
pub fn parse_search(html: &str, code: &str) -> Option<String> {
    // 两侧同口径归一:大写、FC2-PPV-n → FC2-n、去掉多余前导零
    let norm = |s: &str| search_key(&s.to_uppercase().replace("FC2-PPV-", "FC2-"));
    let want = norm(code);
    let doc = Html::parse_document(html);
    let (item, title) = (sel("div.item"), sel("div.video-title strong"));
    let a = sel("a.box");
    doc.select(&item).find_map(|it| {
        let id = text(it.select(&title).next()?);
        (norm(&id) == want).then(|| it.select(&a).next()?.value().attr("href").map(String::from))?
    })
}

/// 详情页 → 部分元数据。
pub fn parse_detail(html: &str) -> Result<PartialMetadata, SourceError> {
    let doc = Html::parse_document(html);
    let mut m = PartialMetadata::default();

    m.title = doc.select(&sel("h2.title .current-title")).next().map(text).filter(|s| !s.is_empty());
    m.original_title = doc.select(&sel("h2.title .origin-title")).next().map(text).filter(|s| !s.is_empty());
    if m.title.is_none() && m.original_title.is_none() {
        return Err(SourceError::Parse("title not found".into()));
    }

    let block = sel("nav.movie-panel-info div.panel-block");
    let (label, value, link) = (sel("strong"), sel("span.value"), sel("a"));
    for b in doc.select(&block) {
        let (Some(l), Some(v)) = (b.select(&label).next(), b.select(&value).next()) else { continue };
        let name = text(l);
        let name = name.trim_end_matches([':', ':']);
        let plain = text(v);
        let links = |_: bool| -> Vec<String> { v.select(&link).map(text).collect() };
        match name {
            "日期" => m.release_date = Some(plain),
            "時長" => m.runtime_min = plain.split_whitespace().next().and_then(|n| n.parse().ok()),
            "片商" => m.studio = Some(plain),
            "發行" | "發行商" => m.publisher = Some(plain),
            "系列" => m.series = Some(plain),
            "導演" => m.directors = links(false),
            "類別" => m.tags = links(false),
            "評分" => {
                // "4.43分, 由2258人評價":5 分制 → 0–100
                m.score = plain.split('分').next().and_then(|n| n.trim().parse::<f32>().ok()).map(|v| v * 20.0);
            }
            "演員" => {
                // 带 actor-female 的是女优;同一栏里有女优标记时,其余视为男优;全无标记则无法判定
                let all: Vec<(String, bool)> =
                    v.select(&link).map(|a| (text(a), a.value().classes().any(|c| c == "actor-female"))).collect();
                let any_female = all.iter().any(|(_, f)| *f);
                m.actors = all
                    .into_iter()
                    .map(|(n, f)| Actor::new(n, if f { Gender::Female } else if any_female { Gender::Male } else { Gender::Unknown }))
                    .collect();
            }
            _ => {}
        }
    }

    // JavDB 的封面是横版(封底|封面)
    m.thumb_urls = doc.select(&sel("img.video-cover")).next().and_then(|i| i.value().attr("src")).map(String::from).into_iter().collect();
    m.extrafanart = doc
        .select(&sel("div.preview-images a.tile-item"))
        .filter_map(|a| a.value().attr("href").map(String::from))
        .collect();
    Ok(m)
}

#[async_trait]
impl Source for JavDb {
    async fn check_connectivity(&self,http:&Http)->crate::connectivity::ConnectivityOutcome {crate::connectivity::probe_javdb(http,&self.base,&self.cookie).await}
    fn id(&self) -> SourceId {
        "javdb"
    }

    fn supports(&self, code: &Code) -> bool {
        // JavDB 是综合索引:有码/无码/FC2/素人等几乎所有类型都收录
        let _ = code;
        true
    }

    async fn fetch(&self, code: &Code, http: &Http) -> Result<PartialMetadata, SourceError> {
        let check = |st: u16| match st {
            200 => Ok(()),
            404 => Err(SourceError::NotFound),
            403 | 429 | 503 => Err(SourceError::Blocked(format!("status {st}"))),
            s => Err(SourceError::Network(format!("status {s}"))),
        };
        let net = |e: reqwest::Error| SourceError::Network(e.to_string());
        let cookie = clean_cookie(&self.cookie);
        let headers: Vec<(&str, &str)> = if cookie.is_empty() { vec![] } else { vec![("Cookie", cookie.as_str())] };
        let need_login = || {
            SourceError::NeedLogin(if cookie.is_empty() {
                "该影片需要登录 JavDB 才能查看,请在 设置 → 数据源 填写 JavDB Cookie".into()
            } else {
                "JavDB Cookie 已失效,请重新登录后在 设置 → 数据源 更新".into()
            })
        };

        let q = code.search_key();
        let q = q.trim_start_matches("FC2-");
        let page = http.get_text_with(&format!("{}/search?q={}&f=all", self.base, q), &headers).await.map_err(net)?;
        if is_login_redirect(&page.url) {
            return Err(need_login());
        }
        check(page.status)?;
        let path = parse_search(&page.body, &code.id).ok_or(SourceError::NotFound)?;

        let page = http.get_text_with(&format!("{}{}", self.base, path), &headers).await.map_err(net)?;
        if is_login_redirect(&page.url) {
            return Err(need_login());
        }
        check(page.status)?;
        let mut m = parse_detail(&page.body)?;
        m.external_id = path.rsplit('/').next().map(String::from);
        m.source_url = Some(page.url);
        Ok(m)
    }

    async fn fetch_detail(&self, url: &str, _code: &Code, http: &Http) -> Option<Result<(PartialMetadata, String), SourceError>> {
        Some(async {
            let cookie = clean_cookie(&self.cookie);
            let headers: Vec<(&str, &str)> = if cookie.is_empty() { vec![] } else { vec![("Cookie", cookie.as_str())] };
            let page = http.get_text_with(url, &headers).await.map_err(|e| SourceError::Network(e.to_string()))?;
            if is_login_redirect(&page.url) {
                return Err(SourceError::NeedLogin("该影片需要登录 JavDB 才能查看,请在 设置 → 数据源 填写或更新 JavDB Cookie".into()));
            }
            match page.status {
                200 => {}
                404 => return Err(SourceError::NotFound),
                403 | 429 | 503 => return Err(SourceError::Blocked(format!("status {}", page.status))),
                s => return Err(SourceError::Network(format!("status {s}"))),
            }
            let mut m = parse_detail(&page.body)?;
            m.external_id = reqwest::Url::parse(&page.url).ok().and_then(|u| u.path_segments()?.next_back().map(String::from));
            m.source_url = Some(page.url);
            Ok((m, page.body))
        }.await)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cookie_cleanup_and_login_detection() {
        use super::{clean_cookie, is_login_redirect};
        assert_eq!(clean_cookie("  Cookie: _jdb_session=abc; over18=1\n"), "_jdb_session=abc; over18=1");
        assert_eq!(clean_cookie("a=1;\r\n b=2"), "a=1; b=2");
        assert_eq!(clean_cookie("   "), "");
        assert!(is_login_redirect("https://javdb.com/login"));
        assert!(is_login_redirect("https://javdb.com/login/"));
        assert!(!is_login_redirect("https://javdb.com/v/zKk4B7"));
        assert!(!is_login_redirect("not a url"));
    }

    use super::*;

    const SEARCH: &str = include_str!("../../tests/fixtures/javdb/search_ABC-001.html");
    const DETAIL: &str = include_str!("../../tests/fixtures/javdb/detail_ABC-001.html");

    #[test]
    fn search_picks_exact_code() {
        assert_eq!(parse_search(SEARCH, "ABC-001").as_deref(), Some("/v/AAA01"));
        assert_eq!(parse_search(SEARCH, "abc-101").as_deref(), Some("/v/AAA02"));
        assert_eq!(parse_search(SEARCH, "ABC-9999"), None);
        // 带多余前导零的番号(amane 口径不去零)仍能命中
        assert_eq!(parse_search(SEARCH, "ABC-00001").as_deref(), Some("/v/AAA01"));
    }

    #[test]
    fn detail_fields() {
        let m = parse_detail(DETAIL).unwrap();
        assert!(m.title.as_deref().unwrap().contains("Performer A"));
        assert!(m.original_title.as_deref().unwrap().contains("サンプルタイトル"));
        assert_eq!(m.release_date.as_deref(), Some("2021-02-19"));
        assert_eq!(m.runtime_min, Some(150));
        assert_eq!(m.studio.as_deref(), Some("Studio One"));
        assert_eq!(m.directors, vec!["Director A"]);
        assert!((m.score.unwrap() - 88.6).abs() < 0.01, "4.43/5 → 88.6: {:?}", m.score);
        assert!(m.series.is_some());
        // 同栏有女性演员标记时,未标记者视为男性演员
        assert_eq!(
            m.actors,
            vec![Actor::new("Performer A", Gender::Female), Actor::new("Performer B", Gender::Female), Actor::new("Performer C", Gender::Male)]
        );
        assert!(m.tags.contains(&"劇情".to_string()));
        assert_eq!(m.thumb_urls, vec!["https://img.example.test/covers/aa/AAA01.jpg"]);
        assert!(m.extrafanart.len() >= 4 && m.extrafanart[0].contains("samples"));
    }

    #[test]
    fn detail_rejects_garbage() {
        assert!(parse_detail("<html><body>Just a moment</body></html>").is_err());
    }
}
