//! Source-owned, non-destructive connectivity probes using the scraping HTTP stack.
use crate::{
    net::{Http, Page, Retry, Solver},
    settings::{Settings, SourceCfg},
    source::SourceError,
};
use serde::Serialize;
use std::{error::Error, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use tokio_util::sync::CancellationToken;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityOutcome {
    pub status: String,
    pub reason: String,
    pub http_status: Option<u16>,
}
impl ConnectivityOutcome {
    pub fn new(status: &str, reason: &str, http_status: Option<u16>) -> Self {
        Self {
            status: status.into(),
            reason: reason.into(),
            http_status,
        }
    }
    pub fn skipped(reason: &str) -> Self {
        Self::new("skipped", reason, None)
    }
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCheck {
    pub source_id: String,
    pub outcome: ConnectivityOutcome,
    pub elapsed_ms: u128,
    pub checked_at: u64,
}
pub fn assess(page: &Page, markers: &[&str]) -> ConnectivityOutcome {
    let status = Some(page.status);
    let body = page.body.to_lowercase();
    let error = crate::sources::common::check(page).err();
    if let Some(e) = error {
        let reason = match (page.status, e) {
            (407, _) => "proxyAuthRequired",
            (429, _) => "rateLimited",
            (525 | 526, _) => "upstreamTlsError",
            (503, _) if crate::sources::common::body_error(&page.body).is_ok() => "upstreamError",
            (_, SourceError::NeedLogin(_)) => "needLogin",
            (_, SourceError::Blocked(_)) => {
                if body.contains("region") || body.contains("地域") {
                    "geoBlocked"
                } else {
                    "blocked"
                }
            }
            (_, SourceError::NotFound) => "endpointNotFound",
            (500..=599, _) => "upstreamError",
            _ => "httpError",
        };
        return ConnectivityOutcome::new("failed", reason, status);
    }
    if page.body.trim().is_empty() || !markers.iter().any(|s| body.contains(&s.to_lowercase())) {
        return ConnectivityOutcome::new("failed", "unexpectedResponse", status);
    }
    ConnectivityOutcome::new("ok", "ok", status)
}
/// Homepage reachability is not evidence that a supplied session is authenticated.
/// Recognize authentication links only inside the site's navigation; other page links
/// can advertise account features even to logged-out visitors.
pub fn assess_javdb(page: &Page, has_cookie: bool) -> ConnectivityOutcome {
    let mut outcome = assess(page, &["javdb", "video-title"]);
    if outcome.status != "ok" { return outcome; }
    let doc = scraper::Html::parse_document(&page.body);
    let selector = scraper::Selector::parse(".navbar a[href]").expect("static selector");
    let Ok(base) = reqwest::Url::parse(&page.url) else {
        return ConnectivityOutcome::new("failed", "unexpectedResponse", Some(page.status));
    };
    let paths: Vec<String> = doc.select(&selector).filter_map(|a| {
        let url = base.join(a.value().attr("href")?).ok()?;
        (url.origin() == base.origin()).then(|| url.path().trim_end_matches('/').to_owned())
    }).collect();
    outcome.reason = if !has_cookie || paths.iter().any(|p| p == "/login") {
        "unauthenticated"
    } else if paths.iter().any(|p| matches!(p.as_str(), "/logout" | "/users/sign_out" | "/sign_out")) {
        "authenticated"
    } else {
        "authUnknown"
    }.into();
    outcome
}
pub async fn probe_javdb(http: &Http, url: &str, cookie: &str) -> ConnectivityOutcome {
    let Ok(base) = reqwest::Url::parse(url) else {
        return ConnectivityOutcome::new("failed", "invalidUrl", None);
    };
    if !matches!(base.scheme(), "http" | "https") || !base.has_host() {
        return ConnectivityOutcome::new("failed", "invalidUrl", None);
    }
    let cookie = crate::sources::javdb::clean_cookie(cookie);
    http.seed_cookies(url, &cookie);
    match http.get_text(url).await {
        Ok(page) => {
            if !reqwest::Url::parse(&page.url).is_ok_and(|u| u.origin() == base.origin()) {
                return ConnectivityOutcome::new("failed", "unexpectedOrigin", Some(page.status));
            }
            assess_javdb(&page, !cookie.is_empty())
        }
        Err(error) => network(error),
    }
}
fn network(error: reqwest::Error) -> ConnectivityOutcome {
    let reason = if error.is_timeout() { "timeout" }
        else if error.is_builder() { "invalidUrl" }
        else if error.is_redirect() { "redirectError" }
        else if let Some(tls) = tls_failure(&error) { tls }
        else if error.is_connect() { "connectionFailed" }
        else if error.is_body() || error.is_decode() { "responseReadFailed" }
        else { "network" };
    ConnectivityOutcome::new("failed", reason, error.status().map(|s| s.as_u16()))
}

fn tls_failure(error: &(dyn Error + 'static)) -> Option<&'static str> {
    let mut cause = Some(error);
    while let Some(current) = cause {
        if let Some(tls) = current.downcast_ref::<rustls::Error>() {
            return Some(match tls {
                rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented => "certificateError",
                _ => "tlsError",
            });
        }
        // An io::Error may wrap the TLS error directly; source() skips that wrapper's payload.
        cause = match current.downcast_ref::<std::io::Error>().and_then(std::io::Error::get_ref) {
            Some(inner) => Some(inner),
            None => current.source(),
        };
    }
    None
}
pub async fn probe_html(
    http: &Http,
    url: &str,
    cookie: &str,
    defaults: &str,
    markers: &[&str],
) -> ConnectivityOutcome {
    http.seed_cookies(url, defaults);
    http.seed_cookies(url, &crate::sources::javdb::clean_cookie(cookie));
    match http.get_text(url).await {
        Ok(p) => assess(&p, markers),
        Err(e) => network(e),
    }
}
pub async fn probe_dmm(http: &Http, url: &str, cookie: &str) -> ConnectivityOutcome {
    http.seed_cookies(url, &crate::sources::javdb::clean_cookie(cookie));
    match http.post_json_text(url,&serde_json::json!({"query":"query KirokuConnectivity { __typename }","operationName":"KirokuConnectivity"}),&[]).await{Ok(p)=>{let result=assess(&p,&["__typename"]);if result.status!="ok"{return result}match serde_json::from_str::<serde_json::Value>(&p.body){Ok(v) if v["errors"].as_array().is_none_or(|e|e.is_empty())&&v["data"]["__typename"].as_str().is_some()=>result,_=>ConnectivityOutcome::new("failed","unexpectedResponse",Some(p.status))}},Err(e)=>network(e)}
}
pub async fn check(
    settings: &Settings,
    source_ids: Option<&[String]>,
    cancel: &CancellationToken,
) -> Vec<SourceCheck> {
    let ids = source_ids.map(|ids| ids.to_vec()).unwrap_or_else(|| {
        settings
            .routes
            .values()
            .flatten()
            .filter(|id| settings.sources.iter().any(|s| s.id == **id && s.enabled))
            .cloned()
            .collect()
    });
    let mut seen = std::collections::HashSet::new();
    let sources = settings.build_sources();
    // Start without a configured proxy: an invalid global proxy must only
    // affect the sites that inherit it. All clients share cookies and throttling.
    let direct = Http::new(Duration::from_secs(settings.min_interval_secs), None)
        .ok()
        .map(|http| http.with_retry(Retry { attempts: 1, unit: Duration::ZERO }));
    let mut clients = std::collections::HashMap::<String, Http>::new();
    let mut client_for = |cfg: &SourceCfg| -> Option<Http> {
        let direct = direct.as_ref()?;
        let proxy = settings.proxy_for(cfg);
        if let Some((url, timeout)) = settings.solver_for(cfg) {
            let solver = Solver::new(url, timeout, proxy).ok()?;
            return Some(direct.with_solver(Some(solver)));
        }
        let Some(url) = proxy else { return Some(direct.clone()) };
        if let Some(client) = clients.get(url) {
            return Some(client.clone());
        }
        let client = direct.with_proxy(Some(url)).ok()?;
        clients.insert(url.to_owned(), client.clone());
        Some(client)
    };
    let mut out = vec![];
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let start = Instant::now();
        let source = sources.iter().find(|s| s.id() == id);
        let cfg = settings.sources.iter().find(|c| c.enabled && c.id == id);
        let outcome = if cancel.is_cancelled() {
            ConnectivityOutcome::skipped("cancelled")
        } else if let (Some(source), Some(cfg)) = (source, cfg) {
            match client_for(cfg) {
                Some(http) => {
                    tokio::select! {
                        _ = cancel.cancelled() => ConnectivityOutcome::skipped("cancelled"),
                        result = source.check_connectivity(&http) => result,
                    }
                }
                None => ConnectivityOutcome::new("failed", "invalidProxy", None),
            }
        } else {
            ConnectivityOutcome::skipped("disabledOrUnknown")
        };
        out.push(SourceCheck {
            source_id: id,
            outcome,
            elapsed_ms: start.elapsed().as_millis(),
            checked_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn javdb_authentication_requires_evidence_beyond_http_200() {
        let page = |body: &str| Page { status: 200, url: "https://javdb.com/".into(), body: body.into() };
        let logged_out = include_str!("../tests/fixtures/javdb/search_ABC-001.html");
        assert_eq!(assess_javdb(&page(logged_out), true).reason, "unauthenticated");
        assert_eq!(assess_javdb(&page("<title>JavDB</title>"), true).reason, "authUnknown");
        assert_eq!(assess_javdb(&page("<title>JavDB</title>"), false).reason, "unauthenticated");
        // Synthetic navigation isolates the positive marker, not a claimed live response.
        assert_eq!(assess_javdb(&page("<title>JavDB</title><nav class='navbar'><a href='/logout'>登出</a></nav>"), true).reason, "authenticated");
        assert_eq!(assess_javdb(&page("<title>JavDB</title><article><a href='/logout'>登出</a></article>"), true).reason, "authUnknown");
        assert_eq!(assess_javdb(&page("<title>JavDB</title><nav class='navbar'><a href='https://other.example/logout'>登出</a></nav>"), true).reason, "authUnknown");
        let blocked = Page { status: 403, url: "https://javdb.com/".into(), body: "JavDB Access denied".into() };
        assert_eq!(assess_javdb(&blocked, true).reason, "blocked");
    }
    #[tokio::test]
    async fn invalid_proxy_only_affects_the_source_that_uses_it() {
        use crate::settings::SourceProxy;
        let mut settings = Settings::default();
        // An unsupported URL scheme reaches the probe but never opens a socket.
        for cfg in &mut settings.sources {
            cfg.base = "file:///synthetic-connectivity-test".into();
        }
        let ids = ["jav321".to_string()];
        let cancel = CancellationToken::new();
        let reference = check(&settings, Some(&ids), &cancel).await.remove(0).outcome.reason;
        assert_ne!(reference, "invalidProxy");

        let other = settings.sources.iter_mut().find(|c| c.id == "javbus").unwrap();
        other.proxy = SourceProxy::Custom { url: "http://[".into() };
        assert_eq!(check(&settings, Some(&ids), &cancel).await[0].outcome.reason, reference);

        settings.proxy = Some("http://[".into());
        let all = ["javbus".into(), "jav321".into(), "javdb".into()];
        for mode in [SourceProxy::Direct, SourceProxy::Custom { url: "http://127.0.0.1:3128".into() }] {
            settings.sources.iter_mut().find(|c| c.id == "jav321").unwrap().proxy = mode;
            let checks = check(&settings, Some(&all), &cancel).await;
            assert_eq!(checks[0].outcome.reason, "invalidProxy"); // Own custom proxy
            assert_eq!(checks[1].outcome.reason, reference);
            assert_eq!(checks[2].outcome.reason, "invalidProxy"); // Inherited global proxy
        }
    }

    #[tokio::test]
    async fn self_check_uses_each_source_proxy() {
        use std::io::{Read, Write};
        let serve = |status: u16, body: &'static str| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            std::thread::spawn(move || for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let _ = stream.read(&mut [0; 4096]);
                let _ = stream.write_all(format!("HTTP/1.1 {status} T\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
            });
            address
        };
        let site = serve(200, "<title>jav321</title><form action=\"/search\"></form>");
        let dead_proxy = serve(502, "");
        let mut settings = Settings { proxy: Some(format!("http://{dead_proxy}")), ..Settings::default() };
        let jav321 = settings.sources.iter_mut().find(|c| c.id == "jav321").unwrap();
        jav321.base = format!("http://{site}");
        let ids = ["jav321".to_string()];
        assert_ne!(check(&settings, Some(&ids), &CancellationToken::new()).await[0].outcome.status, "ok");
        settings.sources.iter_mut().find(|c| c.id == "jav321").unwrap().proxy = crate::settings::SourceProxy::Direct;
        assert_eq!(check(&settings, Some(&ids), &CancellationToken::new()).await[0].outcome.status, "ok");
    }

    #[tokio::test]
    async fn actual_transport_timeout_is_reported_without_raw_url() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(100));
        });
        let url = format!("http://{address}/?private=secret");
        let error = reqwest::Client::builder()
            .timeout(Duration::from_millis(30))
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap_err();
        let result = network(error);
        assert_eq!(result.reason, "timeout");
        assert!(!serde_json::to_string(&result).unwrap().contains("private"));
        worker.join().unwrap();
    }
}
