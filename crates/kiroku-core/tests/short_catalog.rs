use kiroku_core::{code::{parse_code,parse_text,search_key,ContentType},pipeline,video_filter::VideoFilter};
use tokio_util::sync::CancellationToken;
#[test]
fn fz_short_identifiers_keep_site_spelling_and_route() {
    for input in ["FZ73.mkv","fz65.mp4","FZ-77.mkv","/media/FZ73 示例/video.mkv"] {
        let code=parse_code(input).expect("known short catalog identifier");
        assert!(matches!(code.id.as_str(),"FZ73"|"FZ65"|"FZ77"));
        assert_eq!(code.content_type,ContentType::Uncensored);
    }
    assert_eq!(parse_text("FZ73").number.as_deref(),Some("FZ73"));
    assert_eq!(parse_text("FZ073").number.as_deref(),Some("FZ073"));
    assert_eq!(parse_text("FZ73").prefix,"FZ");
    assert_eq!(search_key("FZ-73"),"FZ73");
    assert_eq!(search_key("FZ073"),"FZ73");
}
#[test]
fn manual_short_identifier_preserves_file_phase() {
    let t=tempfile::tempdir().unwrap();
    std::fs::write(t.path().join("unknown-CD2-C.mkv"),b"video").unwrap();
    let mut result=pipeline::scan(&[t.path().into()],None,&VideoFilter::none(),&CancellationToken::new(),&mut |_|{});
    let item=&mut result.items[0];
    pipeline::override_code(item,"FZ73").unwrap();
    let code=item.code.as_ref().unwrap();
    assert_eq!(code.id,"FZ73");
    assert_eq!(code.part,Some(2));
    assert!(code.subtitled);
    assert_eq!(code.content_type,ContentType::Uncensored);
}
#[test]
fn javdb_short_id_search_matches_only_same_identifier() {
    let html=r#"<div class="item"><a class="box" href="/v/other"></a><div class="video-title"><strong>FZ77</strong></div></div><div class="item"><a class="box" href="/v/example"></a><div class="video-title"><strong>FZ73</strong></div></div>"#;
    assert_eq!(kiroku_core::sources::javdb::parse_search(html,"FZ-73").as_deref(),Some("/v/example"));
    assert_eq!(kiroku_core::sources::javdb::parse_search(html,"FZ65"),None);
}
#[test]
fn short_rule_has_boundaries_and_keeps_phase_markers() {
    for input in ["XFZ73", "FZ73X", "FZ7", "AB73"] {
        assert_ne!(parse_text(input).number.as_deref(),Some("FZ73"),"{input}");
    }
    let code=parse_code("FZ73-CD2-C.mkv").unwrap();
    assert_eq!(code.id,"FZ73");
    assert_eq!(code.part,Some(2));
    assert!(code.subtitled);
    assert_eq!(search_key("ABC-73"),"ABC-073");
}
