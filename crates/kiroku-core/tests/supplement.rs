use kiroku_core::{
    journal::Journal,
    pipeline::{self, LocalImport},
    settings::Settings,
    supplement::{self, Kind, Status},
    Metadata,
};
use std::{fs, io::Cursor};
use tokio_util::sync::CancellationToken;

fn png() -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(14, 22)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[test]
fn supplements_selected_companions_from_scan_then_undo_preserves_both_videos_and_existing_art() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let target = root.join("movie");
    let source = temp.path().join("source");
    fs::create_dir_all(source.join(".actors")).unwrap();
    fs::create_dir_all(&target).unwrap();
    let video = target.join("TEST-001.mp4");
    fs::write(&video, b"library video").unwrap();
    fs::write(source.join("TEST-001.mkv"), b"source video").unwrap();
    fs::write(source.join("TEST-001.nfo"),"<movie><num>TEST-001</num><title>Local</title><custom>retained</custom><actor><name>Example</name></actor></movie>").unwrap();
    fs::write(source.join("poster.jpg"), png()).unwrap();
    fs::write(source.join(".actors/Example.png"), png()).unwrap();
    fs::write(target.join("fanart.webp"), b"keep existing bytes").unwrap();
    let cancel = CancellationToken::new();
    let mut settings = Settings::default();
    settings.scan.min_video_mb = 0;
    let items = pipeline::scan(
        std::slice::from_ref(&source),
        Some(&root),
        &settings.video_filter(),
        &cancel,
        &mut |_| {},
    )
    .items;
    let (_, metas) = pipeline::inspect_import(&items, &cancel);
    let meta = metas.get("TEST-001").unwrap();
    let plan = supplement::preview(
        &root,
        &target,
        "TEST-001",
        &video,
        Some(meta),
        LocalImport::beside(&items[0], &items, &cancel),
        &settings,
    )
    .unwrap();
    assert!(plan.entries.iter().any(|e| e.kind == Kind::Fanart
        && e.status == Status::Existing
        && e.path.as_ref().unwrap().ends_with("fanart.webp")));
    let poster_path = plan
        .entries
        .iter()
        .find(|e| e.kind == Kind::Poster)
        .unwrap()
        .path
        .clone()
        .unwrap();
    assert!(!poster_path.exists() && !video.with_extension("nfo").exists());
    let outcome = supplement::apply(
        plan,
        &[Kind::Nfo, Kind::Poster, Kind::Fanart, Kind::Actors],
        &settings,
        &cancel,
        &mut |_, _| panic!("local sources suffice"),
    )
    .unwrap();
    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert_eq!(outcome.written.len(), 3);
    assert!(fs::read_to_string(video.with_extension("nfo"))
        .unwrap()
        .contains("<custom>retained</custom>"));
    assert!(fs::read(&poster_path).unwrap().starts_with(&[0xff, 0xd8]));
    assert!(supplement::read_local_avatar(&target, "Example")
        .unwrap()
        .unwrap()
        .starts_with(b"\x89PNG"));
    let id = outcome.journal_id.unwrap();
    let mut journal =
        Journal::load(&root.join(".kiroku").join(format!("journal-{id}.json"))).unwrap();
    assert!(journal.ops.iter().all(|op| op.source().is_none()));
    assert!(journal.rollback().is_empty());
    assert!(
        !video.with_extension("nfo").exists()
            && !poster_path.exists()
            && !target.join(".actors/Example.png").exists()
    );
    assert_eq!(fs::read(&video).unwrap(), b"library video");
    assert_eq!(
        fs::read(source.join("TEST-001.mkv")).unwrap(),
        b"source video"
    );
    assert_eq!(
        fs::read(target.join("fanart.webp")).unwrap(),
        b"keep existing bytes"
    );
    assert!(source.join(".actors/Example.png").exists());
}

#[test]
fn late_files_stay_untouched_and_no_image_source_is_reported_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let dir = root.join("movie");
    let video = dir.join("TEST-002.mkv");
    fs::create_dir(&dir).unwrap();
    fs::write(&video, b"kept").unwrap();
    let mut meta = Metadata {
        code: "TEST-002".into(),
        ..Default::default()
    };
    meta.fields.title = Some("Example".into());
    meta.fields.poster_urls = vec!["https://example.invalid/poster".into()];
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    let cfg = Settings::default();
    let cancel = CancellationToken::new();
    let plan = supplement::preview(root, &dir, &meta.code, &video, None, None, &cfg).unwrap();
    assert!(plan
        .entries
        .iter()
        .any(|e| e.kind == Kind::Actors && e.status == Status::Unavailable));
    assert!(plan
        .entries
        .iter()
        .any(|e| e.kind == Kind::Fanart && e.status == Status::Unavailable));
    fs::write(dir.join("poster.webp"), b"keep late file").unwrap();
    let out = supplement::apply(plan, &[Kind::Poster], &cfg, &cancel, &mut |_, _| {
        panic!("skip existing")
    })
    .unwrap();
    assert!(out.journal_id.is_none() && out.written.is_empty());
    assert_eq!(out.skipped.len(), 1);
    assert!(!video.with_extension("nfo").exists());
    assert_eq!(
        fs::read(dir.join("poster.webp")).unwrap(),
        b"keep late file"
    );
    assert!(supplement::preview(root, &dir, "TEST-003", &video, None, None, &cfg).is_err());
}
