use kiroku_core::{code::ContentType, connectivity, net::Http, settings::Settings};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
fn server(body: &str, status: u16) -> String {
    response(format!(
        "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ))
}

fn response(reply: String) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut b = [0; 4096];
        let _ = s.read(&mut b);
        s.write_all(reply.as_bytes()).unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn login_evidence_belongs_to_the_configured_origin() {
    let navigation = "<title>JavDB</title><nav class='navbar'><a href='/logout'>Logout</a></nav>";
    let http = Http::new(Duration::ZERO, None).unwrap();
    let other = server(navigation, 200);
    let start = response(format!("HTTP/1.1 302 Found\r\nLocation: {other}/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"));
    let redirected = connectivity::probe_javdb(&http, &start, "session=fixture").await;
    assert_eq!((redirected.status.as_str(), redirected.reason.as_str()), ("failed", "unexpectedOrigin"));
    let ordinary_page = format!("{}/?next=/login", server(navigation, 200));
    assert_eq!(connectivity::probe_javdb(&http, &ordinary_page, "session=fixture").await.reason, "authenticated");
}

#[tokio::test]
async fn distinguishes_invalid_address_refused_connection_and_truncated_body() {
    let http = Http::new(Duration::ZERO, None).unwrap().with_retry(kiroku_core::net::Retry { attempts: 1, unit: Duration::ZERO });
    let invalid = connectivity::probe_javdb(&http, "not a URL", "").await;
    assert_eq!(invalid.reason, "invalidUrl");
    let address = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let refused = connectivity::probe_javdb(&http, &format!("http://{address}/"), "").await;
    assert_eq!((refused.reason.as_str(), refused.http_status), ("connectionFailed", None));
    let partial_body = response("HTTP/1.1 200 OK\r\nContent-Length: 250\r\nConnection: close\r\n\r\nabc".into());
    assert_eq!(connectivity::probe_javdb(&http, &partial_body, "").await.reason, "responseReadFailed");
    let plaintext = server("unrelated", 200).replacen("http://", "https://", 1);
    assert_eq!(connectivity::probe_javdb(&http, &plaintext, "").await.reason, "tlsError");
}

#[tokio::test]
async fn rejects_an_untrusted_local_certificate_without_disclosing_inputs() {
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let identity = include_bytes!("fixtures/connectivity/untrusted-loopback.pem");
    let config = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(
        vec![CertificateDer::from_pem_slice(identity).unwrap()],
        PrivateKeyDer::from_pem_slice(identity).unwrap(),
    ).unwrap();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut session = rustls::ServerConnection::new(config.into()).unwrap();
        let _ = session.complete_io(&mut stream);
    });
    let http = Http::new(Duration::ZERO, None).unwrap().with_retry(kiroku_core::net::Retry { attempts: 1, unit: Duration::ZERO });
    let check = connectivity::probe_javdb(&http, &format!("https://{addr}/?test=private-query"), "session=private-cookie").await;
    assert_eq!(check.reason, "certificateError");
    assert_eq!(check.http_status, None);
    let serialized = serde_json::to_string(&check).unwrap();
    assert!(!serialized.contains("private-") && !serialized.contains("127.0.0.1"));
    peer.join().unwrap();
}
#[tokio::test]
async fn detects_real_block_login_and_success_without_leaking_credentials() {
    for (body, status, reason) in [
        ("<title>Just a moment...</title> cf-chl", 200, "blocked"),
        ("login", 401, "needLogin"),
        ("forbidden", 403, "blocked"),
        ("too many requests", 429, "rateLimited"),
        ("upstream TLS handshake failure", 525, "upstreamTlsError"),
        ("upstream certificate rejected", 526, "upstreamTlsError"),
        ("upstream unavailable", 502, "upstreamError"),
        ("service temporarily unavailable", 503, "upstreamError"),
        ("proxy requires authentication", 407, "proxyAuthRequired"),
        ("not accepted", 400, "httpError"),
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
        assert_eq!(json["outcome"]["reason"], reason);
        assert_eq!(json["outcome"]["httpStatus"], status);
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
