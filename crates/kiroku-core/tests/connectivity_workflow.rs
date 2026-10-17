use kiroku_core::{code::ContentType, connectivity, settings::Settings};
use std::{
    io::{Read, Write},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;
fn server(body: &str, status: u16) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let body = body.to_owned();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut b = [0; 4096];
        let _ = s.read(&mut b);
        write!(
            s,
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    format!("http://{addr}")
}
#[tokio::test]
async fn detects_real_block_login_and_success_without_leaking_credentials() {
    for (body, status, reason) in [
        ("<title>Just a moment...</title> cf-chl", 200, "blocked"),
        ("login", 401, "needLogin"),
        ("forbidden", 403, "blocked"),
        ("too many requests", 429, "rateLimited"),
        ("upstream TLS failure", 526, "network"),
        ("JAV321 <form action='/search'>", 200, "ok"),
        ("generic unrelated homepage", 200, "unexpectedResponse"),
    ] {
        let mut cfg = Settings::default();
        cfg.routes.clear();
        cfg.routes
            .insert(ContentType::Censored, vec!["jav321".into()]);
        for s in &mut cfg.sources {
            s.enabled = s.id == "jav321";
            if s.enabled {
                s.base = server(body, status);
                s.cookie = "super-secret-cookie".into();
            }
        }
        let checks = connectivity::check(&cfg, None, &CancellationToken::new()).await;
        assert_eq!(checks.len(), 1);
        let json = serde_json::to_value(&checks[0]).unwrap();
        assert!(json.to_string().contains(reason), "{json}");
        assert!(!json.to_string().contains("super-secret"));
    }
}
#[tokio::test]
async fn unknown_disabled_sources_and_cancel_are_explicit() {
    let cfg = Settings::default();
    let checks = connectivity::check(
        &cfg,
        Some(&["unknown-source".into()]),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(checks[0].outcome.status, "skipped");
    let token = CancellationToken::new();
    token.cancel();
    let checks = connectivity::check(&cfg, None, &token).await;
    assert!(checks.iter().all(|c| c.outcome.reason == "cancelled"));
}

#[tokio::test]
async fn one_failed_source_does_not_hide_another_result() {
    let mut cfg = Settings::default();
    cfg.routes.clear();
    cfg.routes
        .insert(ContentType::Censored, vec!["javdb".into(), "jav321".into()]);
    for s in &mut cfg.sources {
        s.enabled = matches!(s.id.as_str(), "javdb" | "jav321");
        if s.id == "javdb" {
            s.base = server("forbidden", 403)
        }
        if s.id == "jav321" {
            s.base = server("JAV321", 200)
        }
    }
    let r = connectivity::check(&cfg, None, &CancellationToken::new()).await;
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].outcome.reason, "blocked");
    assert_eq!(r[1].outcome.status, "ok");
}
