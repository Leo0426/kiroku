use kiroku_core::{
    pipeline::{self, import_rules::ImportRules, Conflict, Scraper},
    placement::OrganizeMode,
    settings::Settings,
};
use std::{collections::HashMap, fs};
use tokio_util::sync::CancellationToken;

fn rules() -> ImportRules {
    ImportRules {
        mode: OrganizeMode::Copy,
        subdirectory: "Archive/2026".into(),
        directory_template: "{code}".into(),
        file_template: "{code}".into(),
        plex_naming: false,
    }
}

#[test]
fn batch_rules_reject_escaping_paths_and_do_not_change_defaults() {
    let defaults = Settings::default();
    let effective = rules().project(&defaults).unwrap();
    assert_eq!(effective.organize_mode, OrganizeMode::Copy);
    assert_eq!(effective.dir_template, "Archive/2026/{code}");
    assert_eq!(defaults.organize_mode, OrganizeMode::Move);
    for subdirectory in [
        "/other",
        "../outside",
        "a/../../b",
        r"C:\other",
        ".kiroku",
        "a/{actor}",
    ] {
        assert!(ImportRules {
            subdirectory: subdirectory.into(),
            ..rules()
        }
        .project(&defaults)
        .is_err());
    }
}

#[tokio::test]
async fn a_renamed_copy_rebuilds_companions_and_rolls_back_without_touching_the_occupied_video() {
    let temp = tempfile::tempdir().unwrap();
    let src = temp.path().join("source");
    let root = temp.path().join("library");
    let dest = root.join("Archive/2026/TEST-081");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(&dest).unwrap();
    fs::write(src.join("TEST-081.mp4"), b"new video").unwrap();
    fs::write(src.join("TEST-081.en.srt"), b"captions").unwrap();
    fs::write(
        src.join("TEST-081.nfo"),
        b"<movie><title>Sample</title></movie>",
    )
    .unwrap();
    fs::write(dest.join("TEST-081.mp4"), b"existing").unwrap();
    let mut defaults = Settings::default();
    defaults.scan.min_video_mb = 0;
    let effective = rules().project(&defaults).unwrap();
    let cancel = CancellationToken::new();
    let items = pipeline::scan(
        &[src.clone()],
        Some(&root),
        &effective.video_filter(),
        &cancel,
        &mut |_| {},
    )
    .items;
    let (_, metas) = pipeline::inspect_import(&items, &cancel);
    let pairs = vec![(&items[0], items[0].code.as_ref().unwrap())];
    let plan = |names: &HashMap<u32, String>| {
        pipeline::build_import_plans(
            &pairs,
            &metas,
            &root,
            &effective.dir_template,
            effective.plan_opts(),
            names,
            &cancel,
        )
    };
    assert_eq!(
        plan(&HashMap::new()).unwrap()[0].conflict,
        Some(Conflict::TargetExists)
    );
    assert!(plan(&HashMap::from([(items[0].id, "../escape".into())])).is_err());
    let plans = plan(&HashMap::from([(items[0].id, "alternate".into())])).unwrap();
    assert!(plans[0].selected);
    let scraper = Scraper::with_sources(defaults, vec![]).unwrap();
    let mut outcome = pipeline::apply(plans, &scraper.http, &root, None, &cancel, &|_| {}).await;
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
    for extension in ["mp4", "en.srt", "nfo"] {
        assert!(dest
            .join(format!("TEST-081 - alternate.{extension}"))
            .exists());
        assert!(src.join(format!("TEST-081.{extension}")).exists());
    }
    assert!(outcome.journal.rollback().is_empty());
    assert_eq!(fs::read(dest.join("TEST-081.mp4")).unwrap(), b"existing");
    assert!(!dest.join("TEST-081 - alternate.mp4").exists());
}
