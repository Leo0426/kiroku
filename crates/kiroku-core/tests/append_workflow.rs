use kiroku_core::{append, journal::Journal, library::Library, Metadata};
use std::fs;
use tokio_util::sync::CancellationToken;
#[test]
fn append_preserves_metadata_and_undo_only_new_files() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("manual-name");
    fs::create_dir_all(&dir).unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    let side = serde_json::to_vec(&m).unwrap();
    fs::write(dir.join("kiroku.json"), &side).unwrap();
    fs::write(dir.join("movie.nfo"), b"hand edited").unwrap();
    fs::write(dir.join("ABC-123-CD1.mp4"), b"one").unwrap();
    let input = t.path().join("ABC-123-CD2.mp4");
    fs::write(&input, b"two").unwrap();
    fs::write(t.path().join("ABC-123-CD2.chs.srt"), b"subtitle").unwrap();
    let p = append::preview(&root, "ABC-123", &[input.clone()], None).unwrap();
    assert!(p.conflicts.is_empty());
    assert_eq!(p.ops.len(), 2);
    let result = append::apply(&p, &CancellationToken::new()).unwrap();
    assert!(result.failed.is_empty());
    assert_eq!(result.completed, 2);
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), side);
    assert_eq!(fs::read(dir.join("movie.nfo")).unwrap(), b"hand edited");
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
    let mut j = Journal::load(
        &root
            .join(".kiroku")
            .join(format!("journal-{}.json", result.journal_id)),
    )
    .unwrap();
    assert!(j.rollback().is_empty());
    assert!(input.exists());
    assert!(dir.join("ABC-123-CD1.mp4").exists());
    assert!(!dir.join("ABC-123-CD2.mp4").exists());
}
#[test]
fn append_rejects_stale_and_ambiguous_targets() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("a");
    fs::create_dir_all(&dir).unwrap();
    let bytes = serde_json::to_vec(&Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    })
    .unwrap();
    fs::write(dir.join("kiroku.json"), &bytes).unwrap();
    fs::write(dir.join("ABC-123.mp4"), b"one").unwrap();
    let input = t.path().join("ABC-123-CD2.mp4");
    fs::write(&input, b"two").unwrap();
    let p = append::preview(&root, "ABC-123", &[input.clone()], None).unwrap();
    fs::write(dir.join("kiroku.json"), b"changed").unwrap();
    assert!(append::apply(&p, &CancellationToken::new()).is_err());
    assert!(input.exists());
    fs::write(dir.join("kiroku.json"), &bytes).unwrap();
    let d = root.join("b");
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("kiroku.json"), &bytes).unwrap();
    assert!(append::preview(&root, "ABC-123", &[input], None).is_err());
}
#[test]
fn identical_duplicate_keeps_source_and_subtitle_only_matches_part() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("a");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(dir.join("ABC-123-CD1.mp4"), b"one").unwrap();
    let video = t.path().join("ABC-123-CD1.mp4");
    fs::write(&video, b"one").unwrap();
    let p = append::preview(&root, "ABC-123", &[video.clone()], None).unwrap();
    assert!(p.ops.is_empty());
    assert_eq!(p.skipped.len(), 1);
    assert!(video.exists());
    let sub = t.path().join("ABC-123-CD1.chs.srt");
    fs::write(&sub, b"sub").unwrap();
    let p = append::preview(&root, "ABC-123", &[sub], None).unwrap();
    assert_eq!(p.ops.len(), 1);
    assert!(p.conflicts.is_empty());
}
#[test]
fn pipeline_existing_movie_plans_only_new_video() {
    use kiroku_core::{code::parse_code, organize::PlanOpts, pipeline};
    use std::collections::HashMap;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("manual");
    fs::create_dir_all(&dir).unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    fs::write(dir.join("ABC-123-CD1.mp4"), b"one").unwrap();
    let input = t.path().join("ABC-123-CD2.mp4");
    fs::write(&input, b"two").unwrap();
    let scan = pipeline::scan(
        &[t.path().to_path_buf()],
        Some(&root),
        &Default::default(),
        &CancellationToken::new(),
        &mut |_| {},
    );
    let item = &scan.items[0];
    let c = parse_code("ABC-123-CD2.mp4").unwrap();
    let plans = pipeline::build_plans(
        &[(item, &c)],
        &HashMap::from([(m.code.clone(), m)]),
        &root,
        "{code}",
        PlanOpts::default(),
    );
    assert!(plans[0].selected);
    assert_eq!(
        plans[0].to,
        dir.canonicalize().unwrap().join("ABC-123-CD2.mp4")
    );
    assert_eq!(plans[0].ops.len(), 1);
}
#[cfg(unix)]
#[test]
fn preview_rejects_same_bytes_replaced_file_identity() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("a");
    fs::create_dir_all(&dir).unwrap();
    let side = dir.join("kiroku.json");
    let bytes = serde_json::to_vec(&Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    })
    .unwrap();
    fs::write(&side, &bytes).unwrap();
    fs::write(dir.join("ABC-123-CD1.mp4"), b"one").unwrap();
    let input = t.path().join("ABC-123-CD2.mp4");
    fs::write(&input, b"two").unwrap();
    let p = append::preview(&root, "ABC-123", &[input.clone()], None).unwrap();
    fs::rename(&side, dir.join("backup")).unwrap();
    fs::write(&side, &bytes).unwrap();
    fs::remove_file(dir.join("backup")).unwrap();
    assert!(p.validate_files().is_err());
}
#[test]
fn anonymous_subtitle_requires_explicit_existing_video() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = root.join("a");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    let video = dir.join("ABC-123-CD1.mp4");
    fs::write(&video, b"one").unwrap();
    let sub = t.path().join("subtitle.chs.srt");
    fs::write(&sub, b"subtitle").unwrap();
    assert!(append::preview(&root, "ABC-123", &[sub.clone()], None).is_err());
    let p = append::preview_with_video(&root, "ABC-123", &[sub], None, Some(&video)).unwrap();
    assert_eq!(
        p.ops[0].target().file_name().unwrap(),
        "ABC-123-CD1.chs.srt"
    );
}
#[tokio::test]
async fn cached_existing_movie_is_reported_found_without_network() {
    use kiroku_core::{
        parse_code,
        pipeline::{scrape_missing, ScrapeStatus, Scraper},
        settings::Settings,
    };
    use std::{collections::HashMap, sync::Mutex};
    let sc = Scraper::new(Settings::default()).unwrap();
    let c = parse_code("ABC-123").unwrap();
    let m = Metadata {
        code: c.id.clone(),
        ..Default::default()
    };
    let events = Mutex::new(vec![]);
    let result = scrape_missing(
        &sc,
        &[c],
        &HashMap::from([(m.code.clone(), m)]),
        &CancellationToken::new(),
        &|e| events.lock().unwrap().push(e),
    )
    .await;
    assert!(result.is_empty());
    let ev = events.into_inner().unwrap();
    assert_eq!(ev.len(), 1);
    assert!(matches!(ev[0].status, ScrapeStatus::Found));
}
#[test]
fn append_preserves_existing_plex_filename_pattern() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let d = root.join("a");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(d.join("ABC-123 Original Title - cd1.mp4"), b"one").unwrap();
    let input = t.path().join("ABC-123-CD2.mp4");
    fs::write(&input, b"two").unwrap();
    let p = append::preview(&root, "ABC-123", &[input], None).unwrap();
    assert_eq!(
        p.ops[0].target().file_name().unwrap(),
        "ABC-123 Original Title - cd2.mp4"
    );
}
#[test]
fn selected_idx_also_previews_sub_companion_and_cancel_moves_nothing() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let d = root.join("ABC-123");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    let video = d.join("ABC-123-CD1.mp4");
    fs::write(&video, b"one").unwrap();
    let idx = t.path().join("captions.idx");
    fs::write(&idx, b"index").unwrap();
    fs::write(t.path().join("captions.sub"), b"subtitle").unwrap();
    let p =
        append::preview_with_video(&root, "ABC-123", &[idx.clone()], None, Some(&video)).unwrap();
    assert_eq!(p.ops.len(), 2);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let r = append::apply(&p, &cancel).unwrap();
    assert!(r.cancelled);
    assert_eq!(r.completed, 0);
    assert!(idx.exists());
}
#[test]
fn cancelled_preview_never_produces_applicable_plan() {
    let t = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        append::preview_with_video_cancellable(t.path(), "ABC-123", &[], None, None, &token)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
}
#[cfg(unix)]
#[test]
fn replacing_target_directory_with_internal_symlink_rejects_apply() {
    use std::os::unix::fs::symlink;
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let d = root.join("ABC-123");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(d.join("ABC-123-CD1.mp4"), b"one").unwrap();
    let input = t.path().join("ABC-123-CD2.mp4");
    fs::write(&input, b"two").unwrap();
    let p = append::preview(&root, "ABC-123", &[input.clone()], None).unwrap();
    let moved = root.join("moved");
    fs::rename(&d, &moved).unwrap();
    symlink(&moved, &d).unwrap();
    assert!(append::apply(&p, &CancellationToken::new()).is_err());
    assert!(input.exists());
}
#[test]
fn fc2_ppv_alias_finds_existing_movie_without_dropping_leading_zeroes() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let d = root.join("FC2-00123");
    fs::create_dir_all(&d).unwrap();
    fs::write(
        d.join("kiroku.json"),
        serde_json::to_vec(&Metadata {
            code: "FC2-00123".into(),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    fs::write(d.join("FC2-00123-CD1.mp4"), b"one").unwrap();
    let input = t.path().join("FC2-PPV-00123-CD2.mp4");
    fs::write(&input, b"two").unwrap();
    let p = append::preview(&root, "FC2-PPV-00123", &[input], None).unwrap();
    assert_eq!(p.code, "FC2-00123");
    assert!(p.conflicts.is_empty());
    assert_eq!(p.ops[0].target().file_name().unwrap(), "FC2-00123-CD2.mp4");
    assert!(append::targets(&root, "FC2-123").unwrap().is_empty());
}
