use kiroku_core::{
    code::{parse_code, ContentType},
    settings::Settings,
};
#[test]
fn new_sources_never_expand_old_routes() {
    let old = Settings::default();
    for id in ["javlibrary", "xcity"] {
        assert!(old.sources.iter().any(|s| s.id == id && !s.enabled));
        assert!(!old.routes.values().flatten().any(|s| s == id));
    }
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("settings.json");
    let mut s = old.clone();
    s.sources
        .retain(|s| s.id != "javlibrary" && s.id != "xcity");
    s.save(&p).unwrap();
    let loaded = Settings::load(&p).unwrap();
    assert_eq!(loaded.routes, s.routes);
    assert_eq!(loaded.field_priority, s.field_priority);
    for id in ["javlibrary", "xcity"] {
        assert!(loaded.sources.iter().any(|s| s.id == id && !s.enabled));
    }
}
#[test]
fn new_sources_factory_and_content_gate() {
    let mut s = Settings::default();
    for c in &mut s.sources {
        c.enabled = c.id == "javlibrary" || c.id == "xcity";
        c.base = "https://mirror.test".into();
        c.cookie = "Cookie: test=1".into();
    }
    assert_eq!(s.build_sources().len(), 2);
    for source in s.build_sources() {
        for ct in [
            ContentType::Fc2,
            ContentType::Amateur,
            ContentType::Uncensored,
            ContentType::Chinese,
            ContentType::Western,
            ContentType::Hentai,
        ] {
            let mut c = parse_code("GHI-335.mp4").unwrap();
            c.content_type = ct;
            assert!(!source.supports(&c));
        }
    }
    assert!(s.chains_for(ContentType::Censored).route.is_empty());
}
#[test]
fn new_source_cache_identity_tracks_domain_cookie_and_proxy() {
    use kiroku_core::metadata::{PartialMetadata, SourceSnapshot};
    let settings = Settings::default();
    let code = parse_code("GHI-335.mp4").unwrap();
    for id in ["javlibrary", "xcity"] {
        let cfg = settings.sources.iter().find(|s| s.id == id).unwrap();
        let snapshot = SourceSnapshot::configured(
            &code,
            cfg,
            None,
            &PartialMetadata {
                title: Some("Test title".into()),
                ..Default::default()
            },
        );
        assert!(snapshot.matches_config(&code, cfg, None));
        let mut changed = cfg.clone();
        changed.base = "https://another-mirror.test".into();
        assert!(!snapshot.matches_config(&code, &changed, None));
        changed = cfg.clone();
        changed.cookie = "private=secret".into();
        assert!(!snapshot.matches_config(&code, &changed, None));
        assert!(!snapshot.matches_config(&code, cfg, Some("http://127.0.0.1:8888")));
        let private = SourceSnapshot::configured(
            &code,
            &changed,
            Some("http://127.0.0.1:8888"),
            &PartialMetadata {
                title: Some("Test title".into()),
                ..Default::default()
            },
        );
        let encoded = serde_json::to_string(&private).unwrap();
        assert!(!encoded.contains("private=secret"));
        assert!(!encoded.contains("8888"));
    }
}
