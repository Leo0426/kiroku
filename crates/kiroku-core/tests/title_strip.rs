//! ADR-0011:标题剔除演员名。
use kiroku_core::{
    metadata::{Actor, Gender},
    names::{self, strip_actor_names, NameRule, NameRules},
    Metadata, PartialMetadata,
};

fn strip(title: &str, names: &[&str]) -> String {
    strip_actor_names(title, &names.iter().map(|s| s.to_string()).collect::<Vec<_>>())
}

#[test]
fn removes_cjk_names_and_tidies_separators() {
    for (title, names, want) in [
        ("新人デビュー テスト花子", &["テスト花子"][..], "新人デビュー"),
        ("テスト花子・サンプル桃子 共演", &["テスト花子", "サンプル桃子"], "共演"),
        ("温泉旅行（テスト花子）", &["テスト花子"], "温泉旅行"),
        ("温泉旅行【テスト花子】 前編", &["テスト花子"], "温泉旅行 前編"),
        ("テスト花子 温泉旅行", &["テスト花子"], "温泉旅行"),
        // 长名优先:不会先删掉「テスト」留下「花子」
        ("温泉 テスト花子", &["テスト", "テスト花子"], "温泉"),
    ] {
        assert_eq!(strip(title, names), want, "{title}");
    }
}

#[test]
fn ascii_names_need_word_boundaries_and_ignore_case() {
    assert_eq!(strip("AIKATSU special AIKA", &["Aika"]), "AIKATSU special");
    assert_eq!(strip("AIKATSU", &["AIKA"]), "AIKATSU");
}

#[test]
fn short_names_and_empty_results_keep_the_original() {
    assert_eq!(strip("花 物語", &["花"]), "花 物語", "单字名不处理");
    assert_eq!(strip("テスト花子", &["テスト花子"]), "テスト花子", "清理后为空时保留原题");
    assert_eq!(strip("  原样  标题 ", &["テスト花子"]), "  原样  标题 ", "没有命中时不做任何整理");
}

#[test]
fn nfc_variants_match() {
    // 「が」分解形式(か + 浊点)
    let decomposed = "か\u{3099}ん 温泉";
    assert_eq!(strip(decomposed, &["がん"]), "温泉");
}

fn meta(title: &str) -> Metadata {
    Metadata {
        code: "SMPL-001".into(),
        fields: PartialMetadata {
            title: Some(title.into()),
            original_title: Some(title.into()),
            actors: vec![Actor::new("花子旧名", Gender::Female)],
            ..Default::default()
        },
        ..Default::default()
    }
}

fn on() -> NameRules {
    NameRules {
        actors: vec![NameRule { canonical: "テスト花子".into(), aliases: vec!["花子旧名".into()] }],
        strip_actor_names_from_title: true,
        ..Default::default()
    }
}

#[test]
fn projection_strips_canonical_and_alias_names_reversibly() {
    let m = meta("温泉 花子旧名");
    let after = names::project(&m, &on()).unwrap().metadata;
    assert_eq!(after.fields.title.as_deref(), Some("温泉"));
    assert_eq!(after.fields.original_title.as_deref(), Some("温泉 花子旧名"), "原题保留来源原文");
    assert_eq!(after.name_projection["title"].original, "温泉 花子旧名");
    // 再投影幂等;关闭开关后还原
    assert_eq!(names::project(&after, &on()).unwrap().metadata, after);
    let restored = names::project(&after, &NameRules { strip_actor_names_from_title: false, ..on() }).unwrap().metadata;
    assert_eq!(restored.fields.title.as_deref(), Some("温泉 花子旧名"));
    assert!(!restored.name_projection.contains_key("title"));
}

#[test]
fn locked_or_manually_edited_titles_are_left_alone() {
    let mut locked = meta("温泉 テスト花子");
    locked.locked_fields.insert("title".into());
    assert_eq!(names::project(&locked, &on()).unwrap().metadata.fields.title.as_deref(), Some("温泉 テスト花子"));

    let mut edited = names::project(&meta("温泉 テスト花子"), &on()).unwrap().metadata;
    edited.fields.title = Some("手改 テスト花子".into());
    let after = names::project(&edited, &on()).unwrap().metadata;
    assert_eq!(after.fields.title.as_deref(), Some("手改"), "人工改动后以新值为原文");
    assert_eq!(after.name_projection["title"].original, "手改 テスト花子");
}

#[test]
fn switch_off_keeps_the_legacy_fingerprint() {
    let legacy = NameRules::default().fingerprint();
    assert_eq!(legacy, NameRules { strip_actor_names_from_title: false, ..Default::default() }.fingerprint());
    assert_ne!(legacy, NameRules { strip_actor_names_from_title: true, ..Default::default() }.fingerprint());
}
