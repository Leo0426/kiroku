//! Real disk integration across the migration features; synthetic media and localhost only.
use async_trait::async_trait;
use kiroku_core::{
    artwork::{CropBox, Original},
    code::{parse_code, Code},
    library::Library,
    metadata::PartialMetadata,
    net::Http,
    organize::PlanOpts,
    pipeline::{self, Scraper},
    refresh, revision,
    settings::Settings,
    source::{Source, SourceError, SourceId},
    Metadata,
};
use std::{
    collections::HashMap,
    fs,
    io::{Cursor, Read, Write},
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

struct FixtureSource {
    calls: Arc<Mutex<Vec<String>>>,
}
#[async_trait]
impl Source for FixtureSource {
    fn id(&self) -> SourceId {
        "javdb"
    }
    fn supports(&self, _: &Code) -> bool {
        true
    }
    async fn fetch(&self, code: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
        let mut calls = self.calls.lock().unwrap();
        let prior = calls.iter().filter(|c| *c == &code.id).count();
        calls.push(code.id.clone());
        if code.id == "ABC-124" && prior == 0 {
            return Err(SourceError::NotFound);
        }
        Ok(PartialMetadata {
            title: Some(format!("source {}", code.id)),
            plot: Some(format!("plot generation {prior}")),
            studio: Some("unchanged studio".into()),
            ..Default::default()
        })
    }
}
fn png() -> Vec<u8> {
    let mut out = Cursor::new(vec![]);
    image::DynamicImage::new_rgb8(80, 60)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}
fn metadata(dir: &std::path::Path) -> Metadata {
    serde_json::from_slice(&fs::read(dir.join("kiroku.json")).unwrap()).unwrap()
}

#[cfg(unix)]
fn owned_revision() -> (
    tempfile::TempDir,
    kiroku_core::journal::Journal,
    kiroku_core::journal::Journal,
) {
    use kiroku_core::{
        journal::Journal,
        organize::{Op, Plan},
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("poster.jpg");
    let mut import = Journal::new(&root);
    let result = import.execute(
        &Plan {
            ops: vec![Op::WriteFile {
                path: path.clone(),
                content: "before".into(),
            }],
        },
        &mut |_| unreachable!(),
        &|| false,
    );
    assert!(result.error.is_none());
    let before = import.proofs[&0].digest.clone();
    let mut updated = Journal::new(&root);
    updated
        .replace_files(&[kiroku_core::journal::ReplacementInput {
            path,
            expected: Some(before),
            bytes: b"after".to_vec(),
        }])
        .unwrap();
    assert!(updated.replacements[0].before_owner.is_some());
    (temp, import, updated)
}

#[cfg(unix)]
#[test]
fn ownership_anchor_rejects_lost_replaced_or_tampered_proofs_and_keeps_independent_backup() {
    for fault in [
        "stage_copy",
        "stage_missing",
        "stage_content",
        "source_proof",
        "backup",
        "anchor_id",
        "restore_copy",
    ] {
        let (_temp, mut import, updated) = owned_revision();
        let path = updated.replacements[0].path.clone();
        let stage = import.proofs[&0].stage.clone();
        let saved = stage.with_extension("kept");
        let journal_bytes = fs::read(import.path()).unwrap();
        let backup = updated.replacements[0].backup.clone().unwrap();
        let backup_bytes = fs::read(&backup).unwrap();
        let mut updated = kiroku_core::journal::Journal::load(&updated.path()).unwrap();
        match fault {
            "stage_copy" => {
                fs::rename(&stage, &saved).unwrap();
                fs::write(&stage, b"before").unwrap();
            }
            "stage_missing" => {
                fs::rename(&stage, &saved).unwrap();
            }
            "stage_content" => {
                fs::write(&stage, b"tampered").unwrap();
            }
            "source_proof" => {
                let mut false_log = import.clone();
                false_log.proofs.get_mut(&0).unwrap().digest = "fake".into();
                false_log.save().unwrap();
            }
            "backup" => {
                fs::write(&backup, b"broken").unwrap();
            }
            "anchor_id" => {
                updated.replacements[0]
                    .before_owner
                    .as_mut()
                    .unwrap()
                    .journal_id = "../escape".into();
            }
            "restore_copy" => {
                fs::write(
                    updated.replacements[0].stage.with_extension("restore"),
                    b"before",
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(!updated.rollback().is_empty(), "{fault}");
        assert_eq!(fs::read(&path).unwrap(), b"after", "{fault}");
        assert!(!updated.replacements[0].undone);
        match fault {
            "stage_copy" => {
                fs::remove_file(&stage).unwrap();
                fs::rename(&saved, &stage).unwrap();
            }
            "stage_missing" => {
                fs::rename(&saved, &stage).unwrap();
            }
            "stage_content" => {
                fs::write(&stage, b"before").unwrap();
            }
            "source_proof" => {
                fs::write(import.path(), journal_bytes).unwrap();
            }
            "backup" => {
                fs::write(&backup, backup_bytes).unwrap();
            }
            "anchor_id" => {
                updated.replacements[0]
                    .before_owner
                    .as_mut()
                    .unwrap()
                    .journal_id = import.id.clone();
            }
            "restore_copy" => {
                fs::remove_file(updated.replacements[0].stage.with_extension("restore")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(updated.rollback().is_empty(), "{fault}");
        assert!(same_file::is_same_file(&path, &stage).unwrap());
        // Re-read the repaired journal: the false snapshot replaced its inode,
        // so the old in-memory writer must not overwrite the restored history.
        if fault=="source_proof" {import=kiroku_core::journal::Journal::load(&import.path()).unwrap();}
        assert!(import.rollback().is_empty(), "{fault}");
        assert!(!path.exists());
    }
}

#[cfg(unix)]
#[test]
fn equal_bytes_foreign_inode_is_never_v2_ownership_or_valid_restored_before() {
    let (_temp, mut import, mut updated) = owned_revision();
    let path = updated.replacements[0].path.clone();
    let foreign = path.with_extension("foreign");
    fs::write(&foreign, b"before").unwrap();
    fs::rename(&foreign, &path).unwrap();
    assert!(!updated.rollback().is_empty());
    assert!(!import.rollback().is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"before");
    // A v3 update of an already foreign equal-byte replacement gets no v2 anchor.
    let mut ordinary = kiroku_core::journal::Journal::new(&import.root);
    ordinary
        .replace_files(&[kiroku_core::journal::ReplacementInput {
            path: path.clone(),
            expected: Some(import.proofs[&0].digest.clone()),
            bytes: b"ordinary".to_vec(),
        }])
        .unwrap();
    assert!(ordinary.replacements[0].before_owner.is_none());
    assert!(ordinary.rollback().is_empty());
    assert!(!import.rollback().is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"before");
}

#[tokio::test]
async fn legacy_root_nfo_repair_executes_and_undo_preserves_video_without_sidecar() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/poster", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        stream.read(&mut request).unwrap();
        let bytes = png();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .unwrap();
        stream.write_all(&bytes).unwrap();
    });
    let nfo = format!("<movie><num>ABC-123</num><title>foreign</title><thumb aspect=\"poster\">{url}</thumb></movie>");
    fs::write(root.join("ABC-123.nfo"), &nfo).unwrap();
    fs::write(root.join("ABC-123.mp4"), b"synthetic video").unwrap();
    let lib = Library::open(root).unwrap();
    lib.reindex_dir(root).unwrap();
    assert!(lib.get("ABC-123").unwrap().is_some());
    let captured = revision::capture(root, root, "ABC-123").unwrap();
    let plan = pipeline::build_image_repair(&captured.before, root, root).unwrap();
    let http = Settings::default().build_http().unwrap();
    let mut result = pipeline::apply(
        vec![plan],
        &http,
        root,
        None,
        &CancellationToken::new(),
        &|_| {},
    )
    .await;
    server.join().unwrap();
    assert!(result.failed.is_empty());
    lib.reindex_dir(root).unwrap();
    assert!(lib.get("ABC-123").unwrap().unwrap().card.poster.is_some());
    assert_eq!(fs::read_to_string(root.join("ABC-123.nfo")).unwrap(), nfo);
    assert!(!root.join("kiroku.json").exists());
    assert!(result.journal.rollback().is_empty());
    lib.reindex_dir(root).unwrap();
    assert!(!root.join("poster.jpg").exists());
    assert_eq!(
        fs::read(root.join("ABC-123.mp4")).unwrap(),
        b"synthetic video"
    );
    assert_eq!(fs::read_to_string(root.join("ABC-123.nfo")).unwrap(), nfo);
    assert!(!root.join("kiroku.json").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn retry_import_repair_edit_lock_cache_crop_reverse_undo_reindex() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("input");
    let root = temp.path().join("library");
    fs::create_dir(&input).unwrap();
    fs::create_dir(&root).unwrap();
    for name in ["ABC-123-CD1.mp4", "ABC-123-CD2.mp4", "ABC-124.mp4"] {
        fs::write(input.join(name), name.as_bytes()).unwrap();
    }
    let calls = Arc::new(Mutex::new(vec![]));
    let mut settings = Settings::default();
    for source in &mut settings.sources {
        source.enabled = source.id == "javdb";
    }
    let scraper = Scraper::with_sources(
        settings,
        vec![Box::new(FixtureSource {
            calls: calls.clone(),
        })],
    )
    .unwrap();
    let cancel = CancellationToken::new();
    let a = parse_code("ABC-123").unwrap();
    let b = parse_code("ABC-124").unwrap();
    let success = scraper.scrape_item(&a, &cancel, &|_| {}).await.unwrap();
    assert!(scraper.scrape_item(&b, &cancel, &|_| {}).await.is_err());
    let retry = scraper.scrape_item(&b, &cancel, &|_| {}).await.unwrap();
    assert_eq!(*calls.lock().unwrap(), ["ABC-123", "ABC-124", "ABC-124"]);
    let scanned = pipeline::scan(
        &[input.clone()],
        None,
        &kiroku_core::video_filter::VideoFilter::none(),
        &cancel,
        &mut |_| {},
    );
    let pairs: Vec<_> = scanned
        .items
        .iter()
        .map(|s| (s, s.code.as_ref().unwrap()))
        .collect();
    let metas = HashMap::from([(a.id.clone(), success), (b.id.clone(), retry)]);
    let plans = pipeline::build_plans(&pairs, &metas, &root, "{code}", PlanOpts::default());
    let mut imported = pipeline::apply(plans, &scraper.http, &root, None, &cancel, &|_| {}).await;
    assert!(imported.failed.is_empty());
    assert_eq!(imported.applied.len(), 3);
    let dir = root.join(&a.id);
    let lib = Library::open(&root).unwrap();
    lib.reindex_dir(&dir).unwrap();
    let videos: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "mp4"))
        .map(|p| {
            let bytes = fs::read(&p).unwrap();
            (p, bytes)
        })
        .collect();
    assert_eq!(videos.len(), 2);
    let initial_side = fs::read(dir.join("kiroku.json")).unwrap();
    let assets = kiroku_core::assets::Paths::for_movie(&dir, &metadata(&dir)).unwrap();
    let initial_nfo = fs::read(&assets.nfo).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/poster", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        stream.read(&mut request).unwrap();
        let bytes = png();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .unwrap();
        stream.write_all(&bytes).unwrap();
    });
    let mut repair_meta = metadata(&dir);
    repair_meta.fields.poster_urls = vec![url];
    let plan = pipeline::build_image_repair(&repair_meta, &dir, &root).unwrap();
    let mut repaired =
        pipeline::apply(vec![plan], &scraper.http, &root, None, &cancel, &|_| {}).await;
    server.join().unwrap();
    assert!(repaired.failed.is_empty());
    let repaired_poster = fs::read(&assets.poster).unwrap();
    let edit = revision::preview(
        &root,
        &dir,
        &a.id,
        serde_json::json!({"title":"manual title","series":null}),
    )
    .unwrap();
    let mut edited = revision::apply(&edit).unwrap().journal;
    let before = metadata(&dir);
    let locked = refresh::set_lock(&before, "series", true).unwrap();
    let mut locks = revision::apply(&revision::prepare(&root, &dir, &a.id, locked).unwrap())
        .unwrap()
        .journal;
    // Reload persisted snapshots and locks, reuse the same identity without network.
    let before = metadata(&dir);
    let count = calls.lock().unwrap().len();
    let cached = scraper
        .scrape_cached(&a, &before.raw_sources, false, &cancel, &|_| {})
        .await
        .unwrap();
    assert_eq!(calls.lock().unwrap().len(), count);
    let forced = scraper
        .scrape_cached(&a, &cached.raw_sources, true, &cancel, &|_| {})
        .await
        .unwrap();
    assert_eq!(calls.lock().unwrap().len(), count + 1);
    let updated = refresh::merge_selected(
        &before,
        &forced,
        &["plot".into(), "title".into(), "series".into()],
        true,
    )
    .unwrap();
    assert_eq!(updated.fields.title.as_deref(), Some("manual title"));
    assert_eq!(updated.fields.series, None);
    assert_eq!(updated.fields.studio, before.fields.studio);
    assert_ne!(updated.fields.plot, before.fields.plot);
    let mut refreshed = revision::apply(&revision::prepare(&root, &dir, &a.id, updated).unwrap())
        .unwrap()
        .journal;
    let original = Original::from_bytes(
        revision::capture(&root, &dir, &a.id).unwrap(),
        png(),
        None,
        Default::default(),
    )
    .unwrap();
    let (crop, _) = original
        .preview(
            CropBox {
                x: 0,
                y: 0,
                width: 30,
                height: 50,
            },
            None,
        )
        .unwrap();
    let mut first = revision::apply(&crop).unwrap().journal;
    let reopened = Original::load(revision::capture(&root, &dir, &a.id).unwrap())
        .unwrap()
        .unwrap();
    let (crop, _) = reopened
        .preview(
            CropBox {
                x: 40,
                y: 0,
                width: 30,
                height: 50,
            },
            None,
        )
        .unwrap();
    let mut second = revision::apply(&crop).unwrap().journal;
    for journal in [
        &mut second,
        &mut first,
        &mut refreshed,
        &mut locks,
        &mut edited,
    ] {
        assert!(journal.rollback().is_empty());
        lib.reindex_dir(&dir).unwrap();
    }
    assert_eq!(fs::read(&assets.poster).unwrap(), repaired_poster);
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), initial_side);
    assert_eq!(fs::read(&assets.nfo).unwrap(), initial_nfo);
    assert!(repaired.journal.rollback().is_empty());
    lib.reindex_dir(&dir).unwrap();
    assert!(!assets.poster.exists());
    for (path, bytes) in videos {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    assert_eq!(lib.rebuild(&cancel, &mut |_, _| {}).unwrap(), 2);
    assert_eq!(lib.get(&a.id).unwrap().unwrap().meta, metadata(&dir));
    let failures = imported.journal.rollback();
    assert!(failures.is_empty(), "{failures:?}");
    for name in ["ABC-123-CD1.mp4", "ABC-123-CD2.mp4", "ABC-124.mp4"] {
        assert_eq!(fs::read(input.join(name)).unwrap(), name.as_bytes());
    }
    assert_eq!(lib.rebuild(&cancel, &mut |_, _| {}).unwrap(), 0);
}

#[cfg(unix)]
#[test]
fn alias_root_original_import_crop_and_reverse_undo() {
    use kiroku_core::{
        journal::Journal,
        organize::{Op, Plan},
    };
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("library");
    let root = temp.path().join("alias");
    fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &root).unwrap();
    let dir = root.join("ABC-123");
    fs::create_dir(&dir).unwrap();
    let seed = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&seed).unwrap()).unwrap();
    let original = Original::from_bytes(
        revision::capture(&root, &dir, &seed.code).unwrap(),
        png(),
        None,
        Default::default(),
    )
    .unwrap();
    fs::remove_file(dir.join("kiroku.json")).unwrap();
    let mut meta = seed;
    meta.artwork = Some(original.artwork.clone());
    let mut ops = vec![
        Op::WriteFile {
            path: dir.join("kiroku.json"),
            content: serde_json::to_string(&meta).unwrap(),
        },
        Op::Download {
            path: dir.join(&original.artwork.original),
            url: "fixture".into(),
            crop: None,
            stamp: None,
        },
    ];
    let mut inputs = vec![];
    for part in ["CD1", "CD2"] {
        let input = temp.path().join(format!("ABC-123-{part}.mp4"));
        fs::write(&input, part.as_bytes()).unwrap();
        ops.push(Op::MoveFile {
            from: input.clone(),
            to: dir.join(input.file_name().unwrap()),
        });
        inputs.push((input, part.as_bytes().to_vec()));
    }
    let mut imported = Journal::new(&root);
    let result = imported.execute(&Plan { ops }, &mut |_| Ok(original.bytes.clone()), &|| {
        false
    });
    assert!(result.error.is_none(), "{:?}", result.error);
    let initial_side = fs::read(dir.join("kiroku.json")).unwrap();
    let loaded = Original::load(revision::capture(&root, &dir, &meta.code).unwrap())
        .unwrap()
        .unwrap();
    let (crop, _) = loaded
        .preview(
            CropBox {
                x: 0,
                y: 0,
                width: 30,
                height: 50,
            },
            None,
        )
        .unwrap();
    let mut cropped = revision::apply(&crop).unwrap().journal;
    let cropped_side = fs::read(dir.join("kiroku.json")).unwrap();
    assert!(
        !imported.rollback().is_empty(),
        "newer crop must block import undo"
    );
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), cropped_side);
    assert_eq!(
        fs::read(dir.join(&original.artwork.original)).unwrap(),
        original.bytes
    );
    assert!(cropped.rollback().is_empty());
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), initial_side);
    // Exercise both the preflight proof check and the actual original deletion boundary.
    let errors = imported.rollback();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(!dir.join(&original.artwork.original).exists());
    assert!(!dir.join("kiroku.json").exists());
    for (input, bytes) in inputs {
        assert_eq!(fs::read(input).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[test]
fn alias_root_original_guard_rejects_descendant_aliases_and_escaping_paths() {
    use kiroku_core::{
        journal::Journal,
        organize::{Op, Plan},
    };
    for fault in [
        "internal_symlink",
        "external_symlink",
        "parent",
        "outside",
        "proof_symlink",
        "proof_parent",
        "proof_outside",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("library");
        let root = temp.path().join("alias");
        let outside = temp.path().join("outside");
        fs::create_dir(&real).unwrap();
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&real, &root).unwrap();
        let dir = root.join("movie");
        fs::create_dir(&dir).unwrap();
        let original = ".kiroku-original-fixture.png";
        let bytes = png();
        let side = format!(r#"{{"artwork":{{"original":"{original}"}}}}"#);
        let mut journal = Journal::new(&root);
        let result = journal.execute(
            &Plan {
                ops: vec![
                    Op::WriteFile {
                        path: dir.join("kiroku.json"),
                        content: side.clone(),
                    },
                    Op::Download {
                        path: dir.join(original),
                        url: "fixture".into(),
                        crop: None,
                        stamp: None,
                    },
                ],
            },
            &mut |_| Ok(bytes.clone()),
            &|| false,
        );
        assert!(result.error.is_none());
        let proof = journal.proofs[&0].stage.clone();
        let bad = match fault {
            "internal_symlink" => {
                std::os::unix::fs::symlink(&dir, root.join("link")).unwrap();
                root.join("link").join(original)
            }
            "external_symlink" => {
                std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
                root.join("link").join(original)
            }
            "parent" => dir.join("..").join("movie").join(original),
            "outside" => outside.join(original),
            "proof_symlink" => {
                let link = dir.join("proof-link");
                std::os::unix::fs::symlink(&proof, &link).unwrap();
                link
            }
            "proof_parent" => dir
                .join("..")
                .join("movie")
                .join(proof.file_name().unwrap()),
            "proof_outside" => {
                let linked = outside.join("proof");
                fs::hard_link(&proof, &linked).unwrap();
                linked
            }
            _ => unreachable!(),
        };
        if fault.starts_with("proof_") {
            journal.proofs.get_mut(&0).unwrap().stage = bad;
        } else if let Op::Download { path, .. } = &mut journal.ops[1] {
            *path = bad;
        }
        let errors = journal.rollback();
        assert!(!errors.is_empty(), "unsafe {fault} must fail");
        assert!(journal.rolled_back_ops.is_empty(), "{fault}");
        assert_eq!(
            fs::read_to_string(dir.join("kiroku.json")).unwrap(),
            side,
            "{fault}"
        );
        assert_eq!(fs::read(dir.join(original)).unwrap(), bytes, "{fault}");
    }
}
