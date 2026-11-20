use kiroku_core::{journal::{Journal, JournalItem, JournalStatus}, library::Library, recovery};
use std::{fs, path::Path, time::{Duration, Instant}};
use tokio_util::sync::CancellationToken;

fn fixture(root: &Path) -> Journal {
    let mut journal = Journal::new(root);
    journal.status = JournalStatus::Partial;
    for code in ["TEST-001", "TEST-002"] {
        let dir = root.join(code);
        fs::create_dir_all(dir.join("extrafanart")).unwrap();
        fs::write(dir.join(format!("{code}.mp4")), b"neutral video").unwrap();
        fs::write(dir.join("movie.nfo"), format!("<movie><num>{code}</num><title>Neutral example</title></movie>")).unwrap();
        journal.items.push(JournalItem { code: code.into(), dir: dir.clone() });
        journal.ops.push(kiroku_core::organize::Op::WriteFile { path: dir.join("extrafanart/image.jpg"), content: String::new() });
    }
    journal.save().unwrap();
    journal
}

#[test]
fn locked_index_stops_batch_without_repeating_timeout_for_each_directory() {
    let temp = tempfile::tempdir().unwrap();
    let journal = fixture(temp.path());
    let writer = Library::open(temp.path()).unwrap();
    let blocker = rusqlite::Connection::open(Library::db_path(temp.path())).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    let result = recovery::preserve(temp.path(), &journal.id, &CancellationToken::new()).unwrap();
    blocker.execute_batch("ROLLBACK").unwrap();
    assert!(started.elapsed() < Duration::from_secs(2), "lock wait multiplied across batch: {:?}", started.elapsed());
    assert_eq!(result.index_errors.len(), 1);
    assert!(result.recovery_error.is_none());
    assert_eq!(Journal::load(&journal.path()).unwrap().status, JournalStatus::Partial);
    drop(writer);
}

#[test]
#[ignore = "requires explicit KIROKU_NAS_TEST_ROOT; only owned neutral temporary files"]
fn nas_preserve_with_desktop_connection_alive() {
    let parent = std::env::var_os("KIROKU_NAS_TEST_ROOT").unwrap();
    let temp = tempfile::Builder::new().prefix("kiroku-recovery-lock-").tempdir_in(parent).unwrap();
    let journal = fixture(temp.path());
    let desktop = Library::open(temp.path()).unwrap();
    let reader = desktop.reader_handle().open().unwrap();
    assert_eq!(reader.list(&Default::default()).unwrap().total, 0);
    let result = recovery::preserve_with_index(temp.path(), &journal.id, &CancellationToken::new(), Some(&desktop), &mut |_| {}).unwrap();
    assert!(result.index_errors.is_empty(), "{:?}", result.index_errors);
    assert_eq!(reader.list(&Default::default()).unwrap().total, 2);
}

#[test]
#[ignore = "requires explicit KIROKU_NAS_TEST_ROOT"]
fn nas_preserve_after_reader_closed() {
    let parent = std::env::var_os("KIROKU_NAS_TEST_ROOT").unwrap();
    let temp = tempfile::Builder::new().prefix("kiroku-recovery-closed-").tempdir_in(parent).unwrap();
    let journal = fixture(temp.path());
    let desktop = Library::open(temp.path()).unwrap();
    {
        let reader = desktop.reader_handle().open().unwrap();
        assert_eq!(reader.list(&Default::default()).unwrap().total, 0);
    }
    let result = recovery::preserve_with_index(temp.path(), &journal.id, &CancellationToken::new(), Some(&desktop), &mut |_| {}).unwrap();
    assert!(result.index_errors.is_empty(), "{:?}", result.index_errors);
    assert_eq!(desktop.list(&Default::default()).unwrap().total, 2);
}

#[test]
fn artwork_subdirectories_are_not_indexed_and_cancellation_is_retryable() {
    let temp = tempfile::tempdir().unwrap();
    let journal = fixture(temp.path());
    let writer = Library::open(temp.path()).unwrap();
    let cancel = CancellationToken::new();
    let result = recovery::preserve_with_index(temp.path(), &journal.id, &cancel, Some(&writer), &mut |p| {
        if p.stage == "index" && p.done == 1 { cancel.cancel(); }
    }).unwrap();
    assert_eq!(result.index_total, 2);
    assert_eq!(result.index_completed, 1);
    assert_eq!(result.index_pending, 1);
    assert!(result.cancelled);
    assert_eq!(writer.list(&Default::default()).unwrap().total, 1);
    let retry = recovery::preserve_with_index(temp.path(), &journal.id, &CancellationToken::new(), Some(&writer), &mut |_| {}).unwrap();
    assert_eq!(retry.index_completed, 2);
    assert_eq!(retry.index_pending, 0);
    assert!(retry.index_errors.is_empty());
    assert_eq!(writer.list(&Default::default()).unwrap().total, 2);
}

#[test]
fn completed_directory_not_in_items_still_gets_indexed() {
    let temp = tempfile::tempdir().unwrap();
    let mut journal = fixture(temp.path());
    let additional = temp.path().join("TEST-003");
    fs::create_dir(&additional).unwrap();
    fs::write(additional.join("TEST-003.mp4"), b"neutral video").unwrap();
    fs::write(additional.join("movie.nfo"), "<movie><num>TEST-003</num><title>Neutral</title></movie>").unwrap();
    journal.ops.push(kiroku_core::organize::Op::WriteFile { path: additional.join("movie.nfo"), content: String::new() });
    journal.save().unwrap();
    let result = recovery::preserve(temp.path(), &journal.id, &CancellationToken::new()).unwrap();
    assert_eq!(result.index_completed, 3);
    assert!(result.index_errors.is_empty());
}

#[test]
#[ignore = "requires explicit KIROKU_NAS_TEST_ROOT"]
fn nas_legacy_index_migrates_without_modifying_shared_database() {
    let parent = std::env::var_os("KIROKU_NAS_TEST_ROOT").unwrap();
    let nas = tempfile::Builder::new().prefix("kiroku-legacy-index-").tempdir_in(parent).unwrap();
    let local = tempfile::tempdir().unwrap();
    let original = fixture(local.path());
    {
        let library = Library::open(local.path()).unwrap();
        for item in &original.items { library.reindex_dir(&item.dir).unwrap(); }
    }
    // Legacy SMB indexes use rollback journaling; a local WAL header would
    // request unsupported network shared memory before backup can read it.
    { let legacy_format = rusqlite::Connection::open(Library::db_path(local.path())).unwrap();
      legacy_format.execute_batch("PRAGMA journal_mode=DELETE").unwrap(); }
    fs::create_dir(nas.path().join(".kiroku")).unwrap();
    let legacy = nas.path().join(".kiroku/library.db");
    fs::copy(Library::db_path(local.path()), &legacy).unwrap();
    let bytes = fs::read(&legacy).unwrap();
    let library = Library::open(nas.path()).unwrap();
    assert!(!Library::db_path(nas.path()).starts_with(nas.path()));
    assert_eq!(library.list(&Default::default()).unwrap().total, 2);
    let conn = rusqlite::Connection::open(Library::db_path(nas.path())).unwrap();
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(mode, "wal");
    assert_eq!(fs::read(legacy).unwrap(), bytes);
}

#[test]
#[ignore = "requires explicit KIROKU_NAS_TEST_ROOT"]
fn nas_sixty_six_movies_reconcile_while_desktop_reader_remains_open() {
    let parent = std::env::var_os("KIROKU_NAS_TEST_ROOT").unwrap();
    let temp = tempfile::Builder::new().prefix("kiroku-batch-recovery-").tempdir_in(parent).unwrap();
    let mut journal = Journal::new(temp.path());
    journal.status = JournalStatus::Partial;
    for number in 1..=66 {
        let code = format!("TEST-{number:03}");
        let dir = temp.path().join(&code);
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join(format!("{code}.mp4")), b"neutral video").unwrap();
        fs::write(dir.join("movie.nfo"), format!("<movie><num>{code}</num><title>Neutral example</title></movie>")).unwrap();
        journal.items.push(JournalItem { code, dir: dir.clone() });
        if number <= 61 {
            fs::create_dir(dir.join("extrafanart")).unwrap();
            journal.ops.push(kiroku_core::organize::Op::WriteFile {path:dir.join("extrafanart/image.jpg"),content:String::new()});
        }
    }
    journal.save().unwrap();
    let desktop = Library::open(temp.path()).unwrap();
    let reader = desktop.reader_handle().open().unwrap();
    assert_eq!(reader.list(&Default::default()).unwrap().total, 0);
    let started = Instant::now();
    let result = recovery::preserve_with_index(temp.path(), &journal.id, &CancellationToken::new(), Some(&desktop), &mut |p| {
        if p.stage == "index" && p.done > 0 { assert!(reader.list(&Default::default()).unwrap().total <= 66); }
    }).unwrap();
    println!("66 movie directories / 61 artwork subdirectories recovery: {:?}",started.elapsed());
    assert!(result.index_errors.is_empty(), "{:?}", result.index_errors);
    assert_eq!(result.index_total, 66);
    assert_eq!(result.index_completed, 66);
    assert_eq!(reader.list(&Default::default()).unwrap().total, 66);
}
