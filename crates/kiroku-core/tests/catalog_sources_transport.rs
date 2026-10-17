//! Local HTTP contracts verify fetch logic and configuration, not online success.
use kiroku_core::{
    code::parse_code,
    net::Http,
    source::{Source, SourceError},
    sources::{javlibrary::JavLibrary, xcity::XCity},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::Duration,
};
fn server(pages: Vec<(&str, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    let pages: Vec<(String, String)> = pages.into_iter().map(|(p, b)| (p.to_string(), b)).collect();
    let worker = std::thread::spawn(move || {
        let mut requests = vec![];
        for (path, body) in pages {
            let (mut s, _) = l.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut b = vec![0; 16384];
            let n = s.read(&mut b).unwrap();
            let r = String::from_utf8_lossy(&b[..n]).to_string();
            assert!(r.starts_with(&format!("GET {path} ")), "{r}");
            requests.push(r);
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
        requests
    });
    (format!("http://{addr}"), worker)
}
#[tokio::test]
async fn xcity_checks_neighbour_and_honours_cookie_referer() {
    let real = include_str!("fixtures/xcity/detail_GHI335.html");
    let search = r#"<table class="resultList"><tr><td><a href="/avod/detail/?id=wrong">wrong</a><a href="/avod/detail/?id=200001">exact</a><a href="https://untrusted.test/avod/detail/?id=evil">external</a></td></tr></table>"#;
    let (base, worker) = server(vec![
        ("/result/?q=GHI335", search.into()),
        ("/avod/detail/?id=wrong", real.replace("GHI335", "GHI336")),
        ("/avod/detail/?id=200001", real.into()),
    ]);
    let source = XCity {
        base: base.clone(),
        cookie: "Cookie: custom=secret; over18=custom".into(),
    };
    let h = Http::new(Duration::ZERO, None).unwrap();
    let m = source
        .fetch(&parse_code("GHI-00335.mp4").unwrap(), &h)
        .await
        .unwrap();
    assert!(m.title.unwrap().contains("Performer F"));
    let requests = worker.join().unwrap();
    for r in requests {
        let r = r.to_lowercase();
        assert!(r.contains("custom=secret"));
        assert!(r.contains("adc=1"));
        assert!(r.contains("over18=custom"));
        assert!(r.contains("referer: http://127.0.0.1/"));
    }
}
#[tokio::test]
async fn javlibrary_direct_search_hit() {
    let body = r#"<h3 class="post-title"><a>Title</a></h3><div id="video_id"><table><tr><td class="text">GHI-335</td></tr></table></div>"#;
    let (base, worker) = server(vec![("/cn/vl_searchbyid.php?keyword=GHI-335", body.into())]);
    let s = JavLibrary {
        base,
        cookie: "a=b".into(),
    };
    let m = s
        .fetch(
            &parse_code("GHI-335.mp4").unwrap(),
            &Http::new(Duration::ZERO, None).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(m.title.as_deref(), Some("Title"));
    assert!(worker.join().unwrap()[0].contains("a=b"));
}
#[tokio::test]
async fn source_checks_use_configured_hosts_and_classify_challenge() {
    let h = Http::new(Duration::ZERO, None).unwrap();
    let (base, w) = server(vec![(
        "/cn/",
        include_str!("fixtures/javlibrary/challenge.html").into(),
    )]);
    let s = JavLibrary {
        base,
        cookie: String::new(),
    };
    assert_eq!(s.check_connectivity(&h).await.reason, "blocked");
    w.join().unwrap();
    // 探测地址的查询词写死在 XCity::check_connectivity 里
    let (base, w) = server(vec![(
        "/result/?q=ADZ335",
        include_str!("fixtures/xcity/search_GHI335.html").into(),
    )]);
    let s = XCity {
        base,
        cookie: String::new(),
    };
    assert_eq!(s.check_connectivity(&h).await.status, "ok");
    w.join().unwrap();
}
#[tokio::test]
async fn wrong_content_never_requests_network() {
    let h = Http::new(Duration::ZERO, None).unwrap();
    let c = parse_code("FC2-1234567.mp4").unwrap();
    let j = JavLibrary {
        base: "invalid".into(),
        cookie: String::new(),
    };
    let x = XCity {
        base: "invalid".into(),
        cookie: String::new(),
    };
    assert!(matches!(j.fetch(&c, &h).await, Err(SourceError::NotFound)));
    assert!(matches!(x.fetch(&c, &h).await, Err(SourceError::NotFound)));
}
#[tokio::test]
async fn xcity_direct_search_hit_is_verified() {
    let (base, worker) = server(vec![(
        "/result/?q=GHI335",
        include_str!("fixtures/xcity/detail_GHI335.html").into(),
    )]);
    let source = XCity {
        base,
        cookie: String::new(),
    };
    let m = source
        .fetch(
            &parse_code("GHI-335.mp4").unwrap(),
            &Http::new(Duration::ZERO, None).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(m.runtime_min, Some(128));
    worker.join().unwrap();
}
