use kiroku_core::{
    code::parse_code,
    journal::Journal,
    library::Library,
    metadata::Metadata,
    organize::{self, PlanOpts},
    placement::OrganizeMode,
};
use std::fs;
#[test]
fn registered_link_indexes_and_offline_source_preserves_index() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"video").unwrap();
    let code = parse_code("ABC-123.mp4").unwrap();
    let meta = Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    let mut plan = organize::plan_with(
        &src,
        &code,
        &meta,
        &root,
        "{code}",
        true,
        PlanOpts {
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
    let lib = Library::open(&root).unwrap();
    let dir = root.join("ABC-123");
    assert_eq!(lib.reindex_dir(&dir).unwrap(), Some("ABC-123".into()));
    fs::remove_file(src).unwrap();
    assert!(lib.reindex_dir(&dir).is_err());
    assert!(lib.get("ABC-123").unwrap().is_some());
}
#[test]
#[cfg(unix)]
fn unregistered_link_never_indexes() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("ABC-123");
    fs::create_dir_all(&dir).unwrap();
    let src = t.path().join("source.mp4");
    fs::write(&src, b"video").unwrap();
    std::os::unix::fs::symlink(src, dir.join("ABC-123.mp4")).unwrap();
    fs::write(
        dir.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    let lib = Library::open(&root).unwrap();
    assert!(lib.reindex_dir(&dir).is_err());
    assert!(lib.get("ABC-123").unwrap().is_none());
}
#[test]
fn retained_append_updates_manifest_and_undo_restores_it() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("ABC-123");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("ABC-123-CD1.mp4"), b"one").unwrap();
    fs::write(
        dir.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    let src = t.path().join("ABC-123-CD2.mp4");
    fs::write(&src, b"two").unwrap();
    let mut p = kiroku_core::append::preview(&root, "ABC-123", &[src.clone()], None).unwrap();
    p.set_mode(OrganizeMode::Symlink).unwrap();
    let result =
        kiroku_core::append::apply(&p, &tokio_util::sync::CancellationToken::new()).unwrap();
    assert!(result.failed.is_empty(), "{:?}", result.failed);
    let m = kiroku_core::append::read_meta(&dir).unwrap();
    assert_eq!(m.placements.len(), 1);
    assert!(
        kiroku_core::placement::resolve_media(&root, &dir, &dir.join("ABC-123-CD2.mp4"), &m)
            .is_ok()
    );
    let mut j = Journal::load(
        &root
            .join(".kiroku")
            .join(format!("journal-{}.json", result.journal_id)),
    )
    .unwrap();
    assert!(j.rollback().is_empty());
    assert!(kiroku_core::append::read_meta(&dir)
        .unwrap()
        .placements
        .is_empty());
    assert!(src.exists());
}
#[test]
fn explicit_link_revalidation_is_previewed_and_source_changes_revoke_it() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"one").unwrap();
    let code = parse_code("ABC-123.mp4").unwrap();
    let meta = Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    let mut plan = organize::plan_with(
        &src,
        &code,
        &meta,
        &root,
        "{code}",
        true,
        PlanOpts {
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
    fs::write(&src, b"two").unwrap();
    let preview = kiroku_core::placement::revalidation_preview(&root, &dir, &code.id).unwrap();
    fs::write(&src, b"three").unwrap();
    assert!(kiroku_core::revision::apply(&preview).is_err());
    let preview = kiroku_core::placement::revalidation_preview(&root, &dir, &code.id).unwrap();
    assert!(kiroku_core::revision::apply(&preview)
        .unwrap()
        .error
        .is_none());
    let m = kiroku_core::append::read_meta(&dir).unwrap();
    assert!(
        kiroku_core::placement::resolve_media(&root, &dir, &dir.join("ABC-123.mp4"), &m).is_ok()
    );
}
#[tokio::test]
async fn retained_existing_multi_part_batch_records_every_append_and_undo() {
    use kiroku_core::pipeline;
    use std::collections::HashMap;
    use tokio_util::sync::CancellationToken;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("ABC-123");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("ABC-123-CD1.mp4"), b"one").unwrap();
    fs::write(
        dir.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    let input = t.path().join("in");
    fs::create_dir(&input).unwrap();
    for name in ["ABC-123-CD2.mp4", "ABC-123-CD3.mp4"] {
        fs::write(input.join(name), name.as_bytes()).unwrap();
    }
    let cancel = CancellationToken::new();
    let scan = pipeline::scan(
        &[input],
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
            mode: OrganizeMode::Symlink,
            ..Default::default()
        },
    );
    let http = kiroku_core::net::Http::new(std::time::Duration::ZERO, None).unwrap();
    let mut out = pipeline::apply(plans, &http, &root, None, &cancel, &|_| {}).await;
    assert!(out.failed.is_empty(), "{:?}", out.failed);
    assert_eq!(
        kiroku_core::append::read_meta(&dir)
            .unwrap()
            .placements
            .len(),
        2
    );
    assert!(out.journal.rollback().is_empty());
    assert!(kiroku_core::append::read_meta(&dir)
        .unwrap()
        .placements
        .is_empty());
}
#[test]
#[cfg(unix)]
fn forged_manifest_cannot_trust_arbitrary_links() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("ABC-123");
    fs::create_dir_all(&dir).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"video").unwrap();
    let file = dir.join("ABC-123.mp4");
    std::os::unix::fs::symlink(src.canonicalize().unwrap(), &file).unwrap();
    let mut p = kiroku_core::placement::capture(OrganizeMode::Symlink, &src, &file, &dir).unwrap();
    p.anchor = Some(file.clone());
    let m = Metadata {
        code: "ABC-123".into(),
        placements: vec![p],
        ..Default::default()
    };
    assert!(kiroku_core::placement::resolve_media(&root, &dir, &file, &m).is_err());
}
