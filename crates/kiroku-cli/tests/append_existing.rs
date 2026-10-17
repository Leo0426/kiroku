use kiroku_core::{library::Library, settings::Settings, Metadata};
use std::{fs, process::Command};
#[test]
fn normal_scrape_command_reuses_disk_metadata_and_indexes_appended_part() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let d = root.join("manual-name");
    let source = t.path().join("incoming");
    fs::create_dir_all(&d).unwrap();
    fs::create_dir(&source).unwrap();
    let side = serde_json::to_vec(&Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    })
    .unwrap();
    fs::write(d.join("kiroku.json"), &side).unwrap();
    fs::write(d.join("ABC-123-CD1.mp4"), b"one").unwrap();
    let lib = Library::open(&root).unwrap();
    lib.reindex_dir(&d).unwrap();
    drop(lib);
    fs::write(source.join("ABC-123-CD2.mp4"), b"two").unwrap();
    let mut settings = Settings {
        cleanup_source: false,
        ..Default::default()
    };
    for s in &mut settings.sources {
        s.enabled = false
    }
    let config = t.path().join("settings.json");
    settings.save(&config).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_kiroku"))
        .args([
            "scrape",
            source.to_str().unwrap(),
            "--out",
            root.to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--apply",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read(d.join("kiroku.json")).unwrap(), side);
    assert_eq!(
        Library::open(&root)
            .unwrap()
            .get("ABC-123")
            .unwrap()
            .unwrap()
            .files
            .len(),
        2
    );
}
