//! 求解器(FlareSolverr v1)传输层。`fixtures/flaresolverr/` 是按 FlareSolverr 3.5.2 回复格式手写的合成样例;
//! 回放用的 `ok()` 按同一格式拼装,目标页正文用各站合成 fixture。
//! 真实求解器的端到端检查见 `live_sources.rs`。
use kiroku_core::{
    code::parse_code,
    net::{parse_solution, Http, Retry, Solver},
    settings::{Settings, SourceCfg},
    source::{Source, SourceError},
    sources::javbus::JavBus,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    time::{Duration, Instant},
};

/// 依次回复 `replies`,把每个请求的正文发回测试。
fn solver(replies: Vec<(u16, String)>) -> (String, mpsc::Receiver<String>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", l.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for (status, body) in replies {
            let (mut sock, _) = l.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut bytes = vec![];
            let mut buf = [0; 65536];
            let request = loop {
                let n = sock.read(&mut buf).unwrap();
                bytes.extend_from_slice(&buf[..n]);
                let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                let head = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let len = head.lines().find_map(|l| l.strip_prefix("content-length: ").and_then(|v| v.parse::<usize>().ok())).unwrap_or(0);
                if n == 0 || bytes.len() >= end + 4 + len { break String::from_utf8_lossy(&bytes[end + 4..]).into_owned() }
            };
            tx.send(request).unwrap();
            write!(sock, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    (endpoint, rx)
}

fn ok(url: &str, status: u16, html: &str, cookies: &str) -> String {
    serde_json::json!({
        "status": "ok", "message": "Challenge solved!",
        "solution": { "url": url, "status": status, "headers": {}, "response": html,
            "cookies": serde_json::from_str::<serde_json::Value>(cookies).unwrap(), "userAgent": "Mozilla/5.0 Solver" },
        "startTimestamp": 1, "endTimestamp": 2, "version": "3.3.21"
    }).to_string()
}

fn http(min_interval: Duration) -> Http {
    Http::new(min_interval, None).unwrap().with_retry(Retry { attempts: 1, unit: Duration::ZERO })
}

#[test]
fn captured_solver_replies_are_parsed_and_errors_explained() {
    let s = parse_solution(200, include_bytes!("fixtures/flaresolverr/ok_search.json")).unwrap();
    assert_eq!((s.status, s.url.as_str()), (200, "https://lib.example.test/cn/vl_searchbyid.php?keyword=ABC-001"));
    assert!(s.body.contains("识别码搜寻结果"));
    assert!(s.cookies.iter().any(|c| c.name == "cf_clearance" && c.domain.ends_with("lib.example.test")), "{:?}", s.cookies);
    let err = parse_solution(500, include_bytes!("fixtures/flaresolverr/error_timeout.json")).unwrap_err();
    assert!(err.contains("Timeout after 1.0 seconds"), "{err}");
    assert!(parse_solution(404, b"<html>not here</html>").unwrap_err().contains("不是 FlareSolverr"));
}

#[tokio::test]
async fn a_source_fetches_through_the_solver_with_its_cookies_and_proxy() {
    let html = include_str!("fixtures/javbus/detail_ABC-001.html");
    let (endpoint, requests) = solver(vec![(200, ok("https://www.javbus.com/ABC-001", 200, html,
        r#"[{"name":"cf_clearance","value":"solved","domain":"www.javbus.com","path":"/"}]"#))]);
    let h = http(Duration::ZERO).with_solver(Some(Solver::new(&endpoint, Duration::from_secs(30), Some("socks5://127.0.0.1:1080")).unwrap()));
    h.seed_cookies("https://www.javbus.com/", "session=abc");
    let m = JavBus::default().fetch(&parse_code("ABC-001.mp4").unwrap(), &h).await.unwrap();
    assert!(m.title.is_some());
    assert_eq!(m.source_url.as_deref(), Some("https://www.javbus.com/ABC-001"));

    let sent: serde_json::Value = serde_json::from_str(&requests.recv().unwrap()).unwrap();
    assert_eq!(sent["cmd"], "request.get");
    assert_eq!(sent["url"], "https://www.javbus.com/ABC-001");
    assert_eq!(sent["maxTimeout"], 30000);
    assert_eq!(sent["proxy"]["url"], "socks5://127.0.0.1:1080");
    let cookies: Vec<(String, String)> = sent["cookies"].as_array().unwrap().iter()
        .map(|c| (c["name"].as_str().unwrap().into(), c["value"].as_str().unwrap().into())).collect();
    assert!(cookies.contains(&("session".into(), "abc".into())), "会话 Cookie: {cookies:?}");
    assert!(cookies.iter().any(|(n, _)| n == "existmag"), "来源的 Cookie 头也要带上: {cookies:?}");
}

#[tokio::test]
async fn solver_failures_become_network_errors_without_direct_fallback() {
    let (endpoint, requests) = solver(vec![(500, r#"{"status":"error","message":"Error: Error solving the challenge. Timeout after 60.0 seconds."}"#.into())]);
    let h = http(Duration::ZERO).with_solver(Some(Solver::new(&endpoint, Duration::from_secs(60), None).unwrap()));
    let e = JavBus::default().fetch(&parse_code("ABC-001.mp4").unwrap(), &h).await.unwrap_err();
    assert!(matches!(e, SourceError::Network(ref s) if s.contains("502")), "{e:?}");
    requests.recv().unwrap();
    assert!(requests.recv_timeout(Duration::from_millis(200)).is_err(), "失败后不应再发请求");
}

#[tokio::test]
async fn solved_cookies_join_the_session_and_spacing_follows_the_target_host() {
    let first = ok("https://lib.test/a", 200, "a", r#"[{"name":"cf_clearance","value":"solved","domain":"lib.test","path":"/"}]"#);
    let second = ok("https://lib.test/b", 200, "b", "[]");
    let (endpoint, requests) = solver(vec![(200, first), (200, second)]);
    let h = http(Duration::from_millis(400)).with_solver(Some(Solver::new(&endpoint, Duration::from_secs(5), None).unwrap()));
    let started = Instant::now();
    assert_eq!(h.get_text("https://lib.test/a").await.unwrap().body, "a");
    let page = h.get_text("https://lib.test/b").await.unwrap();
    assert_eq!((page.status, page.body.as_str()), (200, "b"));
    assert!(started.elapsed() >= Duration::from_millis(400), "同一目标站仍按间隔");
    requests.recv().unwrap();
    let sent: serde_json::Value = serde_json::from_str(&requests.recv().unwrap()).unwrap();
    assert_eq!(sent["cookies"][0]["name"], "cf_clearance");
    assert!(sent.get("proxy").is_none());
}

#[tokio::test]
async fn form_posts_go_through_the_solver_and_json_posts_are_refused() {
    let (endpoint, requests) = solver(vec![(200, ok("https://www.jav321.com/video/x", 200, "ok", "[]"))]);
    let h = http(Duration::ZERO).with_solver(Some(Solver::new(&endpoint, Duration::from_secs(5), None).unwrap()));
    assert_eq!(h.post_form_text("https://www.jav321.com/search", &[("sn", "ABC-001"), ("q", "a b&c")]).await.unwrap().body, "ok");
    let sent: serde_json::Value = serde_json::from_str(&requests.recv().unwrap()).unwrap();
    assert_eq!((sent["cmd"].as_str(), sent["postData"].as_str()), (Some("request.post"), Some("sn=ABC-001&q=a+b%26c")));
    let refused = h.post_json_text("https://api.test/graphql", &serde_json::json!({}), &[]).await.unwrap();
    assert_eq!(refused.status, 502);
}

#[test]
fn settings_route_only_opted_in_sources_to_a_configured_solver() {
    let mut s = Settings::default();
    let javlib = |s: &Settings| s.sources.iter().find(|c| c.id == "javlibrary").unwrap().clone();
    assert!(s.solver_for(&javlib(&s)).is_none());
    s.sources.iter_mut().find(|c| c.id == "javlibrary").unwrap().solver = true;
    assert!(s.solver_for(&javlib(&s)).is_none(), "没填地址时这项能力不存在");
    s.solver.url = " http://127.0.0.1:8191/v1 ".into();
    assert_eq!(s.solver_for(&javlib(&s)), Some(("http://127.0.0.1:8191/v1", Duration::from_secs(60))));
    let javdb: &SourceCfg = s.sources.iter().find(|c| c.id == "javdb").unwrap();
    assert!(s.solver_for(javdb).is_none());
    s.validate().unwrap();
    s.solver.url = "127.0.0.1:8191".into();
    assert!(s.validate().unwrap_err().contains("求解器地址"));
    // 旧设置文件没有这些字段
    let old: Settings = serde_json::from_str(r#"{"sources":[{"id":"javdb","enabled":true,"base":"https://javdb.com"}],"min_interval_secs":5,"proxy":null,"dir_template":"{code}"}"#).unwrap();
    assert_eq!((old.solver.url.as_str(), old.solver.timeout_secs, old.sources[0].solver), ("", 60, false));
    assert!(!serde_json::to_string(&old.sources[0]).unwrap().contains("solver"), "默认关闭时保存省略");
}
