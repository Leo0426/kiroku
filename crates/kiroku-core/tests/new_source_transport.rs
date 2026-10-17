//! Loopback replays synthetic bodies (see fixtures/README.md); response headers/session cookie are a transport
//! test harness, not claimed as a captured site response.
use kiroku_core::{
    code::parse_code,
    net::{Http, Retry},
    source::{Source, SourceError},
    sources::{avsox::Avsox, dmm::Dmm, fc2ppvdb::Fc2PpvDb, mgstage::Mgstage},
};
use std::{
    io::{Read, Write},
    time::Duration,
};
fn server(
    responses: Vec<(u16, &'static str, &'static str)>,
) -> (
    String,
    std::sync::mpsc::Receiver<String>,
    std::thread::JoinHandle<()>,
) {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        for (status, headers, body) in responses {
            let (mut sock, _) = l.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut bytes = vec![];
            let mut buf = [0; 4096];
            loop {
                let n = sock.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let h = String::from_utf8_lossy(&bytes[..end]);
                    let len = h
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|v| v.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            tx.send(String::from_utf8(bytes).unwrap()).unwrap();
            write!(sock,"HTTP/1.1 {status} Test\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap()
        }
    });
    (base, rx, thread)
}
fn http() -> Http {
    Http::new(Duration::ZERO, None).unwrap().with_retry(Retry {
        attempts: 1,
        unit: Duration::ZERO,
    })
}
#[tokio::test]
async fn avsox_session_keeps_csrf_with_configured_cookie() {
    let (base, rx, t) = server(vec![
        (
            200,
            "Set-Cookie: _csrf=fresh-session; Path=/\r\n",
            include_str!("fixtures/avsox/avsox_pure_shell.body"),
        ),
        (
            200,
            "Content-Type: application/json\r\n",
            include_str!("fixtures/avsox/avsox_search.json"),
        ),
        (
            200,
            "Content-Type: application/json\r\n",
            include_str!("fixtures/avsox/avsox_movie.json"),
        ),
    ]);
    let source = Avsox {
        base: base.clone(),
        cookie: "preference=cn; _csrf=stale-config".into(),
    };
    let code = parse_code("010115-001.mp4").unwrap();
    let m = source.fetch(&code, &http()).await.unwrap();
    assert_eq!(m.external_id.as_deref(), Some("noleepk"));
    assert_eq!(m.source_url, Some(format!("{base}/cn/movies/noleepk")));
    t.join().unwrap();
    let requests: Vec<_> = rx.try_iter().collect();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].contains("GET /cn/ HTTP/1.1"));
    for request in &requests[1..] {
        let lower = request.to_lowercase();
        assert!(lower.contains("_csrf=fresh-session"), "{request}");
        assert!(!lower.contains("_csrf=stale-config"));
        assert!(lower.contains("preference=cn"));
        assert!(lower.contains(&format!(
            "x-csrf-token: {}",
            kiroku_core::sources::avsox::csrf_token(include_str!(
                "fixtures/avsox/avsox_pure_shell.body"
            ))
            .unwrap()
            .to_lowercase()
        )));
        assert!(lower.contains(&format!("origin: {base}")));
        assert!(lower.contains("x-requested-with: xmlhttprequest"));
        assert!(lower.contains(&format!("referer: {base}/cn/search/010115-001")));
    }
    assert!(requests[1].contains("POST /javu/data/api/search HTTP/1.1"));
    assert!(requests[2].contains("[\"noleepk\",\"cn\"]"));
}
#[tokio::test]
async fn dmm_search_gap_uses_exact_detail_and_configured_api() {
    let (base, rx, t) = server(vec![
        (
            200,
            "",
            include_str!("fixtures/dmm/dmm_search_api_empty_valid.json"),
        ),
        (
            200,
            "",
            include_str!("fixtures/dmm/dmm_search_api_empty_valid.json"),
        ),
        (200, "", include_str!("fixtures/dmm/dmm_api_neighbor.json")),
    ]);
    let source = Dmm {
        base: "https://configured-page.invalid".into(),
        api_base: format!("{base}/custom/graphql"),
        cookie: "custom=1".into(),
    };
    let m = source
        .fetch(&parse_code("XYZ-124.mp4").unwrap(), &http())
        .await
        .unwrap();
    assert_eq!(m.external_id.as_deref(), Some("xyz00124"));
    assert!(m
        .source_url
        .unwrap()
        .starts_with("https://configured-page.invalid/"));
    t.join().unwrap();
    let requests: Vec<_> = rx.try_iter().collect();
    assert_eq!(requests.len(), 3);
    for r in &requests {
        assert!(r.starts_with("POST /custom/graphql HTTP/1.1"));
        assert!(r.to_lowercase().contains("cookie: custom=1"))
    }
    assert!(requests[2].contains("\"id\":\"xyz00124\""));
}
#[tokio::test]
async fn mgstage_interception_is_error_without_search_retry() {
    let (base, rx, t) = server(vec![(
        403,
        "",
        include_str!("fixtures/mgstage/mgstage_detail.html"),
    )]);
    let src = Mgstage {
        base,
        cookie: String::new(),
    };
    assert!(matches!(
        src.fetch(&parse_code("259JKL-1456.mp4").unwrap(), &http())
            .await,
        Err(SourceError::Blocked(_))
    ));
    t.join().unwrap();
    let r = rx.recv().unwrap();
    assert!(r.contains("adc=1"));
    assert!(r.contains("/product/product_detail/259JKL-1456/"));
}
#[tokio::test]
async fn fc2ppvdb_526_is_network_not_challenge_or_empty() {
    let (base, _rx, t) = server(vec![(526, "", "")]);
    let src = Fc2PpvDb {
        base,
        cookie: String::new(),
    };
    assert!(
        matches!(src.fetch(&parse_code("FC2-3000001.mp4").unwrap(),&http()).await,Err(SourceError::Network(s)) if s.contains("526")&&s.contains("TLS"))
    );
    t.join().unwrap();
}

#[tokio::test]
async fn unauthorized_http_status_requires_login_not_network() {
    // Literal HTTP transport harness, no claimed site response/fixture.
    let (base, _rx, t) = server(vec![(401, "", "")]);
    let source = Fc2PpvDb {
        base,
        cookie: "expired=1".into(),
    };
    let result = source
        .fetch(&parse_code("FC2-3000001.mp4").unwrap(), &http())
        .await;
    assert!(
        matches!(result, Err(SourceError::NeedLogin(message)) if message.contains("401") && message.contains("Cookie"))
    );
    t.join().unwrap();
}
