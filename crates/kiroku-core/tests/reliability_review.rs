use kiroku_core::{
    append,
    journal::{Journal, JournalStatus},
    maintenance::{self, RetentionPolicy},
    organize::{Op, PlanOpts},
    pipeline, Metadata,
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;
fn movie(root: &Path) -> PathBuf {
    let d = root.join("manual");
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
    d
}
fn plans(root: &Path, incoming: &Path) -> Vec<pipeline::PlanItem> {
    let scan = pipeline::scan(
        &[incoming.to_path_buf()],
        Some(root),
        &Default::default(),
        &CancellationToken::new(),
        &mut |_| {},
    );
    let pairs: Vec<_> = scan
        .items
        .iter()
        .map(|i| (i, i.code.as_ref().unwrap()))
        .collect();
    pipeline::build_plans(
        &pairs,
        &HashMap::from([(
            "ABC-123".into(),
            Metadata {
                code: "ABC-123".into(),
                ..Default::default()
            },
        )]),
        root,
        "{code}",
        PlanOpts::default(),
    )
}
#[test]
fn discovery_errors_never_become_new_archives() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    movie(&root);
    let bad = root.join("corrupt");
    fs::create_dir(&bad).unwrap();
    fs::write(bad.join("kiroku.json"), b"bad json").unwrap();
    let incoming = t.path().join("incoming");
    fs::create_dir(&incoming).unwrap();
    fs::write(incoming.join("ABC-123-CD2.mp4"), b"two").unwrap();
    let p = plans(&root, &incoming);
    assert!(!p[0].selected);
    assert!(p[0].conflict.is_some());
    assert!(p[0].ops.is_empty());
    let p = plans(&t.path().join("offline"), &incoming);
    assert!(!p[0].selected);
    assert!(p[0].ops.is_empty());
}
#[test]
fn folder_and_multi_selection_pair_new_part_subtitles() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    movie(&root);
    let incoming = t.path().join("incoming");
    fs::create_dir(&incoming).unwrap();
    let video = incoming.join("ABC-123-CD2.mp4");
    let sub = incoming.join("ABC-123-CD2.chs.srt");
    fs::write(&video, b"two").unwrap();
    fs::write(&sub, b"subtitle").unwrap();
    for input in [vec![incoming.clone()], vec![sub.clone(), video.clone()]] {
        let p = append::preview(&root, "ABC-123", &input, None).unwrap();
        assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
        assert_eq!(p.ops.len(), 2);
    }
}
#[test]
fn all_siblings_derive_from_the_video_stem() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    movie(&root);
    let video = t.path().join("ABC-123-CD2.mp4");
    fs::write(&video, b"two").unwrap();
    for suffix in [".chs.idx", ".chs.sub", ".en.srt"] {
        fs::write(t.path().join(format!("ABC-123-CD2{suffix}")), b"sub").unwrap();
    }
    let p = append::preview(&root, "ABC-123", &[video], None).unwrap();
    let names = p
        .ops
        .iter()
        .map(|op| {
            op.target()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    for name in [
        "ABC-123-CD2.chs.idx",
        "ABC-123-CD2.chs.sub",
        "ABC-123-CD2.en.srt",
    ] {
        assert!(names.contains(&name.to_owned()), "{names:?}")
    }
}
#[test]
fn maintenance_keeps_completed_counts_when_a_later_path_disappears() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let d = movie(&root);
    let mut j = Journal::new(&root);
    j.ts = 0;
    j.status = JournalStatus::RolledBack;
    j.ops = vec![Op::MoveFile {
        from: t.path().join("source.mp4"),
        to: d.join("old.mp4"),
    }];
    j.rolled_back_ops = vec![0];
    let stage = d.join(format!(".kiroku-{}-0.tmp", j.id));
    fs::write(&stage, b"stage").unwrap();
    j.proofs.insert(
        0,
        kiroku_core::journal::Proof {
            link: None, source_proof: None,
            stage: stage.clone(),
            digest: "previous".into(),
        },
    );
    j.save().unwrap();
    let p = maintenance::preview(
        &root,
        &t.path().join("logs"),
        &RetentionPolicy {
            history_days: 0,
            log_days: 0,
        },
    )
    .unwrap();
    let c = p.candidates.iter().find(|c| c.kind == "history").unwrap();
    let log = j.path();
    let r = maintenance::apply(
        &p,
        &[c.id.clone()],
        &|path| {
            fs::remove_file(path)?;
            fs::remove_file(&log)?;
            Ok(())
        },
        &CancellationToken::new(),
    );
    let r = r.expect("必须返回部分结果");
    assert_eq!(r.removed, 1);
    assert!(!r.failed.is_empty() || !r.skipped.is_empty());
}
#[test]
fn video_choices_use_the_explicit_selected_disk_target() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let a = movie(&root);
    let b = root.join("second");
    fs::create_dir(&b).unwrap();
    fs::copy(a.join("kiroku.json"), b.join("kiroku.json")).unwrap();
    fs::write(b.join("ABC-123-CD2.mp4"), b"two").unwrap();
    let files = append::target_videos(&root, "ABC-123", &b).unwrap();
    assert_eq!(
        files,
        vec![b.canonicalize().unwrap().join("ABC-123-CD2.mp4")]
    );
}
#[tokio::test]
async fn vanished_root_after_preview_is_not_recreated_by_apply() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let incoming = t.path().join("incoming");
    fs::create_dir(&incoming).unwrap();
    let input = incoming.join("ABC-123.mp4");
    fs::write(&input, b"video").unwrap();
    let p = plans(&root, &incoming);
    assert!(p[0].selected);
    fs::remove_dir(&root).unwrap();
    let http = kiroku_core::net::Http::new(std::time::Duration::ZERO, None).unwrap();
    let r = pipeline::apply(p, &http, &root, None, &CancellationToken::new(), &|_| {}).await;
    assert!(!r.failed.is_empty());
    assert!(!root.exists());
    assert!(input.exists());
}
#[test]
fn missing_new_library_requires_explicit_creation_intent() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("new-library");
    let input = t.path().join("ABC-123.mp4");
    fs::write(&input, b"video").unwrap();
    let scan = pipeline::scan(
        &[t.path().to_path_buf()],
        Some(&root),
        &Default::default(),
        &CancellationToken::new(),
        &mut |_| {},
    );
    let item = &scan.items[0];
    let p = pipeline::build_plans(
        &[(item, item.code.as_ref().unwrap())],
        &HashMap::from([(
            "ABC-123".into(),
            Metadata {
                code: "ABC-123".into(),
                ..Default::default()
            },
        )]),
        &root,
        "{code}",
        PlanOpts {
            create_library: true,
            ..Default::default()
        },
    );
    assert!(p[0].selected);
    assert!(!root.exists(), "预览不创建目录");
}
#[tokio::test]
async fn root_lost_during_image_preparation_is_not_recreated_by_final_save() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    fs::create_dir(&root).unwrap();
    let incoming = t.path().join("incoming");
    fs::create_dir(&incoming).unwrap();
    let input = incoming.join("ABC-123.mp4");
    fs::write(&input, b"video").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/cover.jpg", listener.local_addr().unwrap());
    let removed = root.clone();
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut b = [0; 4096];
        stream.read(&mut b).unwrap();
        fs::remove_dir(&removed).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nbad")
            .unwrap();
    });
    let scan = pipeline::scan(
        &[incoming],
        Some(&root),
        &Default::default(),
        &CancellationToken::new(),
        &mut |_| {},
    );
    let item = &scan.items[0];
    let mut meta = Metadata {
        code: "ABC-123".into(),
        ..Default::default()
    };
    meta.fields.poster_urls = vec![url];
    let p = pipeline::build_plans(
        &[(item, item.code.as_ref().unwrap())],
        &HashMap::from([("ABC-123".into(), meta)]),
        &root,
        "{code}",
        PlanOpts::default(),
    );
    let http = kiroku_core::net::Http::new(std::time::Duration::ZERO, None).unwrap();
    let r = pipeline::apply(p, &http, &root, None, &CancellationToken::new(), &|_| {}).await;
    peer.join().unwrap();
    assert!(!r.failed.is_empty());
    assert!(!root.exists(), "日志末尾不能重建掉线库");
    assert!(input.exists());
}
#[test]
fn unavailable_library_root_is_not_a_deleted_movie_for_reindex() {
    use kiroku_core::library::{Library, Query};
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("lib");
    let dir = movie(&root);
    let lib = Library::open(&root).unwrap();
    lib.reindex_dir(&dir).unwrap();
    fs::rename(&root, t.path().join("disconnected")).unwrap();
    assert!(lib.reindex_dir(&dir).is_err());
    assert_eq!(lib.list(&Query::default()).unwrap().total, 1);
    assert!(!root.exists());
}
