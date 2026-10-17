use kiroku_core::{
    journal::Journal,
    maintenance::{self, RetentionPolicy},
    metadata::{Actor, Gender, SourceSnapshot},
    names::{NameRule, NameRules},
    organize::{self, PlanOpts},
    pipeline,
    placement::OrganizeMode,
    Metadata, PartialMetadata,
};
use std::{collections::HashMap, fs};
use tokio_util::sync::CancellationToken;
fn metadata() -> Metadata {
    Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    }
}
#[test]
fn subtitles_share_final_budgeted_stem_and_part() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("ABC-123-CD2.mp4");
    fs::write(&src, b"video").unwrap();
    for suffix in [".zh-CN.srt", ".en.ass"] {
        fs::write(t.path().join(format!("ABC-123-CD2{suffix}")), b"sub").unwrap();
    }
    let c = kiroku_core::parse_code("ABC-123-CD2.mp4").unwrap();
    let mut m = metadata();
    m.fields.actors = vec![Actor::new("长".repeat(240), Gender::Female)];
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let opts = PlanOpts {
        file_template: "{actors}-{code}-CD{cd?}".into(),
        ..Default::default()
    };
    let plan = organize::plan_with(&src, &c, &m, &root, "{code}", true, opts.clone());
    let media: Vec<_> = plan.ops.iter().filter(|o| o.source().is_some()).collect();
    assert_eq!(media.len(), 3);
    let stem = media[0].target().file_stem().unwrap().to_str().unwrap();
    assert!(stem.ends_with("ABC-123-CD2"));
    for op in &media[1..] {
        let name = op.target().file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(&format!("{stem}.")), "{name}");
        assert!(name.len() <= 240);
        assert_eq!(kiroku_core::parse_code(name).unwrap().part, Some(2));
    }
    let item = pipeline::ScanItem {
        id: 0,
        path: src,
        size: 5,
        info: kiroku_core::code::parse_file_info("ABC-123-CD2.mp4"),
        code: Some(c.clone()),
    };
    let p = pipeline::build_plans(
        &[(&item, &c)],
        &HashMap::from([(m.code.clone(), m)]),
        &root,
        "{code}",
        opts,
    );
    assert_eq!(p[0].to, media[0].target());
    assert!(p[0].notices.iter().any(|n| n.contains(stem)));
}
#[tokio::test]
async fn split_same_code_retained_parts_are_rejected_without_disk_effects() {
    for mode in [
        OrganizeMode::Copy,
        OrganizeMode::Hardlink,
        OrganizeMode::Symlink,
    ] {
        for tpl in ["{raw_name}", "{ext}", "{raw_dir}"] {
            let t = tempfile::tempdir().unwrap();
            let root = t.path().join("lib");
            fs::create_dir(&root).unwrap();
            let input = t.path().join("input");
            for (folder, name) in [("one", "ABC-123-CD1.mp4"), ("two", "ABC-123-CD2.mkv")] {
                fs::create_dir_all(input.join(folder)).unwrap();
                fs::write(input.join(folder).join(name), b"video").unwrap();
            }
            let cancel = CancellationToken::new();
            let scan = pipeline::scan(
                &[input.clone()],
                Some(&root),
                &Default::default(),
                &cancel,
                &mut |_| {},
            );
            let pairs: Vec<_> = scan
                .items
                .iter()
                .map(|i| (i, i.code.as_ref().unwrap()))
                .collect();
            let plans = pipeline::build_plans(
                &pairs,
                &HashMap::from([("ABC-123".into(), metadata())]),
                &root,
                tpl,
                PlanOpts {
                    mode,
                    ..Default::default()
                },
            );
            assert_eq!(plans.len(), 2);
            assert!(plans
                .iter()
                .all(|p| !p.selected && p.conflict == Some(pipeline::Conflict::InvalidTemplate)));
            let http = kiroku_core::net::Http::new(std::time::Duration::ZERO, None).unwrap();
            let out = pipeline::apply(plans, &http, &root, None, &cancel, &|_| {}).await;
            assert!(out.applied.is_empty());
            assert_eq!(out.failed.len(), 2);
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
            assert!(input.join("one/ABC-123-CD1.mp4").exists());
        }
    }
}
#[tokio::test]
async fn selected_refresh_preserves_unadopted_name_fields_and_records() {
    let c = kiroku_core::parse_code("ABC-123").unwrap();
    let mut settings = kiroku_core::settings::Settings::default();
    settings.routes.insert(c.content_type, vec!["javdb".into()]);
    let mut before = metadata();
    before.fields = PartialMetadata {
        title: Some("old title".into()),
        actors: vec![Actor::new("alias", Gender::Female)],
        ..Default::default()
    };
    let old = NameRules {
        actors: vec![NameRule {
            canonical: "old canonical".into(),
            aliases: vec!["alias".into()],
        }],
        ..Default::default()
    };
    before = kiroku_core::names::project(&before, &old).unwrap().metadata;
    settings.name_rules = NameRules {
        actors: vec![NameRule {
            canonical: "new canonical".into(),
            aliases: vec!["alias".into()],
        }],
        ..Default::default()
    };
    let raw = PartialMetadata {
        title: Some("new title".into()),
        actors: vec![Actor::new("alias", Gender::Female)],
        ..Default::default()
    };
    let cfg = settings.sources.iter().find(|s| s.id == "javdb").unwrap();
    before.raw_sources.insert(
        "javdb".into(),
        SourceSnapshot::configured(&c, cfg, None, &raw),
    );
    let sc = pipeline::Scraper::new(settings).unwrap();
    for (fields, replace) in [
        (vec!["title".into()], true),
        (vec![], true),
        (vec!["actors".into()], false),
    ] {
        let after = sc
            .refresh_item(
                &c,
                &before,
                &fields,
                replace,
                false,
                &CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(after.fields.actors, before.fields.actors);
        assert_eq!(
            serde_json::to_value(&after.name_projection).unwrap(),
            serde_json::to_value(&before.name_projection).unwrap()
        );
        assert_eq!(after.raw_sources, before.raw_sources);
    }
}
#[test]
fn title_only_translation_does_not_invent_legacy_original_plot() {
    let mut before = metadata();
    before.fields.title = Some("legacy title".into());
    before.fields.plot = Some("already translated legacy plot".into());
    let candidate = Metadata {
        fields: PartialMetadata {
            title: Some("新しいタイトル".into()),
            ..Default::default()
        },
        ..metadata()
    };
    let mut after =
        kiroku_core::refresh::merge_selected(&before, &candidate, &["title".into()], true).unwrap();
    pipeline::apply_translation(
        &mut after,
        kiroku_core::translate::Translated {
            title: Some("译标题".into()),
            plot: None,
        },
    );
    assert_eq!(
        after.text_versions.original.title.as_deref(),
        Some("新しいタイトル")
    );
    assert!(after.text_versions.original.plot.is_none());
    let p = kiroku_core::localization::project(
        &after,
        &kiroku_core::localization::NfoLanguage::Original,
    );
    assert!(p.warnings.iter().any(|w| w.contains("原文简介未保存")));
    assert_eq!(after.fields.plot, before.fields.plot);
}
#[test]
fn v4_active_and_expired_journals_allow_unrelated_cleanup() {
    for mode in [OrganizeMode::Copy, OrganizeMode::Hardlink] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let src = t.path().join("ABC-123.mp4");
        fs::write(&src, b"video").unwrap();
        let c = kiroku_core::parse_code("ABC-123").unwrap();
        let mut plan = organize::plan_with(
            &src,
            &c,
            &metadata(),
            &root,
            "{code}",
            true,
            PlanOpts {
                mode,
                ..Default::default()
            },
        );
        let mut j = Journal::new(&root);
        kiroku_core::placement::bind_plan(&mut plan, &j).unwrap();
        assert!(j
            .execute(&plan, &mut |_| unreachable!(), &|| false)
            .error
            .is_none());
        assert_eq!(j.version, 4);
        let unused = root.join("ABC-123/.kiroku-original-unused.png");
        fs::write(&unused, b"unused").unwrap();
        let policy = RetentionPolicy {
            history_days: 0,
            log_days: 0,
        };
        let p = maintenance::preview(&root, &t.path().join("logs"), &policy).unwrap();
        assert!(
            !p.protected.iter().any(|s| s.contains("损坏或未知日志")),
            "{:?}",
            p.protected
        );
        assert!(p.candidates.iter().any(|c| c.paths.contains(&unused)));
        let anchors: Vec<_> = j.proofs.values().map(|p| p.stage.clone()).collect();
        assert!(p
            .candidates
            .iter()
            .all(|c| c.paths.iter().all(|p| !anchors.contains(p))));
        assert!(j.rollback().is_empty());
        j.ts = 0;
        j.save().unwrap();
        let p = maintenance::preview(&root, &t.path().join("logs"), &policy).unwrap();
        assert!(
            p.candidates.iter().any(|c| c.paths.contains(&j.path())),
            "{:?}",
            p.protected
        );
    }
}
#[test]
fn manifest_records_stay_with_their_owner_directory() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let mut combined = organize::Plan { ops: vec![] };
    for part in [1, 2] {
        let src = t.path().join(format!("ABC-123-CD{part}.mp4"));
        fs::write(&src, b"video").unwrap();
        let c = kiroku_core::parse_code(src.to_str().unwrap()).unwrap();
        combined.ops.extend(
            organize::plan_with(
                &src,
                &c,
                &metadata(),
                &root,
                "{raw_name}",
                true,
                PlanOpts {
                    mode: OrganizeMode::Copy,
                    ..Default::default()
                },
            )
            .ops,
        );
    }
    kiroku_core::placement::bind_plan(&mut combined, &Journal::new(&root)).unwrap();
    for op in &combined.ops {
        if let organize::Op::WriteFile { path, content } = op {
            if path.file_name().unwrap() == "kiroku.json" {
                let m: Metadata = serde_json::from_str(content).unwrap();
                assert_eq!(m.placements.len(), 1);
                assert_eq!(
                    m.placements[0].anchor.as_ref().unwrap().parent(),
                    path.parent()
                );
            }
        }
    }
}
#[test]
fn invalid_v4_target_and_unknown_versions_fail_closed() {
    for version in [4, 99] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let unused = root.join(".kiroku-original-unused.png");
        fs::write(&unused, b"unused").unwrap();
        let mut j = Journal::new(&root);
        j.version = version;
        j.ops.push(organize::Op::CopyFile {
            from: t.path().join("source.mp4"),
            to: t.path().join("outside.mp4"),
            expected: None,
        });
        j.save().unwrap();
        let p = maintenance::preview(
            &root,
            &t.path().join("logs"),
            &RetentionPolicy {
                history_days: 0,
                log_days: 0,
            },
        )
        .unwrap();
        assert!(!p.candidates.iter().any(|c| c.paths.contains(&unused)));
        assert!(p.protected.iter().any(|s| s.contains("损坏或未知日志")));
    }
}
#[tokio::test]
async fn retained_multipart_import_rebuilds_all_parts_with_confined_proofs() {
    for mode in [
        OrganizeMode::Copy,
        OrganizeMode::Hardlink,
        OrganizeMode::Symlink,
    ] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        fs::create_dir(&root).unwrap();
        let input = t.path().join("input");
        fs::create_dir(&input).unwrap();
        for part in [1, 2] {
            fs::write(
                input.join(format!("ABC-123-CD{part}.mp4")),
                format!("video{part}"),
            )
            .unwrap();
            fs::write(input.join(format!("ABC-123-CD{part}.zh-CN.srt")), b"sub").unwrap();
        }
        let cancel = CancellationToken::new();
        let scan = pipeline::scan(
            &[input.clone()],
            Some(&root),
            &Default::default(),
            &cancel,
            &mut |_| {},
        );
        let pairs: Vec<_> = scan
            .items
            .iter()
            .map(|i| (i, i.code.as_ref().unwrap()))
            .collect();
        let mut m = metadata();
        m.fields.actors = vec![Actor::new("长".repeat(240), Gender::Female)];
        let plans = pipeline::build_plans(
            &pairs,
            &HashMap::from([(m.code.clone(), m)]),
            &root,
            "{code}",
            PlanOpts {
                mode,
                file_template: "{actors}-{code}-CD{cd?}".into(),
                ..Default::default()
            },
        );
        assert!(plans.iter().all(|p| p.selected));
        let http = kiroku_core::net::Http::new(std::time::Duration::ZERO, None).unwrap();
        let out = pipeline::apply(plans, &http, &root, None, &cancel, &|_| {}).await;
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        assert_eq!(out.applied.len(), 2);
        let dir = root.join("ABC-123");
        let saved = kiroku_core::append::read_meta(&dir).unwrap();
        assert_eq!(saved.placements.len(), 4);
        for p in &saved.placements {
            assert_eq!(p.anchor.as_ref().unwrap().parent(), Some(dir.as_path()));
            kiroku_core::placement::resolve_media(&root, &dir, &dir.join(&p.path), &saved).unwrap();
        }
        let library = kiroku_core::library::Library::open(&root).unwrap();
        assert_eq!(library.rebuild(&cancel, &mut |_, _| {}).unwrap(), 1);
        let detail = library.get("ABC-123").unwrap().unwrap();
        assert_eq!(
            detail.files.iter().map(|f| f.part).collect::<Vec<_>>(),
            vec![Some(1), Some(2)]
        );
        assert_eq!(fs::read_dir(&input).unwrap().count(), 4);
    }
}
#[tokio::test]
async fn fresh_cached_source_text_is_captured_before_translation() {
    let c = kiroku_core::parse_code("ABC-123").unwrap();
    let mut settings = kiroku_core::settings::Settings::default();
    settings.routes.insert(c.content_type, vec!["javdb".into()]);
    let raw = PartialMetadata {
        title: Some("新しいタイトル".into()),
        plot: Some("あらすじ".into()),
        ..Default::default()
    };
    let cfg = settings.sources.iter().find(|s| s.id == "javdb").unwrap();
    let snapshots = std::collections::BTreeMap::from([(
        "javdb".into(),
        SourceSnapshot::configured(&c, cfg, None, &raw),
    )]);
    let sc = pipeline::Scraper::new(settings).unwrap();
    let mut m = sc
        .scrape_cached(&c, &snapshots, false, &CancellationToken::new(), &|_| {
            panic!("must use raw cache")
        })
        .await
        .unwrap();
    pipeline::apply_translation(
        &mut m,
        kiroku_core::translate::Translated {
            title: Some("译标题".into()),
            plot: Some("译简介".into()),
        },
    );
    assert_eq!(m.text_versions.original.title, raw.title);
    assert_eq!(m.text_versions.original.plot, raw.plot);
    assert_eq!(m.raw_sources["javdb"].fields, raw);
}
