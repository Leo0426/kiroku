use kiroku_core::{
    metadata::{Actor, Gender},
    template::{self, Ctx},
    Metadata,
};
use std::path::Path;
#[test]
fn contexts_and_exact_validation() {
    let mut m = Metadata::default();
    m.code = "SMPL-001".into();
    m.fields.actors = vec![
        Actor::new("A", Gender::Female),
        Actor::new("B", Gender::Male),
        Actor::new("A", Gender::Female),
    ];
    m.fields.runtime_min = Some(90);
    let c = kiroku_core::code::parse_code(&m.code).unwrap();
    let ctx = Ctx::with_source(&m, Some(&c), Path::new("/inbox/Folder/raw.mp4"));
    assert_eq!(
        template::render_dir(
            "{prefix}/{suffix}/{actors}/{actresses}/{raw_name}/{raw_dir}/{ext}/{runtime}",
            &ctx
        ),
        "SMPL/001/A、B/A/raw/Folder/mp4/90"
    );
    assert!(template::validate_file_template("{codefake}").is_err());
    assert!(template::validate_file_template("{code}.{ext}").is_err());
    assert!(template::validate_template("{code}[oops", true).is_err());
}
#[test]
fn unicode_budget_protects_suffix_code_and_part_and_counts_extension() {
    let mut m = Metadata::default();
    m.code = "SMPL-001".into();
    m.fields.actors = vec![Actor::new("长".repeat(160), Gender::Female)];
    let code = kiroku_core::code::parse_code("SMPL-001-CD2.mp4").unwrap();
    let ctx = Ctx::with_source(&m, Some(&code), Path::new("/inbox/SMPL-001-CD2.mp4"));
    let (_, stem, warnings) =
        template::preview_paths("{actors}/{code}", "{actors}-{code}-CD{cd?}", &ctx).unwrap();
    assert!(stem.len() + 4 <= 240);
    assert!(stem.ends_with("SMPL-001-CD2"));
    assert!(warnings.len() >= 2);
    assert!(template::validate_template("C:/outside/{code}", false).is_err());
    assert!(template::validate_template("{code}[a[b]]", true).is_err());
    assert!(template::validate_template("{code}[[x]a]", true).is_err());
    assert!(template::preview_paths("{code}", "{code}-{legacy}", &ctx).is_ok());
    assert!(template::validate_file_template("{code}-{legacy}").is_err());
}
#[test]
fn legacy_settings_keep_unknown_templates_and_other_preferences() {
    let mut settings = kiroku_core::settings::Settings::default();
    settings.file_template = "{code}-{legacy}".into();
    settings.dir_template = "{actor}/{unknown}/{code}".into();
    settings.library_root = Some("/my-library".into());
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.json");
    settings.save(&path).unwrap();
    let loaded = kiroku_core::settings::Settings::load(&path).unwrap();
    assert_eq!(loaded.library_root, settings.library_root);
    assert_eq!(loaded.file_template, settings.file_template);
    assert!(loaded.validate().is_ok());
    assert!(template::validate_file_template(&loaded.file_template).is_err());
}
#[test]
fn truncation_notices_and_collisions_block_execution() {
    use kiroku_core::{
        organize::PlanOpts,
        pipeline::{self, Conflict, ScanItem},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    std::fs::create_dir(&root).unwrap();
    let mut m = Metadata::default();
    m.code = "SMPL-001".into();
    m.fields.actors = vec![Actor::new("长".repeat(160), Gender::Female)];
    let code = kiroku_core::code::parse_code("SMPL-001-CD2.mp4").unwrap();
    let first = ScanItem {
        id: 1,
        path: temp.path().join("SMPL-001-CD2.mp4"),
        size: 1000,
        info: kiroku_core::code::parse_file_info("SMPL-001-CD2.mp4"),
        code: Some(code.clone()),
    };
    let mut second = first.clone();
    second.id = 2;
    second.path = temp.path().join("SMPL-001-CD2 (1).mp4");
    let metas = [(m.code.clone(), m)].into();
    let opts = PlanOpts {
        file_template: "{actors}-{code}-CD{cd?}".into(),
        ..Default::default()
    };
    let plans = pipeline::build_plans(
        &[(&first, &code), (&second, &code)],
        &metas,
        &root,
        "{code}",
        opts,
    );
    assert!(plans
        .iter()
        .any(|p| p.notices.iter().any(|n| n.contains("截短"))));
    assert!(plans
        .iter()
        .any(|p| p.conflict == Some(Conflict::DuplicateInBatch) && !p.selected));
    assert!(plans.iter().all(|p| p
        .to
        .file_name()
        .is_none_or(|n| n.to_string_lossy().len() <= 240)));
}
