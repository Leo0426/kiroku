use kiroku_core::{
    code::parse_code,
    source::{Source, SourceError},
    sources::javlibrary::{self, JavLibrary},
};
// Deliberately synthetic selector contracts, NOT captured normal-page fixtures.
const CONTRACT: &str = r#"<h3 class="post-title"><a>GHI-335 Title</a></h3><div id="video_id"><table><tr><td class="text">GHI-335</td></tr></table></div><div id="video_date"><table><tr><td class="text">2021-10-02</td></tr></table></div><div id="video_length"><span class="text">128</span></div><div id="video_maker"><table><tr><td class="text"><a>Studio Six</a></td></tr></table></div><div id="video_label"><table><tr><td class="text"><a>Label</a></td></tr></table></div><div id="video_director"><table><tr><td class="text"><a>Director</a></td></tr></table></div><span class="cast"><span class="star"><a>Actor</a></span></span><div id="video_genres"><table><tr><td class="text"><span class="genre"><a>Tag</a></span></td></tr></table></div><img id="video_jacket_img" src="../cover.jpg"><span class="score">(8.25)</span>"#;
#[test]
fn captured_challenge_is_blocked() {
    let c = parse_code("GHI-335.mp4").unwrap();
    let b = include_str!("fixtures/javlibrary/challenge.html");
    assert!(matches!(
        javlibrary::parse_detail(b, "https://www.javlibrary.com/cn/", &c),
        Err(SourceError::Blocked(_))
    ));
    assert!(matches!(
        javlibrary::parse_search(b, "https://www.javlibrary.com/cn/", &c),
        Err(SourceError::Blocked(_))
    ));
}
#[test]
fn selector_contract_exact_fields_and_relative_cover() {
    let c = parse_code("GHI-00335.mp4").unwrap();
    let m = javlibrary::parse_detail(CONTRACT, "https://mirror.test/cn/?v=123", &c).unwrap();
    assert_eq!(m.title.as_deref(), Some("Title"));
    assert_eq!(m.runtime_min, Some(128));
    assert_eq!(m.release_date.as_deref(), Some("2021-10-02"));
    assert_eq!(m.studio.as_deref(), Some("Studio Six"));
    assert_eq!(m.publisher.as_deref(), Some("Label"));
    assert_eq!(m.directors, ["Director"]);
    assert_eq!(m.actors[0].name, "Actor");
    assert_eq!(m.tags, ["Tag"]);
    assert_eq!(m.thumb_urls, ["https://mirror.test/cover.jpg"]);
    assert_eq!(m.score, Some(82.5));
    assert!(matches!(
        javlibrary::parse_detail(
            CONTRACT,
            "https://mirror.test/",
            &parse_code("GHI-336.mp4").unwrap()
        ),
        Err(SourceError::NotFound)
    ));
}
#[test]
fn selector_contract_search_never_selects_neighbour() {
    let b = r#"<div class="videothumblist"><div class="video"><a href="./?v=wrong" title="GHI-336 Neighbor"><div class="id">GHI-336</div></a></div><div class="video"><a href="./?v=exact" title="GHI-335 Exact"><div class="id">GHI-335</div></a></div></div>"#;
    let c = parse_code("GHI-335.mp4").unwrap();
    assert_eq!(
        javlibrary::parse_search(b, "https://mirror.test/cn/vl_searchbyid.php", &c).unwrap(),
        ["https://mirror.test/cn/?v=exact"]
    );
    assert!(javlibrary::parse_search(
        b,
        "https://mirror.test/cn/",
        &parse_code("GHI-337.mp4").unwrap()
    )
    .unwrap()
    .is_empty());
}
#[test]
fn only_censored_supported() {
    let s = JavLibrary {
        base: "https://example.test".into(),
        cookie: String::new(),
    };
    assert!(s.supports(&parse_code("GHI-335.mp4").unwrap()));
    assert!(!s.supports(&parse_code("FC2-1234567.mp4").unwrap()));
}

// 合成的搜索页与详情页(见 fixtures/README.md),结构对照 2026-10-15 的真实页面。
#[test]
fn captured_search_lists_only_exact_codes() {
    let c = parse_code("ABC-001.mp4").unwrap();
    let urls = javlibrary::parse_search(
        include_str!("fixtures/javlibrary/search_ABC-001.html"),
        "https://lib.example.test/cn/vl_searchbyid.php?keyword=ABC-001",
        &c,
    )
    .unwrap();
    // 普通版和蓝光版都是 ABC-001;ABC-010 等相邻番号不选
    assert_eq!(urls, ["https://lib.example.test/cn/vaaa01.html", "https://lib.example.test/cn/vaaa02.html"]);
}
#[test]
fn captured_detail_maps_every_field() {
    let c = parse_code("ABC-001.mp4").unwrap();
    let m = javlibrary::parse_detail(
        include_str!("fixtures/javlibrary/detail_ABC-001.html"),
        "https://lib.example.test/cn/vaaa01.html",
        &c,
    )
    .unwrap();
    assert_eq!(m.title.as_deref(), Some("Sample Title 001"), "番号前缀被去掉");
    assert_eq!(m.release_date.as_deref(), Some("2021-02-19"));
    assert_eq!(m.runtime_min, Some(150));
    assert_eq!(m.studio.as_deref(), Some("Studio One"));
    assert_eq!(m.publisher.as_deref(), Some("Label One"));
    assert_eq!(m.directors, ["Director A"]);
    assert_eq!(m.actors.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Performer A", "Performer B"]);
    assert_eq!(m.tags, ["剧情", "恋爱"]);
    assert_eq!(m.thumb_urls, ["https://img.example.test/abc001/abc001pl.jpg"]);
    assert_eq!(m.score, Some(78.0));
    assert!(matches!(
        javlibrary::parse_detail(include_str!("fixtures/javlibrary/detail_ABC-001.html"), "https://lib.example.test/cn/", &parse_code("ABC-010.mp4").unwrap()),
        Err(SourceError::NotFound)
    ));
}
