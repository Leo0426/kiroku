//! 按用户给出的详情页地址取字段:每个支持的来源都用合成详情页 fixture,经本地 HTTP 走一遍传输。
use kiroku_core::{code::parse_code, net::Http, source::Source, sources::{avbase::AvBase, fc2::Fc2, javbus::JavBus, javdb::JavDb, javlibrary::JavLibrary, xcity::XCity}};
use std::{io::{Read, Write}, net::TcpListener, time::Duration};

fn serve(body: &'static str) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || for s in l.incoming() {
        let Ok(mut s) = s else { return };
        let _ = s.read(&mut [0; 16384]);
        let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn supported_sources_parse_a_user_given_detail_page() {
    let http = Http::new(Duration::ZERO, None).unwrap();
    let cases: Vec<(&str, &str, Box<dyn Source>, &str, &str)> = {
        let javdb = serve(include_str!("fixtures/javdb/detail_ABC-001.html"));
        let javbus = serve(include_str!("fixtures/javbus/detail_ABC-001.html"));
        let fc2 = serve(include_str!("fixtures/fc2/fc2_live_7654321.html"));
        let xcity = serve(include_str!("fixtures/xcity/detail_GHI335.html"));
        let javlibrary = serve(include_str!("fixtures/javlibrary/detail_ABC-001.html"));
        let avbase = serve(include_str!("fixtures/avbase/detail_ABC-001.html"));
        vec![
            ("javdb", "ABC-001.mp4", Box::new(JavDb { base: javdb.clone(), cookie: String::new() }), Box::leak(format!("{javdb}/v/abc").into_boxed_str()), "abc-001"),
            ("javbus", "ABC-001.mp4", Box::new(JavBus { base: javbus.clone() }), Box::leak(format!("{javbus}/ABC-001").into_boxed_str()), "abc-001"),
            ("fc2", "FC2-PPV-7654321.mp4", Box::new(Fc2 { base: fc2.clone(), cookie: String::new() }), Box::leak(format!("{fc2}/article/7654321/").into_boxed_str()), "7654321"),
            ("javlibrary", "ABC-001.mp4", Box::new(JavLibrary { base: javlibrary.clone(), cookie: String::new() }), Box::leak(format!("{javlibrary}/cn/vaaa01.html").into_boxed_str()), "abc-001"),
            ("avbase", "ABC-001.mp4", Box::new(AvBase { base: avbase.clone() }), Box::leak(format!("{avbase}/works/ABC-001").into_boxed_str()), "abc-001"),
            ("xcity", "GHI-335.mp4", Box::new(XCity { base: xcity.clone(), cookie: String::new() }), Box::leak(format!("{xcity}/avod/detail/?id=200001").into_boxed_str()), "ghi335"),
        ]
    };
    for (name, file, source, url, mention) in cases {
        let code = parse_code(file).unwrap();
        let (m, page) = source.fetch_detail(url, &code, &http).await.unwrap_or_else(|| panic!("{name} should support detail URLs")).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(m.title.as_deref().is_some_and(|t| !t.is_empty()), "{name}: title");
        assert!(m.source_url.is_some(), "{name}: source_url");
        assert!(page.to_ascii_lowercase().contains(mention), "{name}: page text is returned for code checks");
    }
}

#[tokio::test]
async fn sources_without_verified_detail_pages_decline_urls() {
    let http = Http::new(Duration::ZERO, None).unwrap();
    let code = parse_code("ABC-001.mp4").unwrap();
    let mut settings = kiroku_core::settings::Settings::default();
    for c in settings.sources.iter_mut() { c.enabled = true }
    for source in settings.build_sources() {
        if ["mgstage", "fc2ppvdb", "dmm", "avsox"].contains(&source.id()) {
            assert!(source.fetch_detail("http://127.0.0.1:9/x", &code, &http).await.is_none(), "{}", source.id());
        }
    }
}
