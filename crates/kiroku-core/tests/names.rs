use kiroku_core::{
    metadata::{Actor, Gender},
    names::{self, NameRule, NameRules},
    Metadata, PartialMetadata,
};
fn rule(canonical: &str, aliases: &[&str]) -> NameRule {
    NameRule {
        canonical: canonical.into(),
        aliases: aliases.iter().map(|s| s.to_string()).collect(),
    }
}
fn rules() -> NameRules {
    NameRules {
        actors: vec![rule("花", &["旧花", "花子"])],
        tags: vec![rule("中字", &["中文字幕"])],
        studio: vec![rule("Studio", &["Old studio"])],
        ..Default::default()
    }
}
fn meta() -> Metadata {
    Metadata {
        code: "SMPL-001".into(),
        fields: PartialMetadata {
            actors: vec![
                Actor::new("旧花", Gender::Female),
                Actor::new("花子", Gender::Female),
            ],
            tags: vec!["中文字幕".into(), "中字".into()],
            studio: Some("Old studio".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}
#[test]
fn aliases_deduplicate_and_preserve_raw() {
    let mut m = meta();
    m.provenance.insert("actors".into(), "javdb".into());
    let after = names::project(&m, &rules()).unwrap().metadata;
    assert_eq!(after.fields.actors, vec![Actor::new("花", Gender::Female)]);
    assert_eq!(after.fields.tags, vec!["中字"]);
    assert_eq!(after.fields.studio.as_deref(), Some("Studio"));
    assert_eq!(after.raw_sources, m.raw_sources);
    assert_eq!(after.provenance, m.provenance);
    assert_eq!(names::project(&after, &rules()).unwrap().metadata, after);
}
#[test]
fn locked_empty_fields_win() {
    let mut m = meta();
    m.fields.actors.clear();
    m.fields.studio = None;
    m.locked_fields.extend(["actors".into(), "studio".into()]);
    let p = names::project(&m, &rules()).unwrap().metadata;
    assert!(p.fields.actors.is_empty());
    assert!(p.fields.studio.is_none());
    assert!(!p.name_projection.contains_key("actors"));
}
#[test]
fn ambiguous_alias_and_cycles_rejected() {
    for entries in [
        vec![rule("A", &["x"]), rule("B", &["x"])],
        vec![rule("A", &["B"]), rule("B", &["A"])],
        vec![rule("A", &["B"]), rule("B", &["C"])],
        vec![rule(" ", &["x"])],
        vec![rule("A", &[""])],
        vec![rule("A", &["x", " x "])],
        vec![rule("A", &["x"]), rule("A", &["z"])],
    ] {
        assert!(NameRules {
            actors: entries,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}
#[test]
fn rule_removal_restores_original() {
    let m = meta();
    let mapped = names::project(&m, &rules()).unwrap().metadata;
    let restored = names::project(&mapped, &NameRules::default())
        .unwrap()
        .metadata;
    assert_eq!(restored.fields, m.fields);
    assert!(restored.name_projection.is_empty());
    let changed = NameRules {
        actors: vec![rule("新花", &["旧花", "花子"])],
        ..Default::default()
    };
    assert_eq!(
        names::project(&mapped, &changed)
            .unwrap()
            .metadata
            .fields
            .actors,
        vec![Actor::new("新花", Gender::Female)]
    );
}
#[test]
fn external_edit_invalidates_projection() {
    let mut m = names::project(&meta(), &rules()).unwrap().metadata;
    m.fields.actors = vec![Actor::new("手动名字", Gender::Unknown)];
    let p = names::project(&m, &NameRules::default()).unwrap().metadata;
    assert_eq!(
        p.fields.actors,
        vec![Actor::new("手动名字", Gender::Unknown)]
    );
    assert!(!p.name_projection.contains_key("actors"));
}
#[test]
fn gender_conflict_warns() {
    let mut m = meta();
    m.fields.actors[1].gender = Gender::Male;
    let p = names::project(&m, &rules()).unwrap();
    assert_eq!(
        p.metadata.fields.actors,
        vec![Actor::new("花", Gender::Female)]
    );
    assert!(!p.warnings.is_empty());
}
#[test]
fn unicode_comparison_is_normalized_and_namespaces_are_independent() {
    let r = NameRules {
        actors: vec![rule("新名", &["é"])],
        tags: vec![rule("标签", &["é"])],
        ..Default::default()
    };
    let m = Metadata {
        fields: PartialMetadata {
            actors: vec![Actor::new(" e\u{301} ", Gender::Unknown)],
            tags: vec!["é".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let p = names::project(&m, &r).unwrap().metadata;
    assert_eq!(p.fields.actors[0].name, "新名");
    assert_eq!(p.fields.tags, vec!["标签"]);
}
#[test]
fn old_settings_load_and_invalid_rules_are_rejected_by_settings() {
    let mut s = kiroku_core::settings::Settings::default();
    let mut v = serde_json::to_value(&s).unwrap();
    v.as_object_mut().unwrap().remove("name_rules");
    assert_eq!(
        serde_json::from_value::<kiroku_core::settings::Settings>(v)
            .unwrap()
            .name_rules,
        NameRules::default()
    );
    s.name_rules = NameRules {
        tags: vec![rule("A", &["B"]), rule("C", &["B"])],
        ..Default::default()
    };
    assert!(s.validate().is_err());
}
