use kiroku_core::{health, library::Library, Metadata};
use std::fs;
use tokio_util::sync::CancellationToken;
#[test]
fn health_is_read_only_and_reports_metadata_artwork_and_cache_issues() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("ABC-123");
    fs::create_dir_all(&dir).unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    fs::write(dir.join("movie.nfo"), b"not XML").unwrap();
    fs::write(dir.join("ABC-123.mp4"), b"video").unwrap();
    let l = Library::open(&root).unwrap();
    l.reindex_dir(&dir).unwrap();
    drop(l);
    fs::remove_file(dir.join("ABC-123.mp4")).unwrap();
    let before = fs::read(Library::db_path(&root)).unwrap();
    let r = health::scan(&root, &CancellationToken::new(), &|_| {}).unwrap();
    let kinds = r.issues.iter().map(|i| i.kind.as_str()).collect::<Vec<_>>();
    assert!(kinds.contains(&"missingVideo"));
    assert!(kinds.contains(&"invalidNfo"));
    assert!(kinds.contains(&"missingArtwork"));
    assert!(kinds.contains(&"staleIndex"));
    assert_eq!(before, fs::read(Library::db_path(&root)).unwrap());
}
#[test]
fn missing_root_is_not_healthy_and_cancelled_report_is_partial() {
    let t = tempfile::tempdir().unwrap();
    assert!(health::scan(
        &t.path().join("missing"),
        &CancellationToken::new(),
        &|_| {}
    )
    .is_err());
    let token = CancellationToken::new();
    token.cancel();
    let r = health::scan(t.path(), &token, &|_| {}).unwrap();
    assert!(r.cancelled);
    assert_eq!(r.scanned, 0);
}
#[test]
fn invalid_sidecar_and_escaping_original_are_reported() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("ABC-123");
    fs::create_dir(&d).unwrap();
    fs::write(d.join("kiroku.json"), b"garbage").unwrap();
    fs::write(d.join("ABC-123.mp4"), b"video").unwrap();
    let r = health::scan(t.path(), &CancellationToken::new(), &|_| {}).unwrap();
    assert!(r.issues.iter().any(|i| i.kind == "invalidSidecar"));
    let m = serde_json::json!({"code":"ABC-123","fields":{},"artwork":{"original":"../../outside.png"}});
    fs::write(d.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    let r = health::scan(t.path(), &CancellationToken::new(), &|_| {}).unwrap();
    assert!(r.issues.iter().any(|i| i.kind == "invalidOriginal"));
}
