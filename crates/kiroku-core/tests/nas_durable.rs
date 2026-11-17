//! Opt-in real filesystem regression. Uses only a fresh temporary directory.
use kiroku_core::{journal::Journal,organize::{Op,Plan}};
use std::fs;
#[test]
#[ignore = "requires KIROKU_NAS_TEST_ROOT pointing to a mounted test parent"]
fn mounted_filesystem_commit_reload_and_rollback() {
 let parent=std::env::var_os("KIROKU_NAS_TEST_ROOT").expect("explicit test parent required");
 let dir=tempfile::Builder::new().prefix("kiroku-regression-").tempdir_in(parent).unwrap();
 let source=dir.path().join("source.mp4");let root=dir.path().join("library");let target=root.join("movie.mp4");
 fs::write(&source,b"neutral test video").unwrap();
 let mut journal=Journal::new(&root);
 let result=journal.execute(&Plan{ops:vec![Op::MoveFile{from:source.clone(),to:target.clone()},Op::WriteFile{path:root.join("movie.nfo"),content:"neutral metadata".into()}]},&mut |_|unreachable!(),&||false);
 assert!(result.error.is_none(),"{:?}",result.error);
 assert!(!source.exists());assert_eq!(fs::read(&target).unwrap(),b"neutral test video");
 let mut reloaded=Journal::load(&journal.path()).unwrap();
 assert!(reloaded.rollback().is_empty());
 assert_eq!(fs::read(&source).unwrap(),b"neutral test video");assert!(!target.exists());
}

#[test]
#[ignore = "requires KIROKU_NAS_TEST_ROOT pointing to a mounted test parent"]
fn mounted_filesystem_index_can_reopen_with_reader() {
 use kiroku_core::library::{Library,Query};
 let parent=std::env::var_os("KIROKU_NAS_TEST_ROOT").unwrap();
 let dir=tempfile::Builder::new().prefix("kiroku-index-regression-").tempdir_in(parent).unwrap();
 let lib=Library::open(dir.path()).unwrap();
 assert_eq!(lib.overview(&Query::default()).unwrap().movies,0);
 let second=Library::open(dir.path()).unwrap();
 assert_eq!(second.overview(&Query::default()).unwrap().movies,0);
}

#[tokio::test]
#[ignore = "requires KIROKU_NAS_TEST_ROOT pointing to a mounted test parent"]
async fn mounted_batch_pipeline_indexes_and_rolls_back_move_and_copy() {
 use kiroku_core::{pipeline,video_filter::VideoFilter,organize::PlanOpts,placement::OrganizeMode,Metadata,net::Http,library::Library};
 use tokio_util::sync::CancellationToken;
 let parent=std::env::var_os("KIROKU_NAS_TEST_ROOT").unwrap();
 for mode in [OrganizeMode::Move,OrganizeMode::Copy] {
  let dir=tempfile::Builder::new().prefix("kiroku-batch-regression-").tempdir_in(&parent).unwrap();
  let source=dir.path().join("source");let library=dir.path().join("library");fs::create_dir(&source).unwrap();fs::create_dir(&library).unwrap();
  let mut metas=std::collections::HashMap::new();
  for code in ["ABC-001","ABC-002","ABC-003"] {fs::write(source.join(format!("{code}.mp4")),b"neutral video").unwrap();metas.insert(code.into(),Metadata{code:code.into(),..Default::default()});}
  let cancel=CancellationToken::new();
  let scan=pipeline::scan(&[source.clone()],Some(&library),&VideoFilter::none(),&cancel,&mut |_|{});
  let pairs=scan.items.iter().map(|i|(i,i.code.as_ref().unwrap())).collect::<Vec<_>>();
  let plans=pipeline::build_plans(&pairs,&metas,&library,"{code}",PlanOpts{mode,..Default::default()});
  let mut out=pipeline::apply(plans,&Http::new(std::time::Duration::ZERO,None).unwrap(),&library,None,&cancel,&|_|{}).await;
  assert!(out.failed.is_empty(),"{:?}",out.failed);assert_eq!(out.applied.len(),3);
  let lib=Library::open(&library).unwrap();
  for code in metas.keys(){lib.reindex_dir(&library.join(code)).unwrap();assert!(lib.get(code).unwrap().is_some());}
  assert!(out.journal.rollback().is_empty());
  for code in metas.keys(){assert_eq!(fs::read(source.join(format!("{code}.mp4"))).unwrap(),b"neutral video");}
 }
}
