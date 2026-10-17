use kiroku_core::{
    names::{self, NameRule, NameRules},
    sources::{dmm, javbus, javdb},
    Metadata, PartialMetadata,
};

fn rule(canonical: &str, aliases: &[&str]) -> NameRule {
    NameRule { canonical: canonical.into(), aliases: aliases.iter().map(|s| s.to_string()).collect() }
}
fn tags(rules: Vec<NameRule>) -> NameRules {
    NameRules { tags: rules, ..Default::default() }
}

#[test]
fn builtin_preset_is_a_valid_rule_set() {
    let preset = names::tag_preset();
    assert!(preset.len() >= 50);
    tags(preset).validate().unwrap();
}

#[test]
fn merging_into_empty_rules_adds_the_whole_preset() {
    let preset = vec![rule("剧情", &["戲劇", "ドラマ"]), rule("恋爱", &["恋愛"])];
    let m = NameRules::default().merge_tags(&preset);
    assert_eq!(m.rules, tags(preset));
    assert_eq!((m.added_rules, m.added_aliases), (2, 3));
}

#[test]
fn merging_never_changes_existing_user_rules() {
    let user = tags(vec![rule("剧情", &["戲劇"]), rule("我的恋爱", &["恋爱"])]);
    let preset = vec![
        rule("剧情", &["戲劇", "ドラマ"]), // 已有规范名:只补 ドラマ
        rule("恋爱", &["恋愛"]),            // 规范名被用户当作别名:整条跳过
        rule("别的剧情", &["戲劇"]),         // 别名都被占用:不加
        rule("纪录片", &["ドキュメンタリー"]),
    ];
    let m = user.merge_tags(&preset);
    assert_eq!(m.rules, tags(vec![rule("剧情", &["戲劇", "ドラマ"]), rule("我的恋爱", &["恋爱"]), rule("纪录片", &["ドキュメンタリー"])]));
    assert_eq!((m.added_rules, m.added_aliases), (1, 2));
    m.rules.validate().unwrap();
    // 再并一次没有变化
    let again = m.rules.merge_tags(&preset);
    assert_eq!((again.added_rules, again.added_aliases, again.rules), (0, 0, m.rules));
}

#[test]
fn source_tags_from_fixtures_project_to_simplified_names() {
    let f = |p: &str| std::fs::read_to_string(format!("tests/fixtures/{p}")).unwrap();
    let rules = NameRules::default().merge_tags(&names::tag_preset()).rules;
    let project = |raw: Vec<String>| {
        let m = Metadata { code: "ABC-001".into(), fields: PartialMetadata { tags: raw, ..Default::default() }, ..Default::default() };
        names::project(&m, &rules).unwrap().metadata.fields.tags
    };
    let db = project(javdb::parse_detail(&f("javdb/detail_ABC-001.html")).unwrap().tags);
    for t in ["剧情", "高画质", "恋爱", "单体作品"] {
        assert!(db.contains(&t.to_string()), "javdb {db:?}");
    }
    let bus = project(javbus::parse_detail(&f("javbus/detail_ABC-001.html"), "https://www.javbus.com").unwrap().tags);
    for t in ["剧情", "高画质", "独家配信", "眼镜", "短发", "单体作品"] {
        assert!(bus.contains(&t.to_string()), "javbus {bus:?}");
    }
    let jp = project(dmm::parse_detail(&f("dmm/dmm_api_detail.json"), "xyz00123").unwrap().tags);
    for t in ["单体作品", "剧情", "高画质", "独家配信", "眼镜"] {
        assert!(jp.contains(&t.to_string()), "dmm {jp:?}");
    }
}
