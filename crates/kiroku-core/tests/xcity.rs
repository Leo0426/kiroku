use kiroku_core::{
    code::parse_code,
    source::{Source, SourceError},
    sources::xcity::{self, XCity},
};
const DETAIL: &str = include_str!("fixtures/xcity/detail_GHI335.html");
#[test]
fn xcity_exact_match_and_relative_cover() {
    let c = parse_code("GHI-00335.mp4").unwrap();
    let m = xcity::parse_detail(DETAIL, "https://xcity.jp/avod/detail/?id=200001", &c).unwrap();
    assert!(m.title.unwrap().contains("Performer F"));
    assert_eq!(m.actors[0].name, "Performer F");
    assert_eq!(m.actors.len(), 1, "navigation idol links are not cast");
    assert_eq!(m.external_id.as_deref(), Some("200001"));
    assert_eq!(m.runtime_min, Some(128));
    assert_eq!(m.release_date.as_deref(), Some("2021-10-02"));
    assert_eq!(m.studio.as_deref(), Some("Studio Six"));
    assert!(m.tags.contains(&"ドラマ".into()));
    assert_eq!(m.thumb_urls, ["https://img.example.test/x/ghi335_l.jpg"]);
}
#[test]
fn xcity_neighbour_not_selected() {
    assert!(matches!(
        xcity::parse_detail(
            DETAIL,
            "https://xcity.jp/",
            &parse_code("GHI-336.mp4").unwrap()
        ),
        Err(SourceError::NotFound)
    ));
}
#[test]
fn xcity_missing_fields_remain_empty() {
    let m = xcity::parse_detail(
        DETAIL,
        "https://xcity.jp/",
        &parse_code("GHI-335.mp4").unwrap(),
    )
    .unwrap();
    assert_eq!(m.score, None);
    assert_eq!(m.publisher, None);
    assert_eq!(m.series, None);
    assert!(m.directors.is_empty());
}
#[test]
fn real_search_and_missing() {
    let c = parse_code("GHI-335.mp4").unwrap();
    assert_eq!(
        xcity::parse_search(
            include_str!("fixtures/xcity/search_GHI335.html"),
            "https://xcity.jp/",
            &c
        )
        .unwrap(),
        ["https://xcity.jp/avod/detail/?id=200001"]
    );
    assert!(xcity::parse_search(
        include_str!("fixtures/xcity/missing.html"),
        "https://xcity.jp/",
        &c
    )
    .unwrap()
    .is_empty());
}
#[test]
fn only_censored_supported() {
    let s = XCity {
        base: "https://xcity.jp".into(),
        cookie: String::new(),
    };
    assert!(s.supports(&parse_code("GHI-335.mp4").unwrap()));
    assert!(!s.supports(&parse_code("FC2-1234567.mp4").unwrap()));
}
#[test]
fn actual_login_response_requires_cookie() {
    let c = parse_code("GHI-335.mp4").unwrap();
    let login = include_str!("fixtures/xcity/login.html");
    assert!(matches!(
        xcity::parse_detail(login, "https://xcity.jp/", &c),
        Err(SourceError::NeedLogin(_))
    ));
    assert!(matches!(
        xcity::parse_search(login, "https://xcity.jp/", &c),
        Err(SourceError::NeedLogin(_))
    ));
}
#[test]
fn missing_cast_does_not_import_navigation_idols() {
    // Explicit mutation of the fixture to exercise optional-field absence.
    let without_cast = DETAIL.replace("/idol/detail/10001/", "/removed/10001/");
    let m = xcity::parse_detail(
        &without_cast,
        "https://xcity.jp/",
        &parse_code("GHI-335.mp4").unwrap(),
    )
    .unwrap();
    assert!(m.actors.is_empty(), "{:?}", m.actors);
}
