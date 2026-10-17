use kiroku_core::{code::parse_code, metadata::Metadata, organize::{self, Op}, pipeline, library::Library};
use std::{collections::HashMap, fs};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn named_companions_import_index_edit_and_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("library");
    let input = temp.path().join("ABC-123-C.mp4");
    fs::create_dir(&root).unwrap();
    fs::write(&input, b"video").unwrap();
    let code = parse_code("ABC-123-C.mp4").unwrap();
    let meta = Metadata { code: code.id.clone(), ..Default::default() };
    let plan = organize::plan(&input, &code, &meta, &root, "{code}");
    assert!(plan.ops.iter().any(|op| matches!(op, Op::WriteFile { path, .. } if path.ends_with("ABC-123-C.nfo"))));
    let scan = pipeline::scan(&[temp.path().into()], Some(&root), &Default::default(), &CancellationToken::new(), &mut |_| {});
    let pairs = scan.items.iter().map(|i| (i, i.code.as_ref().unwrap())).collect::<Vec<_>>();
    let plans = pipeline::build_plans(&pairs, &HashMap::from([(code.id.clone(), meta)]), &root, "{code}", Default::default());
    let http = kiroku_core::net::Http::new(std::time::Duration::ZERO, None).unwrap();
    let mut result = pipeline::apply(plans, &http, &root, None, &CancellationToken::new(), &|_| {}).await;
    assert!(result.failed.is_empty());
    let dir = root.join("ABC-123");
    assert!(dir.join("ABC-123-C.nfo").exists());
    assert!(!dir.join("movie.nfo").exists());
    let lib = Library::open(&root).unwrap();
    lib.reindex_dir(&dir).unwrap();
    let mut cursor = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(4, 6).write_to(&mut cursor, image::ImageFormat::Png).unwrap();
    fs::write(dir.join("ABC-123-C-poster.jpg"), cursor.get_ref()).unwrap();
    fs::write(dir.join("ABC-123-C-fanart.jpg"), cursor.get_ref()).unwrap();
    lib.reindex_dir(&dir).unwrap();
    let detail = lib.get("ABC-123").unwrap().unwrap();
    assert!(detail.card.poster.as_ref().unwrap().ends_with("ABC-123-C-poster.jpg"));
    assert!(detail.fanart.as_ref().unwrap().ends_with("ABC-123-C-fanart.jpg"));
    let captured = kiroku_core::revision::capture(&root, &dir, "ABC-123").unwrap();
    let original = kiroku_core::artwork::Original::from_bytes(captured, cursor.into_inner(), None, Default::default()).unwrap();
    let (preview, _) = original.preview(kiroku_core::artwork::CropBox { x: 0, y: 0, width: 4, height: 6 }, None).unwrap();
    let mut crop = kiroku_core::revision::apply(&preview).unwrap();
    assert!(crop.error.is_none());
    assert!(!dir.join("poster.jpg").exists());
    assert!(crop.journal.rollback().is_empty());
    assert!(result.journal.rollback().is_empty());
    assert_eq!(fs::read(&input).unwrap(), b"video");
}

#[test]
fn artwork_plans_share_the_video_stem() {
    let code = parse_code("ABC-123.mp4").unwrap();
    let mut meta = Metadata { code: code.id.clone(), ..Default::default() };
    meta.fields.thumb_urls.push("https://example.invalid/cover.jpg".into());
    let plan = organize::plan(std::path::Path::new("ABC-123.mp4"), &code, &meta, std::path::Path::new("/library"), "{code}");
    assert!(plan.ops.iter().any(|op| op.target().ends_with("ABC-123-poster.jpg")));
    assert!(plan.ops.iter().any(|op| op.target().ends_with("ABC-123-fanart.jpg")));
}

#[test]
fn named_nfo_only_libraries_keep_existing_generic_artwork() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let meta = Metadata { code: "ABC-123".into(), ..Default::default() };
    fs::write(dir.join("ABC-123.nfo"), kiroku_core::nfo::to_nfo(&meta)).unwrap();
    fs::write(dir.join("ABC-123.mp4"), b"video").unwrap();
    fs::write(dir.join("poster.jpg"), b"poster").unwrap();
    fs::write(dir.join("fanart.jpg"), b"fanart").unwrap();
    let read = kiroku_core::append::read_meta(&dir).unwrap();
    assert!(read.asset_stem.is_none());
    let lib = Library::open(temp.path()).unwrap();
    lib.reindex_dir(&dir).unwrap();
    let detail = lib.get("ABC-123").unwrap().unwrap();
    assert!(detail.card.poster.unwrap().ends_with("/poster.jpg"));
    assert!(detail.fanart.unwrap().ends_with("/fanart.jpg"));
    assert_eq!(kiroku_core::append::targets(temp.path(), "ABC-123").unwrap(), vec![dir.canonicalize().unwrap()]);
    assert!(kiroku_core::revision::capture(temp.path(), &dir, "ABC-123").unwrap().before.asset_stem.is_none());
}

#[test]
fn missing_named_nfo_is_recreated_at_the_bound_stem_and_can_be_undone() {
    let temp = tempfile::tempdir().unwrap();
    let meta = Metadata { code: "ABC-123".into(), asset_stem: Some("ABC-123-C".into()), ..Default::default() };
    fs::write(temp.path().join("kiroku.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    let preview = kiroku_core::revision::preview(temp.path(), temp.path(), "ABC-123", serde_json::json!({"title":"updated"})).unwrap();
    let mut result = kiroku_core::revision::apply(&preview).unwrap();
    assert!(result.error.is_none());
    assert!(temp.path().join("ABC-123-C.nfo").exists());
    assert!(!temp.path().join("movie.nfo").exists());
    assert!(result.journal.rollback().is_empty());
    assert!(!temp.path().join("ABC-123-C.nfo").exists());
}

#[test]
fn ambiguous_nfo_directory_does_not_abort_catalog_inventory() {
    let temp = tempfile::tempdir().unwrap();
    let bad = temp.path().join("bad");
    fs::create_dir(&bad).unwrap();
    fs::write(bad.join("movie.nfo"), b"one").unwrap();
    fs::write(bad.join("another.nfo"), b"two").unwrap();
    let result = kiroku_core::catalog::inventory(temp.path(), &CancellationToken::new()).unwrap();
    assert_eq!(result.errors.len(), 1);
}
