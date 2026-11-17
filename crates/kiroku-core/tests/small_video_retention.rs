use kiroku_core::{pipeline::{scan, cleanup_sources}, video_filter::{VideoFilter, ScanCfg, SkipReason}};
use std::{fs, path::Path};
use tokio_util::sync::CancellationToken;
const MB: u64 = 1024 * 1024;
fn file(path: &Path, size: u64) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::File::create(path).unwrap().set_len(size).unwrap();
}
#[test]
fn default_scan_skips_below_100_mb_but_keeps_boundary() {
    let temp = tempfile::tempdir().unwrap();
    file(&temp.path().join("ABC-001.mp4"), 100 * MB - 1);
    file(&temp.path().join("ABC-002.mp4"), 100 * MB);
    let result = scan(&[temp.path().into()], None, &VideoFilter::default(), &CancellationToken::new(), &mut |_| {});
    assert_eq!(result.items.len(), 1);
    assert!(result.items[0].path.ends_with("ABC-002.mp4"));
    assert_eq!(result.skipped[0].reason, SkipReason::TooSmall);
    assert!(temp.path().join("ABC-001.mp4").exists());
}
#[test]
fn cleanup_preserves_small_videos_even_if_they_match_other_skip_rules() {
    for name in ["ABC-002.mp4", "sample.mp4", "advert.mp4"] {
        let temp = tempfile::tempdir().unwrap();
        let pack = temp.path().join("pack");
        let small = pack.join("extras").join(name);
        file(&small, 100 * MB - 1);
        let filter = VideoFilter::new(&ScanCfg {min_video_mb: 100, blacklist: vec!["advert".into()], ..Default::default()});
        let cleaned = cleanup_sources(&[pack.join("ABC-001.mp4")], &[temp.path().into()], &filter, &|path| fs::remove_dir_all(path));
        assert!(cleaned.is_empty(), "{name} must prevent cleanup");
        assert!(small.exists());
    }
}
#[test]
fn small_video_in_sibling_prevents_ancestor_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let pack = temp.path().join("pack");
    fs::create_dir_all(pack.join("finished")).unwrap();
    let small = pack.join("keep/ABC-002.mp4");
    file(&small, 1);
    let filter = VideoFilter::new(&ScanCfg {min_video_mb: 100, ..Default::default()});
    let cleaned = cleanup_sources(&[pack.join("finished/ABC-001.mp4")], &[temp.path().into()], &filter, &|path| fs::remove_dir_all(path));
    assert_eq!(cleaned, vec![pack.join("finished")]);
    assert!(small.exists());
}

#[tokio::test]
async fn organizing_main_video_leaves_small_video_and_its_folder_untouched() {
    use kiroku_core::{pipeline::{self, Cleanup}, organize::PlanOpts, net::Http, Metadata};
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let library = temp.path().join("library");
    fs::create_dir_all(&library).unwrap();
    let main = source.join("pack/ABC-001.mp4");
    let small = source.join("pack/sample.mp4");
    file(&main, 100 * MB);
    fs::write(&small, b"preserve original bytes").unwrap();
    let filter = VideoFilter::default();
    let cancel = CancellationToken::new();
    let scanned = scan(&[source.clone()], Some(&library), &filter, &cancel, &mut |_| {});
    assert_eq!(scanned.items.len(), 1);
    let pairs: Vec<_> = scanned.items.iter().map(|item| (item, item.code.as_ref().unwrap())).collect();
    let metas = std::collections::HashMap::from([("ABC-001".into(), Metadata {code:"ABC-001".into(), ..Default::default()})]);
    let plans = pipeline::build_plans(&pairs, &metas, &library, "{code}", PlanOpts::default());
    let roots = [source];
    let outcome = pipeline::apply(plans, &Http::new(std::time::Duration::from_secs(1),None).unwrap(), &library,
        Some(Cleanup{roots:&roots, filter:&filter, trash:|path|fs::remove_dir_all(path)}), &cancel, &|_|{}).await;
    assert_eq!(outcome.applied.len(), 1, "{:?}",outcome.failed);
    assert!(outcome.cleaned.is_empty());
    assert!(outcome.journal.trashed.is_empty());
    assert!(!main.exists());
    assert_eq!(fs::read(small).unwrap(), b"preserve original bytes");
}
