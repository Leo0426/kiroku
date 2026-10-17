use kiroku_core::{
    artwork::{self, CropBox},
    revision, Metadata,
};
use std::{fs, io::Cursor};
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = Cursor::new(vec![]);
    image::DynamicImage::new_rgb8(w, h)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}
#[test]
fn downloaded_original_rejects_excessive_edge_before_publication() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let m = Metadata { code: "ABC-123".into(), ..Default::default() };
    fs::write(root.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    fs::write(root.join("movie.mp4"), b"video").unwrap();
    let result = artwork::Original::from_download_bytes(
        revision::capture(&root, &root, &m.code).unwrap(),
        png(8193, 1), Some("https://example.com/original.png".into()), Default::default(),
    );
    assert!(result.is_err(), "a new remote original must respect the image edge budget");
    assert!(fs::read_dir(&root).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".kiroku-original-")));
}
#[test]
fn crop_bounds_and_static_pixel_plane() {
    let bytes = png(800, 600);
    let out = artwork::normalize(&bytes).unwrap();
    assert_eq!(image::load_from_memory(&out).unwrap().width(), 800);
    for rect in [
        CropBox {
            x: 0,
            y: 0,
            width: 0,
            height: 10,
        },
        CropBox {
            x: u32::MAX,
            y: 0,
            width: 2,
            height: 3,
        },
        CropBox {
            x: 799,
            y: 0,
            width: 2,
            height: 1,
        },
    ] {
        assert!(artwork::crop(&out, rect, None).is_err());
    }
    let result = artwork::crop(
        &out,
        CropBox {
            x: 10,
            y: 20,
            width: 250,
            height: 400,
        },
        None,
    )
    .unwrap();
    let img = image::load_from_memory(&result).unwrap();
    assert_eq!((img.width(), img.height()), (250, 400));
    assert!(artwork::normalize(b"<html>blocked</html>").is_err());
}
#[test]
fn original_survives_restart_second_crop_and_exact_undo() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    let side = serde_json::to_vec(&m).unwrap();
    fs::write(root.join("kiroku.json"), &side).unwrap();
    fs::write(
        root.join("movie.nfo"),
        "<movie><!--preserve--><title>old</title></movie>",
    )
    .unwrap();
    fs::write(root.join("movie.mp4"), b"video").unwrap();
    fs::write(root.join("poster.jpg"), b"old poster").unwrap();
    let lib = kiroku_core::library::Library::open(&root).unwrap();
    lib.reindex_dir(&root).unwrap();
    let initial_version = lib.get(&m.code).unwrap().unwrap().card.poster_version;
    let original = artwork::Original::from_download_bytes(
        revision::capture(&root, &root, &m.code).unwrap(),
        png(800, 600),
        None,
        Default::default(),
    )
    .unwrap();
    let (p, _) = original
        .preview(
            CropBox {
                x: 0,
                y: 0,
                width: 300,
                height: 500,
            },
            None,
        )
        .unwrap();
    assert_eq!(fs::read(root.join("poster.jpg")).unwrap(), b"old poster");
    let mut first = revision::apply(&p).unwrap().journal;
    let saved: Metadata =
        serde_json::from_slice(&fs::read(root.join("kiroku.json")).unwrap()).unwrap();
    assert!(saved.locked_fields.contains("poster_urls"));
    lib.reindex_dir(&root).unwrap();
    let cropped_version = lib.get(&m.code).unwrap().unwrap().card.poster_version;
    assert_ne!(cropped_version, initial_version);
    assert_eq!(
        lib.get(&m.code).unwrap().unwrap().meta.artwork,
        saved.artwork
    );
    let original_path = root.join(&saved.artwork.as_ref().unwrap().original);
    assert!(original_path.exists());
    let reopened = artwork::Original::load(revision::capture(&root, &root, &m.code).unwrap())
        .unwrap()
        .unwrap();
    let (p2, _) = reopened
        .preview(
            CropBox {
                x: 500,
                y: 0,
                width: 300,
                height: 500,
            },
            None,
        )
        .unwrap();
    let mut second = revision::apply(&p2).unwrap().journal;
    assert!(second.rollback().is_empty());
    assert!(original_path.exists());
    assert!(first.rollback().is_empty());
    assert!(!original_path.exists());
    assert_eq!(fs::read(root.join("poster.jpg")).unwrap(), b"old poster");
    lib.reindex_dir(&root).unwrap();
    assert_eq!(lib.get(&m.code).unwrap().unwrap().card.poster_version, initial_version);
    assert_eq!(fs::read(root.join("kiroku.json")).unwrap(), side);
    assert_eq!(fs::read(root.join("movie.mp4")).unwrap(), b"video");
}
#[test]
fn quality_portrait_then_thumb_then_ordered_fallback() {
    use artwork::preferred;
    assert_eq!(preferred(&[(true, 1200, 800), (false, 400, 750)]), Some(1));
    assert_eq!(preferred(&[(true, 1200, 800), (false, 400, 600)]), Some(0));
    assert_eq!(preferred(&[(false, 400, 600), (false, 500, 900)]), Some(0));
    assert_eq!(preferred(&[(true, 1200, 800), (false, 1000, 750)]), Some(0));
    assert_eq!(preferred(&[]), None);
}
#[test]
fn locked_poster_repair_does_not_silently_recrop_from_fresh_thumb() {
    let t = tempfile::tempdir().unwrap();
    let mut m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    m.fields.thumb_urls = vec!["https://fresh.invalid/thumb".into()];
    m.locked_fields.insert("poster_urls".into());
    let plan = kiroku_core::pipeline::build_image_repair(&m, t.path(), t.path()).unwrap();
    assert!(!plan
        .ops
        .iter()
        .any(|op| op.target().ends_with("poster.jpg")));
}
#[test]
fn current_raw_repairs_expired_image_urls_without_changing_selected_metadata() {
    use kiroku_core::{metadata::SourceSnapshot, settings::Settings, PartialMetadata};
    let settings = Settings::default();
    let code = kiroku_core::parse_code("ABC-123").unwrap();
    let cfg = settings.sources.iter().find(|s| s.id == "javdb").unwrap();
    let mut before = Metadata {
        code: code.id.clone(),
        ..Default::default()
    };
    before.fields.title = Some("old".into());
    before.fields.poster_urls = vec!["https://expired.invalid/poster".into()];
    let raw = PartialMetadata {
        title: Some("fresh title".into()),
        poster_urls: vec!["https://valid.invalid/poster".into()],
        thumb_urls: vec!["https://valid.invalid/thumb".into()],
        ..Default::default()
    };
    let mut refreshed = before.clone();
    refreshed.fields = raw.clone();
    refreshed.raw_sources.insert(
        "javdb".into(),
        SourceSnapshot::configured(&code, cfg, settings.proxy.as_deref(), &raw),
    );
    let saved =
        kiroku_core::refresh::merge_selected(&before, &refreshed, &["title".into()], true).unwrap();
    assert_eq!(saved.fields.poster_urls, before.fields.poster_urls);
    let repair = artwork::repair_metadata(&saved, &settings);
    assert_eq!(repair.fields.poster_urls, raw.poster_urls);
    assert_eq!(repair.fields.title, saved.fields.title);
    assert_eq!(saved.fields.poster_urls, before.fields.poster_urls);
    let mut locked = saved.clone();
    locked.locked_fields.insert("poster_urls".into());
    assert_eq!(
        artwork::repair_metadata(&locked, &settings)
            .fields
            .poster_urls,
        before.fields.poster_urls
    );
    let mut settings = settings;
    settings
        .sources
        .iter_mut()
        .find(|s| s.id == "javdb")
        .unwrap()
        .cookie = "changed".into();
    assert_eq!(
        artwork::repair_metadata(&saved, &settings)
            .fields
            .poster_urls,
        before.fields.poster_urls
    );
}
#[test]
fn exif_orientation_is_normalized_to_same_static_preview_plane() {
    let mut jpeg = vec![];
    image::DynamicImage::new_rgb8(30, 50)
        .write_with_encoder(image::codecs::jpeg::JpegEncoder::new(&mut jpeg))
        .unwrap();
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut oriented = vec![0xff, 0xd8, 0xff, 0xe1];
    oriented.extend(((exif.len() + 2) as u16).to_be_bytes());
    oriented.extend(exif);
    oriented.extend(&jpeg[2..]);
    let normalized = artwork::normalize(&oriented).unwrap();
    assert_eq!(artwork::dimensions(&normalized).unwrap(), (50, 30));
    assert_eq!(normalized, artwork::normalize(&normalized).unwrap());
    let cropped = artwork::crop(
        &normalized,
        CropBox {
            x: 35,
            y: 5,
            width: 15,
            height: 25,
        },
        None,
    )
    .unwrap();
    assert_eq!(artwork::dimensions(&cropped).unwrap(), (15, 25));
}
#[test]
fn stale_poster_original_and_sidecar_are_preserved() {
    for target in ["poster.jpg", "kiroku.json", "original"] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let m = Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        };
        fs::write(root.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
        let original = artwork::Original::from_bytes(
            revision::capture(&root, &root, &m.code).unwrap(),
            png(80, 60),
            None,
            Default::default(),
        )
        .unwrap();
        let (p, _) = original
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
        let path = if target == "original" {
            root.join(&original.artwork.original)
        } else {
            root.join(target)
        };
        fs::write(&path, b"foreign").unwrap();
        assert!(revision::apply(&p).is_err());
        assert_eq!(fs::read(path).unwrap(), b"foreign");
    }
}
#[test]
fn deleting_a_retained_original_after_open_is_not_silently_recreated() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    fs::write(root.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    let o = artwork::Original::from_bytes(
        revision::capture(&root, &root, &m.code).unwrap(),
        png(80, 60),
        None,
        Default::default(),
    )
    .unwrap();
    let (p, _) = o
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
    revision::apply(&p).unwrap();
    let reopened = artwork::Original::load(revision::capture(&root, &root, &m.code).unwrap())
        .unwrap()
        .unwrap();
    fs::remove_file(root.join(&reopened.artwork.original)).unwrap();
    assert!(reopened
        .preview(
            CropBox {
                x: 2,
                y: 0,
                width: 30,
                height: 50
            },
            None
        )
        .is_err());
}
#[test]
fn original_filename_retains_4k_phase_and_crop_stamps_only_once() {
    use kiroku_core::{
        badge::{BadgeCfg, Phase},
        journal::Journal,
        organize::Op,
    };
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let mut j = Journal::new(&root);
    j.push_ops([
        Op::MoveFile {
            from: root.join("input/ABC-123-4K-C.mp4"),
            to: root.join("ABC-123.mp4"),
        },
        Op::Download {
            url: "https://original.invalid/cover".into(),
            path: root.join("poster.jpg"),
            crop: None,
            stamp: None,
        },
    ]);
    j.save().unwrap();
    let (url, phase) = artwork::legacy_source(&root, &root).unwrap();
    assert_eq!(url.as_deref(), Some("https://original.invalid/cover"));
    assert_eq!(phase.definition.as_deref(), Some("4K"));
    assert!(phase.subtitle);
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    fs::write(root.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    let original = artwork::Original::from_bytes(
        revision::capture(&root, &root, &m.code).unwrap(),
        png(800, 600),
        url,
        phase.clone(),
    )
    .unwrap();
    let stamp = BadgeCfg {
        enabled: true,
        ..Default::default()
    }
    .stamping(&phase, None);
    let rect = CropBox {
        x: 300,
        y: 0,
        width: 400,
        height: 600,
    };
    let (p, poster) = original.preview(rect, stamp.as_ref()).unwrap();
    assert_ne!(poster, artwork::crop(&original.bytes, rect, None).unwrap());
    revision::apply(&p).unwrap();
    let reopened = artwork::Original::load(revision::capture(&root, &root, &m.code).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(reopened.artwork.phase, phase);
    assert_eq!(reopened.preview(rect, stamp.as_ref()).unwrap().1, poster);
    assert_eq!(reopened.bytes, original.bytes);
    let _: Phase = phase;
}
#[test]
fn unsafe_original_reference_is_rejected() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let mut m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    m.artwork = Some(artwork::Artwork {
        original: "../outside.png".into(),
        digest: "fake".into(),
        source_url: None,
        crop: None,
        phase: Default::default(),
    });
    fs::write(root.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    assert!(artwork::Original::load(revision::capture(&root, &root, &m.code).unwrap()).is_err());
}
#[tokio::test]
async fn downloaded_quality_selection_and_fallback_records_actual_url() {
    use std::io::{Read, Write};
    for high in [true, false] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = [0; 1024];
                let n = stream.read(&mut req).unwrap();
                let request = String::from_utf8_lossy(&req[..n]);
                let bytes = if request.contains("/bad") {
                    b"<html>blocked</html>".to_vec()
                } else if request.contains("/thumb") {
                    png(120, 80)
                } else if request.contains("/small") {
                    png(30, 40)
                } else {
                    png(40, if high { 75 } else { 65 })
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .unwrap();
                stream.write_all(&bytes).unwrap();
            }
        });
        let t = tempfile::tempdir().unwrap();
        let mut m = Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        };
        m.fields.thumb_urls = vec![format!("{base}/bad"), format!("{base}/thumb")];
        m.fields.poster_urls = vec![format!("{base}/small"), format!("{base}/portrait")];
        let plan = kiroku_core::pipeline::build_image_repair(&m, t.path(), t.path()).unwrap();
        let out = kiroku_core::pipeline::apply(
            vec![plan],
            &kiroku_core::net::Http::new(std::time::Duration::ZERO, None).unwrap(),
            t.path(),
            None,
            &tokio_util::sync::CancellationToken::new(),
            &|_| {},
        )
        .await;
        server.join().unwrap();
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        assert_eq!(
            artwork::dimensions(&fs::read(t.path().join("poster.jpg")).unwrap()).unwrap(),
            if high { (40, 75) } else { (56, 80) }
        );
        assert!(out.journal.ops.iter().any(|op|matches!(op,kiroku_core::organize::Op::Download{path,url,..} if path.ends_with("poster.jpg") && url.ends_with(if high{"/portrait"}else{"/thumb"}))));
    }
}

#[test]
fn undo_older_crop_keeps_original_referenced_by_newer_crop_then_reverse_undo_succeeds() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    let before = serde_json::to_vec(&m).unwrap();
    fs::write(root.join("kiroku.json"), &before).unwrap();
    let o = artwork::Original::from_bytes(
        revision::capture(&root, &root, &m.code).unwrap(),
        png(80, 60),
        None,
        Default::default(),
    )
    .unwrap();
    let (p, _) = o
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
    let mut a = revision::apply(&p).unwrap().journal;
    let loaded = artwork::Original::load(revision::capture(&root, &root, &m.code).unwrap())
        .unwrap()
        .unwrap();
    let (p, _) = loaded
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
    let mut b = revision::apply(&p).unwrap().journal;
    assert!(!a.rollback().is_empty());
    assert!(
        root.join(&o.artwork.original).exists(),
        "older undo must preserve an original still referenced by newer sidecar"
    );
    assert!(
        artwork::Original::load(revision::capture(&root, &root, &m.code).unwrap())
            .unwrap()
            .is_some()
    );
    assert!(b.rollback().is_empty());
    assert!(a.rollback().is_empty());
    assert!(!root.join(&o.artwork.original).exists());
    assert_eq!(fs::read(root.join("kiroku.json")).unwrap(), before);
}
#[test]
fn original_rollback_preserves_files_when_sidecar_backup_is_corrupt() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    fs::write(root.join("kiroku.json"), serde_json::to_vec(&m).unwrap()).unwrap();
    let o = artwork::Original::from_bytes(
        revision::capture(&root, &root, &m.code).unwrap(),
        png(80, 60),
        None,
        Default::default(),
    )
    .unwrap();
    let (p, _) = o
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
    let mut journal = revision::apply(&p).unwrap().journal;
    let backup = journal
        .replacements
        .iter()
        .find(|r| r.path.ends_with("kiroku.json"))
        .unwrap()
        .backup
        .as_ref()
        .unwrap();
    fs::write(backup, b"foreign backup").unwrap();
    let side = fs::read(root.join("kiroku.json")).unwrap();
    let poster = fs::read(root.join("poster.jpg")).unwrap();
    assert!(!journal.rollback().is_empty());
    assert_eq!(fs::read(root.join("kiroku.json")).unwrap(), side);
    assert_eq!(fs::read(root.join("poster.jpg")).unwrap(), poster);
    assert!(root.join(&o.artwork.original).exists());
}
#[test]
fn import_original_rollback_is_blocked_by_newer_metadata_reference() {
    use kiroku_core::{
        journal::Journal,
        organize::{Op, Plan},
    };
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let bytes = artwork::normalize(&png(80, 60)).unwrap();
    let seed = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    fs::write(root.join("kiroku.json"), serde_json::to_vec(&seed).unwrap()).unwrap();
    let o = artwork::Original::from_bytes(
        revision::capture(&root, &root, &seed.code).unwrap(),
        bytes.clone(),
        None,
        Default::default(),
    )
    .unwrap();
    fs::remove_file(root.join("kiroku.json")).unwrap();
    let mut m = seed;
    m.artwork = Some(o.artwork.clone());
    let mut import = Journal::new(&root);
    let plan = Plan {
        ops: vec![
            Op::WriteFile {
                path: root.join("kiroku.json"),
                content: serde_json::to_string(&m).unwrap(),
            },
            Op::Download {
                path: root.join(&o.artwork.original),
                url: "fixture".into(),
                crop: None,
                stamp: None,
            },
        ],
    };
    assert!(import
        .execute(&plan, &mut |_| Ok(bytes.clone()), &|| false)
        .error
        .is_none());
    let p = revision::preview(&root, &root, &m.code, serde_json::json!({"title":"new"})).unwrap();
    revision::apply(&p).unwrap();
    let side = fs::read(root.join("kiroku.json")).unwrap();
    assert!(!import.rollback().is_empty());
    assert!(root.join(&o.artwork.original).exists());
    assert_eq!(fs::read(root.join("kiroku.json")).unwrap(), side);
}
#[test]
fn legacy_fanart_only_reused_when_same_url_unstamped_and_digest_verified() {
    use kiroku_core::{
        journal::Journal,
        organize::{Op, Plan},
    };
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let bytes = png(80, 60);
    let mut j = Journal::new(&root);
    let plan = Plan {
        ops: vec![Op::Download {
            url: "same".into(),
            path: root.join("fanart.jpg"),
            crop: None,
            stamp: None,
        }],
    };
    assert!(j
        .execute(&plan, &mut |_| Ok(bytes.clone()), &|| false)
        .error
        .is_none());
    assert_eq!(
        artwork::legacy_local_original(&root, &root, "same").unwrap(),
        Some(bytes)
    );
    assert!(artwork::legacy_local_original(&root, &root, "other")
        .unwrap()
        .is_none());
    fs::write(root.join("fanart.jpg"), b"foreign").unwrap();
    assert!(artwork::legacy_local_original(&root, &root, "same")
        .unwrap()
        .is_none());
}
#[test]
fn root_level_foreign_nfo_can_supply_repair_without_sidecar_creation() {
    let t = tempfile::tempdir().unwrap();
    let xml=b"<movie><num>ABC-123</num><title>foreign</title><thumb aspect=\"poster\">https://valid.invalid/poster</thumb></movie>";
    fs::write(t.path().join("ABC-123.nfo"), xml).unwrap();
    let c = revision::capture(t.path(), t.path(), "ABC-123").unwrap();
    let plan = kiroku_core::pipeline::build_image_repair(&c.before, t.path(), t.path()).unwrap();
    assert_eq!(plan.ops.len(), 1);
    assert!(!t.path().join("kiroku.json").exists());
    assert_eq!(fs::read(t.path().join("ABC-123.nfo")).unwrap(), xml);
}

#[test]
fn failed_sidecar_restore_keeps_original_and_retry_cleans_it_only_after_reference_removed() {
    for directory_obstruction in [false, true] {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().canonicalize().unwrap();
        let m = Metadata {
            code: "ABC-123".into(),
            ..Default::default()
        };
        let before = serde_json::to_vec(&m).unwrap();
        fs::write(root.join("kiroku.json"), &before).unwrap();
        fs::write(root.join("poster.jpg"), b"old poster").unwrap();
        let o = artwork::Original::from_bytes(
            revision::capture(&root, &root, &m.code).unwrap(),
            png(80, 60),
            None,
            Default::default(),
        )
        .unwrap();
        let (p, _) = o
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
        let mut j = revision::apply(&p).unwrap().journal;
        let restore = j
            .replacements
            .iter()
            .find(|r| r.path.ends_with("kiroku.json"))
            .unwrap()
            .stage
            .with_extension("restore");
        if directory_obstruction {
            fs::create_dir(&restore).unwrap()
        } else {
            fs::write(&restore, b"conflicting restore bytes").unwrap()
        }
        assert!(!j.rollback().is_empty());
        assert!(
            root.join(&o.artwork.original).exists(),
            "sidecar restoration failed; referenced original must remain"
        );
        assert!(
            artwork::Original::load(revision::capture(&root, &root, &m.code).unwrap())
                .unwrap()
                .is_some()
        );
        assert!(
            !j.replacements
                .iter()
                .find(|r| r.path.ends_with(&o.artwork.original))
                .unwrap()
                .undone
        );
        if directory_obstruction {
            fs::remove_dir(&restore).unwrap()
        } else {
            fs::remove_file(&restore).unwrap()
        }
        let mut j = kiroku_core::journal::Journal::load(&j.path()).unwrap();
        assert!(j.rollback().is_empty());
        assert!(!root.join(&o.artwork.original).exists());
        assert_eq!(fs::read(root.join("kiroku.json")).unwrap(), before);
        assert_eq!(fs::read(root.join("poster.jpg")).unwrap(), b"old poster");
    }
}

#[cfg(unix)]
#[test]
fn sidecar_rename_permission_failure_keeps_original_until_retry() {
    use std::os::unix::fs::PermissionsExt;
    struct WritableOnDrop(std::path::PathBuf);
    impl Drop for WritableOnDrop {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }
    let t = tempfile::tempdir().unwrap();
    let root = t.path().canonicalize().unwrap();
    let dir = root.join("movie");
    fs::create_dir(&dir).unwrap();
    let m = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    let before = serde_json::to_vec(&m).unwrap();
    fs::write(dir.join("kiroku.json"), &before).unwrap();
    let o = artwork::Original::from_bytes(
        revision::capture(&root, &dir, &m.code).unwrap(),
        png(80, 60),
        None,
        Default::default(),
    )
    .unwrap();
    let (p, _) = o
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
    let mut j = revision::apply(&p).unwrap().journal;
    let reset = WritableOnDrop(dir.clone());
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = root.join("probe");
    fs::write(&probe, b"probe").unwrap();
    if fs::rename(&probe, dir.join("probe")).is_ok() {
        drop(reset);
        return;
    } // Privileged test runners bypass mode restrictions.
    let errors = j.rollback();
    assert!(!errors.is_empty());
    assert!(
        errors.iter().any(|e| e.contains("sidecar 仍引用原图")),
        "must reach live-reference deletion guard: {errors:?}"
    );
    assert!(dir.join(&o.artwork.original).exists());
    assert!(
        artwork::Original::load(revision::capture(&root, &dir, &m.code).unwrap())
            .unwrap()
            .is_some()
    );
    drop(reset);
    assert!(j.rollback().is_empty());
    assert!(!dir.join(&o.artwork.original).exists());
    assert_eq!(fs::read(dir.join("kiroku.json")).unwrap(), before);
}

#[test]
fn existing_large_original_remains_loadable_cropable_and_undoable() {
    use sha2::Digest;
    let mut encoded = Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image::GrayImage::new(4097, 4097))
        .write_to(&mut encoded, image::ImageFormat::Png).unwrap();
    let bytes = encoded.into_inner();
    let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
    let t = tempfile::tempdir().unwrap(); let root = t.path().canonicalize().unwrap();
    let original = artwork::original_name(&hash);
    let metadata = Metadata { code: "ABC-123".into(), artwork: Some(artwork::Artwork {
        original: original.clone(), digest: hash, source_url: None, crop: None, phase: Default::default(),
    }), ..Default::default() };
    fs::write(root.join("kiroku.json"), serde_json::to_vec(&metadata).unwrap()).unwrap();
    fs::write(root.join("movie.nfo"), "<movie><title>old</title></movie>").unwrap();
    fs::write(root.join("movie.mp4"), b"video").unwrap();
    fs::write(root.join("poster.jpg"), b"old poster").unwrap();
    fs::write(root.join(&original), &bytes).unwrap();
    let loaded = artwork::Original::load(revision::capture(&root, &root, &metadata.code).unwrap()).unwrap().unwrap();
    let (preview, _) = loaded.preview(CropBox { x: 0, y: 0, width: 40, height: 60 }, None).unwrap();
    let mut journal = revision::apply(&preview).unwrap().journal;
    assert_eq!(fs::read(root.join(&original)).unwrap(), bytes);
    assert!(journal.rollback().is_empty());
    assert_eq!(fs::read(root.join("poster.jpg")).unwrap(), b"old poster");
    assert_eq!(fs::read(root.join(&original)).unwrap(), bytes);
}
