use kiroku_core::{
    batch_edit::BatchEdit,
    library::{Library, Query},
    revision, Metadata,
};
use std::fs;

#[test]
fn batch_candidates_use_disk_previews_reindex_and_restore_exact_bytes() {
    let root = tempfile::tempdir().unwrap();
    let library = Library::open(root.path()).unwrap();
    for code in ["SMPL-001", "SMPL-002"] {
        let dir = root.path().join(code);
        fs::create_dir(&dir).unwrap();
        let mut meta = Metadata {
            code: code.into(),
            ..Default::default()
        };
        meta.fields.tags = vec!["原标签".into()];
        let original = serde_json::to_vec(&meta).unwrap();
        fs::write(dir.join("kiroku.json"), &original).unwrap();
        fs::write(
            dir.join("movie.nfo"),
            b"<movie><title>Example</title></movie>",
        )
        .unwrap();
        fs::write(dir.join(format!("{code}.mp4")), b"video").unwrap();
        library.reindex_dir(&dir).unwrap();
        let before = revision::capture(root.path(), &dir, code).unwrap();
        let tags = BatchEdit::Tags {
            add: vec!["新增标签".into()],
            remove: vec!["原标签".into()],
        }
        .apply_to(&before.before)
        .unwrap();
        let favorite = BatchEdit::Favorite { favorite: true }
            .apply_to(&tags)
            .unwrap();
        let preview = before.prepare_after(favorite).unwrap();
        assert_eq!(
            fs::read(dir.join("kiroku.json")).unwrap(),
            original,
            "preview never writes"
        );
        let mut outcome = revision::apply(&preview).unwrap();
        assert!(outcome.error.is_none());
        library.reindex_dir(&dir).unwrap();
        assert!(library.get(code).unwrap().unwrap().meta.favorite);
        assert!(
            revision::apply(&preview).is_err(),
            "stale previews cannot overwrite a saved edit"
        );
        assert!(outcome.journal.rollback().is_empty());
        library.reindex_dir(&dir).unwrap();
        assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), original);
    }
    assert_eq!(library.list(&Query::default()).unwrap().total, 2);
    assert_eq!(library.overview(&Query::default()).unwrap().favorites, 0);
}
