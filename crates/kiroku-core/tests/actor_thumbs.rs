//! ADR-0009:演员头像写进 NFO。
use kiroku_core::{
    aggregate::aggregate,
    metadata::{Actor, Gender},
    nfo, Metadata, PartialMetadata,
};

#[test]
fn thumbs_are_backfilled_by_name_from_any_source_in_route_order() {
    let javdb = PartialMetadata { actors: vec![Actor::new("Performer A", Gender::Female), Actor::new("Performer B", Gender::Female)], ..Default::default() };
    let javbus = PartialMetadata { actors: vec![Actor::new("Performer A", Gender::Female).with_thumb("https://bus/a.jpg")], ..Default::default() };
    let avbase = PartialMetadata { actors: vec![
        Actor::new("Performer A", Gender::Female).with_thumb("https://avbase/a.jpg"),
        Actor::new("Performer B", Gender::Female).with_thumb("https://avbase/b.jpg"),
    ], ..Default::default() };
    let parts = vec![("javdb", javdb), ("javbus", javbus), ("avbase", avbase)];
    let chains = kiroku_core::aggregate::Chains { route: vec!["javdb".into(), "javbus".into(), "avbase".into()], ..Default::default() };
    let m = aggregate("SMPL-001", &parts, &chains);
    let thumbs: Vec<_> = m.fields.actors.iter().map(|a| a.thumb.as_deref()).collect();
    assert_eq!(thumbs, [Some("https://bus/a.jpg"), Some("https://avbase/b.jpg")]);
}

#[test]
fn nfo_writes_order_and_thumb_and_reads_them_back() {
    let m = Metadata {
        code: "SMPL-001".into(),
        fields: PartialMetadata {
            title: Some("t".into()),
            actors: vec![Actor::new("A&B", Gender::Female).with_thumb("https://x/a.jpg?x=1&y=2"), Actor::new("C", Gender::Female)],
            ..Default::default()
        },
        ..Default::default()
    };
    let xml = nfo::to_nfo(&m);
    assert!(xml.contains("<actor>\n    <name>A&amp;B</name>\n    <type>Actor</type>\n    <order>0</order>\n    <thumb>https://x/a.jpg?x=1&amp;y=2</thumb>\n  </actor>"), "{xml}");
    assert!(xml.contains("<name>C</name>\n    <type>Actor</type>\n    <order>1</order>\n  </actor>"), "{xml}");
    let back = kiroku_core::nfo_read::parse_nfo(&xml, "SMPL-001").unwrap();
    assert_eq!(back.fields.actors[0].thumb.as_deref(), Some("https://x/a.jpg?x=1&y=2"));
    assert_eq!(back.fields.actors[1].thumb, None);
}

#[test]
fn legacy_sidecar_actors_without_thumb_still_load_and_serialize_unchanged() {
    let a: Actor = serde_json::from_str(r#"{"name":"X","gender":"female"}"#).unwrap();
    assert_eq!(a.thumb, None);
    assert_eq!(serde_json::to_string(&a).unwrap(), r#"{"name":"X","gender":"female"}"#);
}

#[test]
fn merged_aliases_keep_a_thumb() {
    use kiroku_core::names::{self, NameRule, NameRules};
    let m = Metadata {
        code: "SMPL-001".into(),
        fields: PartialMetadata {
            actors: vec![Actor::new("旧名", Gender::Female), Actor::new("新名", Gender::Female).with_thumb("https://x/a.jpg")],
            ..Default::default()
        },
        ..Default::default()
    };
    let rules = NameRules { actors: vec![NameRule { canonical: "新名".into(), aliases: vec!["旧名".into()] }], ..Default::default() };
    let after = names::project(&m, &rules).unwrap().metadata;
    assert_eq!(after.fields.actors, vec![Actor::new("新名", Gender::Female).with_thumb("https://x/a.jpg")]);
}
