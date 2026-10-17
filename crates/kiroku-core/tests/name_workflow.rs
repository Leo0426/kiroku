use kiroku_core::{
    metadata::{Actor, Gender, SourceSnapshot},
    names::{self, NameRule, NameRules},
    refresh, revision, Metadata, PartialMetadata,
};
use std::fs;
fn rules() -> NameRules {
    NameRules {
        actors: vec![NameRule {
            canonical: "花".into(),
            aliases: vec!["旧花".into()],
        }],
        ..Default::default()
    }
}
fn meta() -> Metadata {
    Metadata {
        code: "ABC-001".into(),
        fields: PartialMetadata {
            actors: vec![Actor::new("旧花", Gender::Female)],
            ..Default::default()
        },
        ..Default::default()
    }
}
#[test]
fn manual_edit_clears_name_projection() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("movie");
    fs::create_dir(&dir).unwrap();
    let m = names::project(&meta(), &rules()).unwrap().metadata;
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    fs::write(dir.join("movie.nfo"), kiroku_core::nfo::to_nfo(&m)).unwrap();
    let p = revision::preview(
        t.path(),
        &dir,
        "ABC-001",
        serde_json::json!({"actors":[{"name":"人工名字","gender":"female"}]}),
    )
    .unwrap();
    assert!(!p.after.name_projection.contains_key("actors"));
    let r = revision::apply(&p).unwrap();
    assert!(r.error.is_none());
    let saved: Metadata =
        serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap();
    assert!(!saved.name_projection.contains_key("actors"));
}
#[test]
fn existing_movie_names_undo_without_video_move() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("original-location");
    fs::create_dir(&dir).unwrap();
    let original = serde_json::to_vec(&meta()).unwrap();
    fs::write(dir.join("kiroku.json"), &original).unwrap();
    let nfo = b"<movie><actor><name>old</name></actor><playcount>7</playcount></movie>";
    fs::write(dir.join("movie.nfo"), nfo).unwrap();
    fs::write(dir.join("ABC-001.mp4"), b"video").unwrap();
    let p = names::catalog_preview(t.path(), &dir, "ABC-001", &rules()).unwrap();
    assert_eq!(p.after.fields.actors[0].name, "花");
    let r = revision::apply(&p).unwrap();
    assert!(r.error.is_none(), "{:?}", r.error);
    let saved = fs::read_to_string(dir.join("movie.nfo")).unwrap();
    assert!(saved.contains("<playcount>7</playcount>"));
    assert!(saved.contains("花"));
    let mut j = r.journal;
    assert!(j.rollback().is_empty());
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), original);
    assert_eq!(fs::read(dir.join("ABC-001.mp4")).unwrap(), b"video");
}
#[test]
fn selected_refresh_replaces_projection_original() {
    let before = names::project(&meta(), &rules()).unwrap().metadata;
    let candidate = Metadata {
        fields: PartialMetadata {
            actors: vec![Actor::new("新来源名字", Gender::Female)],
            ..Default::default()
        },
        ..meta()
    };
    let next = refresh::merge_selected(&before, &candidate, &["actors".into()], true).unwrap();
    let next = names::project(&next, &NameRules::default())
        .unwrap()
        .metadata;
    assert_eq!(next.fields.actors[0].name, "新来源名字");
}
#[test]
fn explicit_source_selection_normalizes_before_locking() {
    let mut m = meta();
    let code = kiroku_core::parse_code("ABC-001").unwrap();
    m.raw_sources.insert(
        "javdb".into(),
        SourceSnapshot::new(&code, "https://javdb.com", &m.fields),
    );
    let p = refresh::select_source_named(&m, "actors", "javdb", &rules()).unwrap();
    assert_eq!(p.fields.actors[0].name, "花");
    assert!(p.locked_fields.contains("actors"));
}
#[tokio::test]
async fn cached_scrape_applies_current_rules() {
    let mut settings = kiroku_core::settings::Settings::default();
    settings.name_rules = rules();
    let code = kiroku_core::parse_code("ABC-001").unwrap();
    settings
        .routes
        .insert(code.content_type, vec!["javdb".into()]);
    let cfg = settings.sources.iter().find(|c| c.id == "javdb").unwrap();
    let raw = PartialMetadata {
        title: Some("title".into()),
        ..meta().fields
    };
    let snapshots = std::collections::BTreeMap::from([(
        "javdb".into(),
        SourceSnapshot::configured(&code, cfg, None, &raw),
    )]);
    let sc = kiroku_core::pipeline::Scraper::new(settings).unwrap();
    let result = sc
        .scrape_cached(
            &code,
            &snapshots,
            false,
            &tokio_util::sync::CancellationToken::new(),
            &|_| panic!("cache must avoid network"),
        )
        .await
        .unwrap();
    assert_eq!(result.fields.actors[0].name, "花");
    assert_eq!(result.raw_sources["javdb"].fields.actors[0].name, "旧花");
}
#[test]
fn stale_rules_preview_rejected_and_cancel_is_partial() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("movie");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("kiroku.json"),
        serde_json::to_vec(&meta()).unwrap(),
    )
    .unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let p = kiroku_core::catalog::preview(t.path(), &[dir.clone()], &rules(), &cancel).unwrap();
    assert!(kiroku_core::catalog::apply(&p, &NameRules::default(), &cancel, &|_| {}).is_err());
    let before = fs::read(dir.join("kiroku.json")).unwrap();
    cancel.cancel();
    let r = kiroku_core::catalog::apply(&p, &rules(), &cancel, &|_| {}).unwrap();
    assert!(r.cancelled);
    assert!(r.items.is_empty());
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), before);
}
#[test]
fn batch_cancel_preserves_completed_journal_and_unprocessed_movie() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let dirs = [root.join("first"), root.join("second")];
    for dir in &dirs {
        fs::create_dir(dir).unwrap();
        fs::write(
            dir.join("kiroku.json"),
            serde_json::to_vec(&meta()).unwrap(),
        )
        .unwrap();
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    let p = kiroku_core::catalog::preview(root, &dirs, &rules(), &cancel).unwrap();
    let out = kiroku_core::catalog::apply(&p, &rules(), &cancel, &|_| cancel.cancel()).unwrap();
    assert!(out.cancelled);
    assert_eq!(out.items.len(), 1);
    let first: Metadata =
        serde_json::from_slice(&fs::read(dirs[0].join("kiroku.json")).unwrap()).unwrap();
    let second: Metadata =
        serde_json::from_slice(&fs::read(dirs[1].join("kiroku.json")).unwrap()).unwrap();
    assert_eq!(first.fields.actors[0].name, "花");
    assert_eq!(second.fields.actors[0].name, "旧花");
    assert!(out.items[0].journal_id.is_some());
}
#[test]
fn source_selection_does_not_rename_unselected_fields() {
    let mut m = meta();
    m.fields.studio = Some("旧厂".into());
    let c = kiroku_core::parse_code(&m.code).unwrap();
    m.raw_sources.insert(
        "javdb".into(),
        SourceSnapshot::new(&c, "https://javdb.com", &m.fields),
    );
    let mut r = rules();
    r.studio = vec![NameRule {
        canonical: "新厂".into(),
        aliases: vec!["旧厂".into()],
    }];
    let after = refresh::select_source_named(&m, "actors", "javdb", &r).unwrap();
    assert_eq!(after.fields.actors[0].name, "花");
    assert_eq!(after.fields.studio.as_deref(), Some("旧厂"));
}
#[test]
fn import_plan_surfaces_conflicting_actor_genders() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("library");
    fs::create_dir(&root).unwrap();
    let path = t.path().join("ABC-001.mp4");
    fs::write(&path, b"video").unwrap();
    let mut m = meta();
    m.fields.actors.push(Actor::new("花子", Gender::Male));
    let mut r = rules();
    r.actors[0].aliases.push("花子".into());
    let m = names::project(&m, &r).unwrap().metadata;
    let code = kiroku_core::parse_code("ABC-001").unwrap();
    let item = kiroku_core::pipeline::ScanItem {
        id: 0,
        path,
        size: 5,
        info: kiroku_core::code::parse_file_info("ABC-001.mp4"),
        code: Some(code.clone()),
    };
    let plans = kiroku_core::pipeline::build_plans(
        &[(&item, &code)],
        &std::collections::HashMap::from([(code.id.clone(), m)]),
        &root,
        "{actor}/{code}",
        Default::default(),
    );
    assert!(plans[0].notices.iter().any(|n| n.contains("性别信息冲突")));
}
