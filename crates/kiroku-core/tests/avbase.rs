//! AVBase:合成的 `__NEXT_DATA__` 页面(见 fixtures/README.md),结构对照 2026-10-15 的真实页面。
use kiroku_core::{
    code::parse_code,
    metadata::{Actor, Gender},
    source::{Source, SourceError},
    sources::avbase::{self, AvBase},
};

const BASE: &str = "https://www.avbase.net";

#[test]
fn search_lists_the_unprefixed_work_first_and_skips_neighbours() {
    let c = parse_code("ABC-001.mp4").unwrap();
    let urls = avbase::parse_search(include_str!("fixtures/avbase/search_ABC-001.html"), BASE, &c).unwrap();
    // 同番号的另一部作品(其他厂牌前缀)排在后面,由详情和跨站日期核对决定
    assert_eq!(urls, [format!("{BASE}/works/ABC-001"), format!("{BASE}/works/otherlabel:ABC-001")]);
    assert!(avbase::parse_search(include_str!("fixtures/avbase/search_ABC-001.html"), BASE, &parse_code("ABC-010.mp4").unwrap()).unwrap().is_empty());
    assert!(avbase::parse_search(include_str!("fixtures/avbase/search_none.html"), BASE, &parse_code("ZZZZ-999.mp4").unwrap()).unwrap().is_empty());
}

#[test]
fn amateur_codes_match_through_the_product_number() {
    let c = parse_code("300MNO-001.mp4").unwrap();
    assert!(AvBase::default().supports(&c), "{:?}", c.content_type);
    let urls = avbase::parse_search(include_str!("fixtures/avbase/search_300MNO-001.html"), BASE, &c).unwrap();
    assert_eq!(urls, [format!("{BASE}/works/MNO-001")]);
}

#[test]
fn detail_maps_every_field_from_page_data() {
    let c = parse_code("ABC-001.mp4").unwrap();
    let m = avbase::parse_detail(include_str!("fixtures/avbase/detail_ABC-001.html"), &c).unwrap();
    assert!(m.title.as_deref().unwrap().starts_with("Sample Title 001"));
    assert_eq!(m.title, m.original_title);
    assert!(m.plot.as_deref().unwrap().starts_with("Synthetic plot"));
    assert_eq!(m.release_date.as_deref(), Some("2021-02-18"));
    assert_eq!(m.runtime_min, Some(147));
    assert_eq!(m.studio.as_deref(), Some("Studio One"));
    assert_eq!(m.publisher.as_deref(), Some("Label One"));
    assert_eq!(m.series, None);
    assert_eq!(m.directors, ["Director A"]);
    assert_eq!(m.actors, [
        Actor::new("Performer B", Gender::Female).with_thumb("https://img.example.test/actors/performer_b.jpg"),
        Actor::new("Performer A", Gender::Female).with_thumb("https://img.example.test/actors/performer_a.jpg"),
    ]);
    assert!(m.tags.contains(&"ドラマ".to_string()) && m.tags.contains(&"独占配信".to_string()), "{:?}", m.tags);
    assert_eq!(m.tags.iter().filter(|t| *t == "ドラマ").count(), 1, "genres and tags are merged without duplicates");
    assert!(m.thumb_urls[0].ends_with("abc00001pl.jpg"), "{:?}", m.thumb_urls);
    assert!(m.extrafanart.len() >= 8 && m.extrafanart[0].ends_with("abc00001jp-1.jpg"));
    assert!(m.trailer_urls[0].ends_with("abc00001_mhb_w.mp4"));
    assert_eq!(m.external_id.as_deref(), Some("ABC-001"));
    // 页面是别的番号时不采用
    assert!(matches!(
        avbase::parse_detail(include_str!("fixtures/avbase/detail_ABC-001.html"), &parse_code("ABC-010.mp4").unwrap()),
        Err(SourceError::NotFound)
    ));
}

#[test]
fn challenge_and_foreign_pages_are_reported_not_parsed() {
    let c = parse_code("ABC-001.mp4").unwrap();
    // 直连时拿到的 Cloudflare 挑战页
    assert!(matches!(
        avbase::parse_search(include_str!("fixtures/javlibrary/challenge.html"), BASE, &c),
        Err(SourceError::Blocked(_))
    ));
    assert!(matches!(avbase::parse_detail("<html><body>hi</body></html>", &c), Err(SourceError::Parse(_))));
    assert!(!AvBase::default().supports(&parse_code("FC2-PPV-1234567.mp4").unwrap()));
}

#[test]
fn mgstage_products_give_runtime_maker_label_and_series() {
    let m = avbase::parse_detail(include_str!("fixtures/avbase/detail_MNO-001.html"), &parse_code("300MNO-001.mp4").unwrap()).unwrap();
    assert_eq!(m.runtime_min, Some(69));
    assert_eq!(m.release_date.as_deref(), Some("2017-01-07"));
    assert_eq!(m.studio.as_deref(), Some("Studio Five"));
    assert_eq!(m.publisher.as_deref(), Some("labelfive"));
    assert_eq!(m.series.as_deref(), Some("Series Five"));
    assert!(m.plot.as_deref().unwrap().starts_with("Synthetic plot"));
}
