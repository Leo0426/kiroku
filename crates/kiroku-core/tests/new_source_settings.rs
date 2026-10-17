use kiroku_core::{
    code::parse_code,
    metadata::{PartialMetadata, SourceSnapshot},
    settings::Settings,
};
#[test]
fn new_sources_disabled_and_old_routes_preserved() {
    let mut old = Settings::default();
    old.sources
        .retain(|s| ["javdb", "javbus", "jav321"].contains(&s.id.as_str()));
    let routes = old.routes.clone();
    let priority = old.field_priority.clone();
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("settings.json");
    old.save(&p).unwrap();
    let loaded = Settings::load(&p).unwrap();
    for id in [
        "dmm",
        "mgstage",
        "fc2",
        "fc2ppvdb",
        "avsox",
        "javlibrary",
        "xcity",
        "avbase",
    ] {
        assert!(
            loaded.sources.iter().any(|s| s.id == id && !s.enabled),
            "{id}"
        );
    }
    assert_eq!(loaded.routes, routes);
    assert_eq!(loaded.field_priority, priority);
    assert_eq!(loaded.build_sources().len(), 3);
    let mut all = loaded;
    for s in &mut all.sources {
        s.enabled = true
    }
    assert_eq!(all.build_sources().len(), 11);
}
#[test]
fn api_endpoint_changes_only_its_snapshot() {
    let settings = Settings::default();
    let mut cfg = settings
        .sources
        .iter()
        .find(|s| s.id == "dmm")
        .unwrap()
        .clone();
    let code = parse_code("MOVX-123.mp4").unwrap();
    let fields = PartialMetadata {
        title: Some("title".into()),
        ..Default::default()
    };
    let snap = SourceSnapshot::configured(&code, &cfg, None, &fields);
    assert!(snap.matches_config(&code, &cfg, None));
    cfg.api_base = "https://example.test/graphql".into();
    assert!(!snap.matches_config(&code, &cfg, None));
    let encoded = serde_json::to_string(&snap).unwrap();
    assert!(!encoded.contains("api.video"));
}
