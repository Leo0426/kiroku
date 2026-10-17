use kiroku_core::{
    journal::Journal,
    organize::{self, PlanOpts},
    placement::{self, OrganizeMode},
    Metadata,
};
use std::{
    fs, io,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;
fn imported(root: &Path, src: &Path) -> std::path::PathBuf {
    let code = kiroku_core::code::parse_code("ABC-123.mp4").unwrap();
    let mut plan = organize::plan_with(
        src,
        &code,
        &Metadata {
            code: code.id.clone(),
            ..Default::default()
        },
        root,
        "{code}",
        true,
        PlanOpts {
            mode: OrganizeMode::Symlink,
            ..Default::default()
        },
    );
    let mut j = Journal::new(root);
    placement::bind_plan(&mut plan, &j).unwrap();
    assert!(j
        .execute(&plan, &mut |_| unreachable!(), &|| false)
        .error
        .is_none());
    root.join("ABC-123")
}
#[test]
fn source_hash_cancels_after_first_read_without_completing_file() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("video.mp4");
    fs::write(&path, vec![7; 1024 * 1024]).unwrap();
    let token = CancellationToken::new();
    let reads = AtomicUsize::new(0);
    let result = placement::source_proof_observed(&path, &token, &|done, total| {
        assert!(done < total);
        reads.fetch_add(1, Ordering::SeqCst);
        token.cancel();
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}
#[test]
fn source_path_replacement_during_hash_is_rejected_even_for_equal_bytes() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("video.mp4");
    let bytes = vec![7; 1024 * 1024];
    fs::write(&path, &bytes).unwrap();
    let changed = AtomicUsize::new(0);
    let result = placement::source_proof_observed(&path, &CancellationToken::new(), &|_, _| {
        if changed.fetch_add(1, Ordering::SeqCst) == 0 {
            fs::rename(&path, t.path().join("original.mp4")).unwrap();
            fs::write(&path, &bytes).unwrap();
        }
    });
    assert!(result.is_err());
}
#[test]
fn move_does_not_hash_or_require_source_fingerprint() {
    let token = CancellationToken::new();
    token.cancel();
    assert!(OrganizeMode::Move
        .operation_cancellable("unreadable.mp4".into(), "target.mp4".into(), &token)
        .is_ok());
    assert_eq!(
        OrganizeMode::Copy
            .operation_cancellable("unreadable.mp4".into(), "target.mp4".into(), &token)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}
#[test]
fn registered_symlink_import_allows_fresh_append_and_preserves_owned_anchor() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123-CD1.mp4");
    fs::write(&src, b"first").unwrap();
    let dir = imported(&root, &src);
    let source = t.path().join("ABC-123-CD2.mp4");
    fs::write(&source, b"second").unwrap();
    let meta = kiroku_core::append::read_meta(&dir).unwrap();
    let anchor = meta.placements[0].anchor.as_ref().unwrap();
    let mut p = kiroku_core::append::preview(&root, "ABC-123", &[source.clone()], None).unwrap();
    p.set_mode(OrganizeMode::Symlink).unwrap();
    let out = kiroku_core::append::apply(&p, &CancellationToken::new()).unwrap();
    assert!(out.failed.is_empty(), "{:?}", out.failed);
    let mut j = Journal::load(
        &root
            .join(".kiroku")
            .join(format!("journal-{}.json", out.journal_id)),
    )
    .unwrap();
    assert!(j.rollback().is_empty());
    assert!(anchor.symlink_metadata().is_ok());
    assert!(source.exists());
}
#[cfg(unix)]
#[test]
fn arbitrary_hidden_symlink_is_not_a_registered_anchor() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"first").unwrap();
    let dir = imported(&root, &src);
    std::os::unix::fs::symlink(&src, dir.join(".kiroku-spoof-0.tmp")).unwrap();
    let next = t.path().join("ABC-123-CD2.mp4");
    fs::write(&next, b"second").unwrap();
    assert!(kiroku_core::append::preview(&root, "ABC-123", &[next], None).is_err());
}
#[test]
fn health_cancel_is_not_reported_as_changed_link() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"video").unwrap();
    imported(&root, &src);
    let token = CancellationToken::new();
    let report = kiroku_core::health::scan(&root, &token, &|_| token.cancel()).unwrap();
    assert!(report.cancelled);
    assert!(!report
        .issues
        .iter()
        .any(|i| matches!(i.kind.as_str(), "changedMediaLink" | "unsafeVideo")));
}
#[cfg(unix)]
#[test]
fn replaced_manifest_anchor_is_rejected() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"first").unwrap();
    let dir = imported(&root, &src);
    let meta = kiroku_core::append::read_meta(&dir).unwrap();
    let anchor = meta.placements[0].anchor.as_ref().unwrap();
    fs::remove_file(anchor).unwrap();
    std::os::unix::fs::symlink(src.canonicalize().unwrap(), anchor).unwrap();
    let next = t.path().join("ABC-123-CD2.mp4");
    fs::write(&next, b"second").unwrap();
    assert!(kiroku_core::append::preview(&root, "ABC-123", &[next], None).is_err());
}
#[test]
fn cancelled_rebuild_preserves_existing_cache() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"video").unwrap();
    let dir = imported(&root, &src);
    let lib = kiroku_core::library::Library::open(&root).unwrap();
    lib.reindex_dir(&dir).unwrap();
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(lib.rebuild(&token, &mut |_, _| {}).unwrap(), 0);
    assert!(lib.get("ABC-123").unwrap().is_some());
    assert!(lib
        .reindex_dir_cancellable(&dir, &token)
        .unwrap_err()
        .to_string()
        .contains("取消"));
}
#[cfg(unix)]
#[test]
fn replacing_anchor_after_append_preview_stops_before_new_media_is_placed() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let src = t.path().join("ABC-123.mp4");
    fs::write(&src, b"first").unwrap();
    let dir = imported(&root, &src);
    let next = t.path().join("ABC-123-CD2.mp4");
    fs::write(&next, b"second").unwrap();
    let mut p = kiroku_core::append::preview(&root, "ABC-123", &[next.clone()], None).unwrap();
    p.set_mode(OrganizeMode::Symlink).unwrap();
    let meta = kiroku_core::append::read_meta(&dir).unwrap();
    let anchor = meta.placements[0].anchor.as_ref().unwrap();
    fs::remove_file(anchor).unwrap();
    std::os::unix::fs::symlink(src.canonicalize().unwrap(), anchor).unwrap();
    assert!(kiroku_core::append::apply(&p, &CancellationToken::new()).is_err());
    assert!(!dir.join("ABC-123-CD2.mp4").exists());
    assert!(next.exists());
}
