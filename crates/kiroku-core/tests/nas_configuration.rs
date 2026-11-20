//! Isolated process: configuration is intentionally application-global.
use kiroku_core::{
    journal::Journal,
    nas_ops,
    organize::{Op, Plan},
};
use std::fs;

#[test]
fn invalid_opt_in_is_distinct_from_missing_and_prevents_local_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let settings = temp.path().join("settings.json");
    nas_ops::configure_file(&settings).unwrap();
    assert_eq!(nas_ops::status().state, "disabled");
    assert_eq!(nas_ops::diagnose(&|| false).unwrap().state, "disabled");
    let config = settings.with_file_name("nas-operations.json");
    fs::write(&config, b"not json").unwrap();
    assert!(nas_ops::configure_file(&settings).is_err());
    assert_eq!(nas_ops::status().state, "invalid");
    assert!(nas_ops::diagnose(&|| false).is_err());
    let source = temp.path().join("ABC-001.mp4");
    fs::write(&source, b"neutral video").unwrap();
    let root = temp.path().join("library");
    fs::create_dir(&root).unwrap();
    let target = root.join("ABC-001.mp4");
    let mut journal = Journal::new(&root);
    let outcome = journal.execute(
        &Plan {
            ops: vec![Op::MoveFile {
                from: source.clone(),
                to: target.clone(),
            }],
        },
        &mut |_| unreachable!(),
        &|| false,
    );
    assert!(outcome.error.is_some());
    assert!(!target.exists());
    assert!(!journal.path().exists());
    assert_eq!(fs::read(&source).unwrap(), b"neutral video");
    fs::remove_file(config).unwrap();
    nas_ops::configure_file(&settings).unwrap();
    assert_eq!(nas_ops::status().state, "disabled");
    assert!(nas_ops::ensure_writes_allowed().is_ok());
    assert!(nas_ops::configure(Some(nas_ops::Config {
        host: "-unsafe".into(),
        local_root: root.clone(),
        remote_root: "/media".into()
    }))
    .is_err());
    assert_eq!(nas_ops::status().state, "invalid");
    nas_ops::configure(Some(nas_ops::Config {
        host: "neutral-host".into(),
        local_root: root,
        remote_root: "/media".into(),
    }))
    .unwrap();
    assert_eq!(nas_ops::status().state, "unverified");
    // Cancellation before a probe must not invoke SSH or create files.
    assert_eq!(
        nas_ops::diagnose(&|| true).unwrap_err().kind(),
        std::io::ErrorKind::Interrupted
    );
    nas_ops::configure(None).unwrap();
}
