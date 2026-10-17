use kiroku_core::{
    journal::{Checkpoint, Journal},
    organize::{Op, Plan},
    placement::OrganizeMode,
};
use std::fs;
fn op(mode: OrganizeMode, from: std::path::PathBuf, to: std::path::PathBuf) -> Op {
    mode.operation(from, to)
}
#[test]
fn retained_modes_survive_every_checkpoint_and_undo_without_touching_source() {
    for mode in [
        OrganizeMode::Copy,
        OrganizeMode::Hardlink,
        OrganizeMode::Symlink,
    ] {
        for point in [
            Checkpoint::Intent,
            Checkpoint::Staged,
            Checkpoint::Published,
            Checkpoint::SourceRemoved,
            Checkpoint::Committed,
        ] {
            let t = tempfile::tempdir().unwrap();
            let root = t.path().join("lib");
            fs::create_dir(&root).unwrap();
            let src = t.path().join("ABC-123.mp4");
            fs::write(&src, b"video").unwrap();
            let dest = root.join("ABC-123.mp4");
            let mut j = Journal::new(&root);
            let ex = j.execute_observed(
                &Plan {
                    ops: vec![op(mode, src.clone(), dest.clone())],
                },
                &mut |_| unreachable!(),
                &|| false,
                &mut |p| {
                    if p == point {
                        Err(std::io::Error::other("interrupt"))
                    } else {
                        Ok(())
                    }
                },
            );
            assert!(ex.error.is_some());
            assert_eq!(fs::read(&src).unwrap(), b"video");
            let mut j = Journal::load(&j.path()).unwrap();
            assert!(j.rollback().is_empty(), "{mode:?} {point:?}");
            assert!(!dest.exists());
            assert_eq!(fs::read(src).unwrap(), b"video");
        }
    }
}
#[test]
fn changed_target_is_never_removed() {
    for mode in [
        OrganizeMode::Copy,
        OrganizeMode::Hardlink,
        OrganizeMode::Symlink,
    ] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        fs::create_dir(&root).unwrap();
        let src = t.path().join("ABC-123.mp4");
        fs::write(&src, b"video").unwrap();
        let dest = root.join("ABC-123.mp4");
        let mut j = Journal::new(&root);
        let ex = j.execute(
            &Plan {
                ops: vec![op(mode, src.clone(), dest.clone())],
            },
            &mut |_| unreachable!(),
            &|| false,
        );
        assert!(ex.error.is_none());
        fs::remove_file(&dest).unwrap();
        fs::write(&dest, b"user content").unwrap();
        assert!(!j.rollback().is_empty());
        assert_eq!(fs::read(dest).unwrap(), b"user content");
        assert_eq!(fs::read(src).unwrap(), b"video");
    }
}
#[test]
fn planned_retained_source_change_is_rejected() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"video").unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let code = kiroku_core::code::parse_code("ABC-123.mp4").unwrap();
    let meta = kiroku_core::metadata::Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    let mut plan = kiroku_core::organize::plan_with(
        &src,
        &code,
        &meta,
        &root,
        "{code}",
        true,
        kiroku_core::organize::PlanOpts {
            mode: OrganizeMode::Copy,
            ..Default::default()
        },
    );
    fs::write(src, b"changed").unwrap();
    let j = Journal::new(&root);
    assert!(kiroku_core::placement::bind_plan(&mut plan, &j).is_err());
}
#[test]
fn registered_link_resolves_and_replacement_is_rejected() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"video").unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let code = kiroku_core::code::parse_code("ABC-123.mp4").unwrap();
    let meta = kiroku_core::metadata::Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    let mut plan = kiroku_core::organize::plan_with(
        &src,
        &code,
        &meta,
        &root,
        "{code}",
        true,
        kiroku_core::organize::PlanOpts {
            mode: OrganizeMode::Symlink,
            ..Default::default()
        },
    );
    let mut j = Journal::new(&root);
    kiroku_core::placement::bind_plan(&mut plan, &j).unwrap();
    assert!(j
        .execute(&plan, &mut |_| unreachable!(), &|| false)
        .error
        .is_none());
    let dir = root.join("ABC-123");
    let meta = kiroku_core::append::read_meta(&dir).unwrap();
    let file = dir.join("ABC-123.mp4");
    assert_eq!(
        kiroku_core::placement::resolve_media(&root, &dir, &file, &meta).unwrap(),
        src.canonicalize().unwrap()
    );
    #[cfg(unix)]
    {
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&src, &file).unwrap();
        assert!(kiroku_core::placement::resolve_media(&root, &dir, &file, &meta).is_err());
    }
}
#[test]
fn root_identity_change_at_staging_never_publishes_into_replacement_root() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"video").unwrap();
    let to = root.join("ABC-123.mp4");
    let mut j = Journal::new(&root);
    let ex = j.execute_observed(
        &Plan {
            ops: vec![op(OrganizeMode::Copy, src.clone(), to.clone())],
        },
        &mut |_| unreachable!(),
        &|| false,
        &mut |p| {
            if p == Checkpoint::Staged {
                fs::rename(&root, t.path().join("offline"))?;
                fs::create_dir(&root)?;
            }
            Ok(())
        },
    );
    assert!(ex.error.is_some());
    assert!(!to.exists());
    assert_eq!(fs::read(src).unwrap(), b"video");
}
#[tokio::test]
async fn retained_multi_part_import_never_cleans_source_and_registers_all_media() {
    use kiroku_core::{organize::PlanOpts, pipeline, Metadata};
    use std::collections::HashMap;
    use tokio_util::sync::CancellationToken;
    for mode in [
        OrganizeMode::Copy,
        OrganizeMode::Hardlink,
        OrganizeMode::Symlink,
    ] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("lib");
        fs::create_dir(&root).unwrap();
        let input = t.path().join("in");
        fs::create_dir(&input).unwrap();
        for name in ["ABC-123-CD1.mp4", "ABC-123-CD2.mp4", "ABC-123-CD1.srt"] {
            fs::write(input.join(name), name.as_bytes()).unwrap();
        }
        let cancel = CancellationToken::new();
        let scan = pipeline::scan(
            &[input.clone()],
            Some(&root),
            &Default::default(),
            &cancel,
            &mut |_| {},
        );
        let pairs = scan
            .items
            .iter()
            .map(|i| (i, i.code.as_ref().unwrap()))
            .collect::<Vec<_>>();
        let metas = HashMap::from([(
            "ABC-123".into(),
            Metadata {
                code: "ABC-123".into(),
                ..Default::default()
            },
        )]);
        let plans = pipeline::build_plans(
            &pairs,
            &metas,
            &root,
            "{code}",
            PlanOpts {
                mode,
                ..Default::default()
            },
        );
        let trash = |_: &std::path::Path| -> std::io::Result<()> {
            panic!("retained mode must never trash")
        };
        let roots = [input.clone()];
        let filter = Default::default();
        let cleanup = pipeline::Cleanup {
            roots: &roots,
            trash: trash,
            filter: &filter,
        };
        let http = kiroku_core::net::Http::new(std::time::Duration::ZERO, None).unwrap();
        let mut out = pipeline::apply(plans, &http, &root, Some(cleanup), &cancel, &|_| {}).await;
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        let meta = kiroku_core::append::read_meta(&root.join("ABC-123")).unwrap();
        assert_eq!(meta.placements.len(), 3);
        assert!(out.journal.rollback().is_empty());
        assert!(input.join("ABC-123-CD1.mp4").exists());
    }
}
#[test]
fn newer_sidecar_blocks_retained_media_deletion_during_import_undo() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"video").unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let code = kiroku_core::code::parse_code("ABC-123.mp4").unwrap();
    let meta = kiroku_core::Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    let mut plan = kiroku_core::organize::plan_with(
        &src,
        &code,
        &meta,
        &root,
        "{code}",
        true,
        kiroku_core::organize::PlanOpts {
            mode: OrganizeMode::Symlink,
            ..Default::default()
        },
    );
    let mut j = Journal::new(&root);
    kiroku_core::placement::bind_plan(&mut plan, &j).unwrap();
    assert!(j
        .execute(&plan, &mut |_| unreachable!(), &|| false)
        .error
        .is_none());
    let dir = root.join("ABC-123");
    let side = dir.join("kiroku.json");
    let mut meta = kiroku_core::append::read_meta(&dir).unwrap();
    meta.fields.title = Some("newer metadata".into());
    fs::remove_file(&side).unwrap();
    fs::write(&side, serde_json::to_vec(&meta).unwrap()).unwrap();
    assert!(!j.rollback().is_empty());
    assert!(dir.join("ABC-123.mp4").exists());
}
