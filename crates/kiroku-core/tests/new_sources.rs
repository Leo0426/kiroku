use kiroku_core::{
    source::SourceError,
    sources::{avsox, dmm, fc2, fc2ppvdb, mgstage},
};
#[test]
fn dmm_digital_detail() {
    let m = dmm::parse_detail(
        include_str!("fixtures/dmm/dmm_api_detail.json"),
        "xyz00123",
    )
    .unwrap();
    assert_eq!(m.runtime_min, Some(163));
    assert_eq!(m.release_date.as_deref(), Some("2022-06-03"));
    assert_eq!(m.actors[0].name, "Performer G");
    assert!((m.score.unwrap() - 95.2).abs() < 0.01);
    assert_eq!(m.extrafanart.len(), 12);
    assert!(matches!(
        dmm::parse_detail(
            include_str!("fixtures/dmm/dmm_api_empty.json"),
            "zzzz999999999"
        ),
        Err(SourceError::NotFound)
    ));
    assert!(matches!(
        dmm::parse_detail(
            include_str!("fixtures/dmm/dmm_api_neighbor.json"),
            "xyz00123"
        ),
        Err(SourceError::Parse(_))
    ));
}
#[test]
fn dmm_exact_search() {
    assert_eq!(
        dmm::parse_search(
            include_str!("fixtures/dmm/dmm_search_def175_exact.json"),
            "DEF-175"
        )
        .unwrap(),
        vec!["1def00175"]
    );
    assert!(dmm::parse_search(
        include_str!("fixtures/dmm/dmm_search_def_short.json"),
        "DEF-001"
    )
    .unwrap()
    .is_empty());
    assert!(dmm::parse_search(
        include_str!("fixtures/dmm/dmm_search_api_empty_valid.json"),
        "ZZZZ-999999999"
    )
    .unwrap()
    .is_empty());
    assert!(matches!(
        dmm::parse_search(
            include_str!("fixtures/dmm/dmm_search_api_xyz.json"),
            "XYZ-123"
        ),
        Err(SourceError::Parse(_))
    ));
    assert!(dmm::cid_matches("h_068xyz00123", "XYZ-00123"));
    assert!(!dmm::cid_matches("xyz001234", "XYZ-123"));
}
#[test]
fn fc2_detail_and_identity() {
    let m = fc2::parse_detail(
        include_str!("fixtures/fc2/fc2_live_1234567.html"),
        "1234567",
    )
    .unwrap();
    assert!(!m.title.unwrap().is_empty());
    assert_eq!(m.extrafanart.len(), 10);
    assert_eq!(m.release_date, None);
    assert!(
        fc2::parse_detail(
            include_str!("fixtures/fc2/fc2_live_1234567.html"),
            "001234567"
        )
        .is_ok(),
        "provider normalized identity must not erase display zeros"
    );
    assert!(!m.poster_urls.is_empty());
    assert!(matches!(
        fc2::parse_detail(
            include_str!("fixtures/fc2/fc2_live_7654321.html"),
            "1234567"
        ),
        Err(SourceError::Parse(_))
    ));
    assert!(matches!(
        fc2::parse_detail(
            include_str!("fixtures/fc2/fc2_missing.html"),
            "999999999999"
        ),
        Err(SourceError::NotFound)
    ));
}
#[test]
fn avsox_exact_before_fold() {
    assert_eq!(
        avsox::parse_search(
            include_str!("fixtures/avsox/avsox_search.json"),
            "010115-001"
        )
        .unwrap()
        .as_deref(),
        Some("noleepk")
    );
    assert_eq!(
        avsox::parse_search(
            include_str!("fixtures/avsox/avsox_search.json"),
            "010115_001"
        )
        .unwrap()
        .as_deref(),
        Some("kymzwok")
    );
    assert_eq!(
        avsox::parse_search(
            include_str!("fixtures/avsox/avsox_empty.json"),
            "010115-001"
        )
        .unwrap(),
        None
    );
    let m = avsox::parse_detail(
        include_str!("fixtures/avsox/avsox_movie.json"),
        "010115-001",
    )
    .unwrap();
    assert_eq!(m.runtime_min, Some(17));
    assert_eq!(m.actors[0].name, "Performer J");
    assert_eq!(m.studio.as_deref(), Some("Studio Eight"));
    assert!(matches!(
        avsox::parse_detail(
            include_str!("fixtures/avsox/avsox_movie.json"),
            "010115_001"
        ),
        Err(SourceError::Parse(_))
    ));
    assert!(
        !avsox::csrf_token(include_str!("fixtures/avsox/avsox_pure_shell.body"))
            .unwrap()
            .is_empty()
    );
}
#[test]
fn failure_is_never_empty() {
    assert!(matches!(
        mgstage::parse_detail(
            include_str!("fixtures/mgstage/mgstage_detail.html"),
            "259JKL-1456",
            "https://www.mgstage.com/product/product_detail/259JKL-1456/"
        ),
        Err(SourceError::Blocked(_))
    ));
    assert!(matches!(
        fc2ppvdb::parse_detail("", "123", "https://fc2ppvdb.com/articles/123"),
        Err(SourceError::Parse(_))
    ));
}

#[test]
fn unexpected_success_bodies_are_parse_errors_not_missing_products() {
    // Synthetic Avsox shell is replayed as an offline wrong-route body, not claimed as
    // a response from any of these sites.
    let shell = include_str!("fixtures/avsox/avsox_pure_shell.body");
    assert!(matches!(
        mgstage::parse_search(shell, "259JKL-1456", "https://www.mgstage.com"),
        Err(SourceError::Parse(_))
    ));
    assert!(matches!(
        dmm::parse_html_search(shell, "XYZ-123", "https://www.dmm.co.jp"),
        Err(SourceError::Parse(_))
    ));
    assert!(matches!(
        dmm::parse_html_search(
            include_str!("fixtures/dmm/dmm_search.html"),
            "XYZ-123",
            "https://www.dmm.co.jp"
        ),
        Err(SourceError::Blocked(_))
    ));
    assert!(matches!(
        fc2::parse_detail(shell, "123"),
        Err(SourceError::Parse(_))
    ));
    assert!(matches!(
        mgstage::parse_detail(
            shell,
            "259JKL-1456",
            "https://www.mgstage.com/product/product_detail/259JKL-1456/"
        ),
        Err(SourceError::Parse(_))
    ));
    assert!(matches!(
        fc2ppvdb::parse_detail(shell, "123", "https://fc2ppvdb.com/articles/123"),
        Err(SourceError::Parse(_))
    ));
}
#[test]
fn mgstage_literal_identity_rejects_neighbors_and_keeps_zero_semantics() {
    assert!(mgstage::matches_product(
        "https://www.mgstage.com/product/product_detail/259JKL-1456/",
        "259JKL-1456"
    ));
    assert!(!mgstage::matches_product(
        "https://www.mgstage.com/product/product_detail/259JKL-14560/",
        "259JKL-1456"
    ));
    assert!(!mgstage::matches_product(
        "https://www.mgstage.com/product/product_detail/259JKL-1457/",
        "259JKL-1456"
    ));
}

#[test]
fn unauthorized_json_envelope_requires_login_not_parse() {
    // Literal protocol-status input, not a fabricated captured site fixture.
    assert!(
        matches!(avsox::parse_search(r#"{"code":401,"data":null}"#, "010115-001"), Err(SourceError::NeedLogin(message)) if message.contains("401") && message.contains("Cookie"))
    );
}
