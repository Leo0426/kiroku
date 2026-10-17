use kiroku_core::{
    journal::{Journal, JournalStatus},
    maintenance::{self, RetentionPolicy},
    Metadata,
};
use std::{fs, sync::Mutex};
use tokio_util::sync::CancellationToken;
#[test]
fn maintenance_only_offers_unreferenced_originals_and_rolled_back_history() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let d = root.join("ABC-123");
    fs::create_dir_all(&d).unwrap();
    let mut value = serde_json::to_value(Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    })
    .unwrap();
    value["artwork"] = serde_json::json!({"original":".kiroku-original-live.png","digest":"hash","source_url":null,"crop":null,"phase":kiroku_core::badge::Phase::default()});
    fs::write(d.join("kiroku.json"), serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(d.join(".kiroku-original-live.png"), b"keep").unwrap();
    let unused = d.join(".kiroku-original-unused.png");
    fs::write(&unused, b"unused").unwrap();
    let mut j = Journal::new(&root);
    j.ts = 0;
    j.status = JournalStatus::Applied;
    j.save().unwrap();
    let policy = RetentionPolicy {
        history_days: 0,
        log_days: 0,
    };
    let p = maintenance::preview(&root, &t.path().join("logs"), &policy).unwrap();
    assert!(p
        .candidates
        .iter()
        .any(|c| c.paths.contains(&unused.canonicalize().unwrap())));
    assert!(!p.candidates.iter().any(|c| c.paths.contains(&j.path())));
    assert!(p.protected.iter().any(|s| s.contains("有效撤销")));
    let seen = Mutex::new(vec![]);
    let id = p
        .candidates
        .iter()
        .find(|c| c.kind == "original")
        .unwrap()
        .id
        .clone();
    let result = maintenance::apply(
        &p,
        &[id],
        &|path| {
            seen.lock().unwrap().push(path.to_path_buf());
            Ok(())
        },
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(result.removed, 1);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(d.join(".kiroku-original-live.png").exists());
}
#[test]
fn stale_references_and_corrupt_journals_prevent_collection() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("ABC-123");
    fs::create_dir(&d).unwrap();
    let side = d.join("kiroku.json");
    let mut m = serde_json::to_value(Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    })
    .unwrap();
    fs::write(&side, serde_json::to_vec(&m).unwrap()).unwrap();
    let original = d.join(".kiroku-original-unused.png");
    fs::write(&original, b"image").unwrap();
    let logs = t.path().join("logs");
    let policy = RetentionPolicy {
        history_days: 0,
        log_days: 0,
    };
    let p = maintenance::preview(t.path(), &logs, &policy).unwrap();
    let id = p.candidates[0].id.clone();
    m["artwork"] = serde_json::json!({"original":".kiroku-original-unused.png","digest":"hash","source_url":null,"crop":null,"phase":kiroku_core::badge::Phase::default()});
    fs::write(&side, serde_json::to_vec(&m).unwrap()).unwrap();
    let calls = Mutex::new(0);
    let result = maintenance::apply(
        &p,
        &[id],
        &|_| {
            *calls.lock().unwrap() += 1;
            Ok(())
        },
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(*calls.lock().unwrap(), 0);
    assert!(!result.skipped.is_empty());
    fs::create_dir_all(t.path().join(".kiroku")).unwrap();
    fs::write(t.path().join(".kiroku/journal-1.json"), b"broken").unwrap();
    let p = maintenance::preview(t.path(), &logs, &policy).unwrap();
    assert!(p.candidates.is_empty());
    assert!(!p.protected.is_empty());
}
#[test]
fn structurally_invalid_json_sidecar_never_proves_original_unreferenced() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("ABC-123");
    fs::create_dir(&d).unwrap();
    fs::write(d.join("kiroku.json"), b"[]").unwrap();
    fs::write(d.join(".kiroku-original-unused.png"), b"image").unwrap();
    let p = maintenance::preview(
        t.path(),
        &t.path().join("logs"),
        &RetentionPolicy {
            history_days: 0,
            log_days: 0,
        },
    )
    .unwrap();
    assert!(p.candidates.is_empty());
    assert!(!p.protected.is_empty());
}
#[test]
fn cancelled_maintenance_preview_has_no_deletion_candidates() {
    let t = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        maintenance::preview_cancellable(
            t.path(),
            &t.path().join("logs"),
            &Default::default(),
            &token
        )
        .unwrap_err()
        .kind(),
        std::io::ErrorKind::Interrupted
    );
}
#[cfg(unix)]
#[test]
fn historical_root_alias_is_normalized_without_following_descendant_links() {
    use kiroku_core::{journal::Proof, organize::Op};
    use std::{collections::BTreeMap, os::unix::fs::symlink};
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let d = root.join("ABC-123");
    fs::create_dir_all(&d).unwrap();
    let alias = t.path().join("alias");
    symlink(&root, &alias).unwrap();
    let mut j = Journal::new(&alias);
    let stage = alias
        .join("ABC-123")
        .join(format!(".kiroku-{}-0.tmp", j.id));
    fs::write(&stage, b"proof").unwrap();
    j.ts = 0;
    j.ops = vec![Op::MoveFile {
        from: t.path().join("source.mp4"),
        to: alias.join("ABC-123/video.mp4"),
    }];
    j.proofs = BTreeMap::from([(
        0,
        Proof {
            link: None, source_proof: None,
            stage: stage.clone(),
            digest: "unused-after-rollback".into(),
        },
    )]);
    j.rolled_back_ops = vec![0];
    j.status = JournalStatus::RolledBack;
    j.save().unwrap();
    let p = maintenance::preview(
        &alias,
        &t.path().join("logs"),
        &RetentionPolicy {
            history_days: 0,
            log_days: 0,
        },
    )
    .unwrap();
    assert!(p
        .candidates
        .iter()
        .any(|c| c.paths.contains(&stage.canonicalize().unwrap())));
}
#[test]
fn nfo_references_keep_original_even_without_sidecar_artwork() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("ABC-123");
    fs::create_dir(&d).unwrap();
    fs::write(
        d.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(
        d.join("movie.nfo"),
        b"<movie><num>ABC-123</num><thumb>.kiroku-original-live.png</thumb></movie>",
    )
    .unwrap();
    fs::write(d.join(".kiroku-original-live.png"), b"image").unwrap();
    let p = maintenance::preview(
        t.path(),
        &t.path().join("logs"),
        &RetentionPolicy {
            history_days: 0,
            log_days: 0,
        },
    )
    .unwrap();
    assert!(p.candidates.is_empty());
}
#[test]
fn substituted_library_cannot_authorize_deleting_identical_generated_data() {
    let t = tempfile::tempdir().unwrap();
    let root=t.path().join("lib");fs::create_dir(&root).unwrap();
    let asset=root.join(".kiroku-original-unused.png");fs::write(&asset,b"same").unwrap();
    let p=maintenance::preview(&root,&t.path().join("logs"),&RetentionPolicy{history_days:0,log_days:0}).unwrap();
    fs::rename(&root,t.path().join("old-lib")).unwrap();fs::create_dir(&root).unwrap();fs::write(&asset,b"same").unwrap();
    let calls=Mutex::new(0);
    let result=maintenance::apply(&p,&[p.candidates[0].id.clone()],&|_|{*calls.lock().unwrap()+=1;Ok(())},&CancellationToken::new());
    assert!(result.is_err() || !result.unwrap().skipped.is_empty());
    assert_eq!(*calls.lock().unwrap(),0);assert!(asset.exists());
}
#[test]
fn zero_day_manual_preview_preserves_active_log_and_active_journal() {
    let t=tempfile::tempdir().unwrap();let root=t.path().join("lib");let logs=t.path().join("logs");fs::create_dir(&root).unwrap();fs::create_dir(&logs).unwrap();
    let today=logs.join(format!("kiroku-{}.log",chrono::Local::now().format("%Y-%m-%d")));
    let old=logs.join("kiroku-2000-01-01.log");fs::write(&today,b"active").unwrap();fs::write(&old,b"old").unwrap();
    let mut active=Journal::new(&root);active.status=JournalStatus::Applied;active.save().unwrap();
    let policy=RetentionPolicy{history_days:0,log_days:0};let p=maintenance::preview(&root,&logs,&policy).unwrap();
    assert!(p.candidates.iter().any(|c|c.paths.contains(&old.canonicalize().unwrap())));
    assert!(!p.candidates.iter().any(|c|c.paths.contains(&today.canonicalize().unwrap())||c.paths.contains(&active.path().canonicalize().unwrap())));
    let selected=p.candidates.iter().map(|c|c.id.clone()).collect::<Vec<_>>();
    let result=maintenance::apply(&p,&selected,&|path|{fs::remove_file(path)},&CancellationToken::new()).unwrap();
    assert_eq!(result.removed,1);assert!(today.exists());assert!(active.path().exists());assert!(!old.exists());
}
