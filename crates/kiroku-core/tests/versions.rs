//! ADR-0008:同番号同分片、内容不同的视频作为新版本并存。
use kiroku_core::{append, journal::Journal, organize::Op, Metadata};
use std::{fs, path::{Path, PathBuf}};
use tokio_util::sync::CancellationToken;

fn library(t: &Path, videos: &[(&str, &[u8])]) -> (PathBuf, PathBuf) {
    let root = t.join("lib");
    let dir = root.join("ABC-123");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("kiroku.json"), serde_json::to_vec(&Metadata { code: "ABC-123".into(), ..Default::default() }).unwrap()).unwrap();
    for (name, bytes) in videos {
        fs::write(dir.join(name), bytes).unwrap();
    }
    (root, dir.canonicalize().unwrap())
}

fn targets(p: &append::AppendPreview) -> Vec<String> {
    p.ops.iter().map(|o| o.target().file_name().unwrap().to_string_lossy().into_owned()).collect()
}

#[test]
fn a_subtitled_version_coexists_with_its_subtitle_and_rolls_back() {
    let t = tempfile::tempdir().unwrap();
    let (root, dir) = library(t.path(), &[("ABC-123.mp4", b"original")]);
    let input = t.path().join("in").join("ABC-123-C.mp4");
    fs::create_dir_all(input.parent().unwrap()).unwrap();
    fs::write(&input, b"subtitled").unwrap();
    fs::write(input.with_file_name("ABC-123-C.chs.srt"), b"srt").unwrap();
    let p = append::preview(&root, "ABC-123", &[input.clone()], None).unwrap();
    assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
    assert_eq!(targets(&p), ["ABC-123 - 中字.mp4", "ABC-123 - 中字.chs.srt"]);
    assert_eq!(p.new_versions, [dir.join("ABC-123 - 中字.mp4")]);
    let r = append::apply(&p, &CancellationToken::new()).unwrap();
    assert!(r.failed.is_empty());
    assert_eq!(fs::read(dir.join("ABC-123.mp4")).unwrap(), b"original", "旧版本不动");
    assert_eq!(fs::read(dir.join("ABC-123 - 中字.mp4")).unwrap(), b"subtitled");
    let mut j = Journal::load(&root.join(".kiroku").join(format!("journal-{}.json", r.journal_id))).unwrap();
    assert!(j.rollback().is_empty());
    assert!(input.exists() && !dir.join("ABC-123 - 中字.mp4").exists());
}

#[test]
fn labels_combine_phases_and_fall_back_to_numbered_versions() {
    let t = tempfile::tempdir().unwrap();
    let (root, _) = library(t.path(), &[("ABC-123.mp4", b"original"), ("ABC-123 - v2.mp4", b"second")]);
    let fourk = t.path().join("ABC-123-UC 4K.mp4");
    fs::write(&fourk, b"4k").unwrap();
    let p = append::preview(&root, "ABC-123", &[fourk], None).unwrap();
    assert_eq!(targets(&p), ["ABC-123 - 4K 中字 破解.mp4"]);

    let plain = t.path().join("plain").join("ABC-123.mp4");
    fs::create_dir_all(plain.parent().unwrap()).unwrap();
    fs::write(&plain, b"third").unwrap();
    let p = append::preview(&root, "ABC-123", &[plain], None).unwrap();
    assert_eq!(targets(&p), ["ABC-123 - v3.mp4"], "v2 已被占用");
}

#[test]
fn bytes_identical_to_any_existing_version_are_skipped() {
    let t = tempfile::tempdir().unwrap();
    let (root, _) = library(t.path(), &[("ABC-123.mp4", b"original"), ("ABC-123 - 中字.mp4", b"subtitled")]);
    let input = t.path().join("ABC-123-C.mp4");
    fs::write(&input, b"subtitled").unwrap();
    let p = append::preview(&root, "ABC-123", &[input.clone()], None).unwrap();
    assert!(p.ops.is_empty() && p.conflicts.is_empty(), "{:?} {:?}", p.ops, p.conflicts);
    assert_eq!(p.skipped.len(), 1);
}

#[test]
fn parts_keep_their_own_stem() {
    let t = tempfile::tempdir().unwrap();
    let (root, _) = library(t.path(), &[("ABC-123-CD1.mp4", b"one"), ("ABC-123-CD2.mp4", b"two")]);
    let input = t.path().join("ABC-123-CD1.mp4");
    fs::write(&input, b"better one").unwrap();
    let p = append::preview(&root, "ABC-123", &[input], None).unwrap();
    assert_eq!(targets(&p), ["ABC-123-CD1 - v2.mp4"]);
}

#[test]
fn import_plans_new_versions_unselected_with_a_notice() {
    use kiroku_core::{code::parse_code, organize::PlanOpts, pipeline};
    use std::collections::HashMap;
    let t = tempfile::tempdir().unwrap();
    let (root, dir) = library(t.path(), &[("ABC-123.mp4", b"original")]);
    let incoming = t.path().join("in");
    fs::create_dir_all(&incoming).unwrap();
    fs::write(incoming.join("ABC-123-C.mp4"), b"subtitled").unwrap();
    let scan = pipeline::scan(&[incoming.clone()], Some(&root), &Default::default(), &CancellationToken::new(), &mut |_| {});
    let item = &scan.items[0];
    let c = parse_code("ABC-123-C.mp4").unwrap();
    let meta = Metadata { code: "ABC-123".into(), ..Default::default() };
    let plans = pipeline::build_plans(&[(item, &c)], &HashMap::from([(meta.code.clone(), meta)]), &root, "{code}", PlanOpts::default());
    let p = &plans[0];
    assert_eq!(p.conflict, None);
    assert!(!p.selected, "新版本默认不勾选");
    assert_eq!(p.to, dir.join("ABC-123 - 中字.mp4"));
    assert!(p.notices.iter().any(|n| n.contains("新版本")), "{:?}", p.notices);
    assert!(matches!(&p.ops[..], [Op::MoveFile { .. }]));
}
