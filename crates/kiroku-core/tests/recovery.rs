use kiroku_core::{
    journal::{Journal, JournalStatus},
    recovery,
};
use std::fs;
#[test]
fn read_only_report_distinguishes_partial_corrupt_and_completed() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let mut completed = Journal::new(&root);
    completed.id = "100".into();
    completed.status = JournalStatus::Applied;
    completed.save().unwrap();
    let mut partial = Journal::new(&root);
    partial.id = "200".into();
    partial.status = JournalStatus::Partial;
    partial.save().unwrap();
    let bad = root.join(".kiroku/journal-300.json");
    fs::write(&bad, b"broken").unwrap();
    let before = fs::read(&bad).unwrap();
    let report = recovery::inspect(&root, &tokio_util::sync::CancellationToken::new()).unwrap();
    assert_eq!(report.entries.len(), 2);
    assert_eq!(report.entries[0].kind, "corrupt");
    assert!(!report.entries[0].can_undo);
    assert_eq!(report.entries[1].kind, "partial");
    assert!(report.entries[1].can_undo);
    assert_eq!(fs::read(&bad).unwrap(), before);
    assert_eq!(
        Journal::load(&partial.path()).unwrap().status,
        JournalStatus::Partial
    );
}
#[test]
fn absent_root_is_error_and_empty_library_is_not_created_or_modified() {
    let t = tempfile::tempdir().unwrap();
    let missing = t.path().join("offline");
    assert!(recovery::inspect(&missing, &tokio_util::sync::CancellationToken::new()).is_err());
    assert!(!missing.exists());
    let r = recovery::inspect(t.path(), &tokio_util::sync::CancellationToken::new()).unwrap();
    assert!(r.entries.is_empty());
    assert!(!t.path().join(".kiroku").exists());
}
#[cfg(unix)]
#[test]
fn linked_journal_is_untrusted_and_not_followed() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join(".kiroku")).unwrap();
    let foreign = tempfile::NamedTempFile::new().unwrap();
    fs::write(foreign.path(), b"private").unwrap();
    std::os::unix::fs::symlink(foreign.path(), t.path().join(".kiroku/journal-200.json")).unwrap();
    let r = recovery::inspect(t.path(), &tokio_util::sync::CancellationToken::new()).unwrap();
    assert_eq!(r.entries.len(), 1);
    assert_eq!(r.entries[0].kind, "corrupt");
    assert!(!r.entries[0].can_undo);
}
#[test]
fn inconsistent_log_identity_is_not_undoable() {
    let t = tempfile::tempdir().unwrap();
    let mut j = Journal::new(t.path());
    j.id = "100".into();
    j.status = JournalStatus::Partial;
    j.save().unwrap();
    fs::rename(j.path(), t.path().join(".kiroku/journal-200.json")).unwrap();
    let r = recovery::inspect(t.path(), &tokio_util::sync::CancellationToken::new()).unwrap();
    assert_eq!(r.entries[0].kind, "corrupt");
    assert!(!r.entries[0].can_undo);
}
#[test]
fn applied_with_pending_intent_is_reported() {
    use kiroku_core::{
        journal::{Pending, Proof},
        organize::Op,
    };
    let t = tempfile::tempdir().unwrap();
    let mut j = Journal::new(t.path());
    j.id = "200".into();
    j.status = JournalStatus::Applied;
    j.pending = Some(Pending {
        op: Op::WriteFile {
            path: t.path().join("movie.nfo"),
            content: "pending".into(),
        },
        proof: Proof {
            stage: t.path().join(".kiroku-200-0.tmp"),
            digest: "unused".into(),
            link: None,
            source_proof: None, published_identity: None,
        },
    });
    j.save().unwrap();
    let r = recovery::inspect(t.path(), &tokio_util::sync::CancellationToken::new()).unwrap();
    assert_eq!(r.entries[0].kind, "pending");
    assert_eq!(r.entries[0].pending_operations, 1);
    assert!(!t.path().join("movie.nfo").exists());
}
#[test]
fn offline_history_does_not_claim_empty() {
    let t = tempfile::tempdir().unwrap();
    let missing = t.path().join("offline");
    assert!(Journal::inspect(&missing).is_err());
    assert!(Journal::inspect(t.path()).unwrap().is_empty());
    assert!(!missing.exists());
}

#[test]
fn preserve_recovery_commits_pending_without_undoing_completed_work() {
    use kiroku_core::{journal::Checkpoint, organize::{Op, Plan}};
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("library");
    fs::create_dir(&root).unwrap();
    let source = t.path().join("ABC-001.mp4");
    fs::write(&source, b"neutral video").unwrap();
    let target = root.join("ABC-001.mp4");
    let mut journal = Journal::new(&root);
    let plan = Plan { ops: vec![Op::MoveFile { from: source.clone(), to: target.clone() }] };
    let result = journal.execute_observed(&plan, &mut |_| unreachable!(), &||false, &mut |point| {
        if point == Checkpoint::SourceRemoved { Err(std::io::Error::other("simulated interruption")) } else { Ok(()) }
    });
    assert!(result.error.is_some());
    let cancel = tokio_util::sync::CancellationToken::new();
    let report = recovery::inspect(&root, &cancel).unwrap();
    assert!(report.entries[0].can_preserve);
    let result = recovery::preserve(&root, &journal.id, &cancel).unwrap();
    assert!(result.recovery_error.is_none(), "{:?}", result.recovery_error);
    assert!(result.index_errors.is_empty(), "{:?}", result.index_errors);
    assert_eq!(result.recorded_operations, 1);
    assert_eq!(result.index_total, 1);
    assert_eq!(result.index_completed, 1);
    assert_eq!(fs::read(&target).unwrap(), b"neutral video");
    assert!(!source.exists());
    let mut saved = Journal::load(&journal.path()).unwrap();
    assert!(saved.pending.is_none());
    assert_eq!(saved.status, JournalStatus::Partial);
    assert!(saved.rollback().is_empty());
    assert_eq!(fs::read(source).unwrap(), b"neutral video");
}

#[test]
fn preserve_rejects_revision_undo_progress_and_invalid_ids_without_writes() {
    let t = tempfile::tempdir().unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    for (version, rolled_back) in [(3, vec![]), (1, vec![]), (2, vec![0])] {
        let mut j = Journal::new(t.path());
        j.version = version;
        j.rolled_back_ops = rolled_back;
        j.status = JournalStatus::Partial;
        j.save().unwrap();
        let bytes = fs::read(j.path()).unwrap();
        assert!(recovery::preserve(t.path(), &j.id, &cancel).is_err());
        assert_eq!(fs::read(j.path()).unwrap(), bytes);
        fs::remove_file(j.path()).unwrap();
    }
    assert!(recovery::preserve(t.path(), "../foreign", &cancel).is_err());
    cancel.cancel();
    assert!(recovery::preserve(t.path(), "200", &cancel).is_err());
}
