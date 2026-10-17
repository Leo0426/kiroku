use kiroku_core::{journal::{HistoryCache,Journal,JournalStatus,ReplacementCheckpoint,ReplacementInput},organize::{Op,Plan}};
use sha2::{Digest,Sha256};
use std::{fs,io};
use tokio_util::sync::CancellationToken;
fn original(root:&std::path::Path)->(Journal,std::path::PathBuf){let path=root.join("movie.nfo");let mut j=Journal::new(root);assert!(j.execute(&Plan{ops:vec![Op::WriteFile{path:path.clone(),content:"before".into()}]},&mut |_|unreachable!(),&||false).error.is_none());j.save().unwrap();(j,path)}
fn input(path:std::path::PathBuf)->ReplacementInput{ReplacementInput{path,expected:Some(format!("{:x}",Sha256::digest(b"before"))),bytes:b"after".to_vec()}}
#[test]
fn display_cache_never_authorizes_recovery_or_a_stale_before_owner(){
    let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let (mut imported,path)=original(&root);
    let mut cache=HistoryCache::default();assert!(cache.read(&root,&CancellationToken::new()).unwrap()[0].error.is_none());
    let bytes=fs::read(imported.path()).unwrap();let mut false_log=imported.clone();false_log.proofs.get_mut(&0).unwrap().digest="fake".into();false_log.save().unwrap();
    let mut revised=Journal::new(&root);revised.replace_files(&[input(path.clone())]).unwrap();assert!(revised.replacements[0].before_owner.is_none());
    assert!(revised.rollback().is_empty());assert_eq!(fs::read(&path).unwrap(),b"before");
    fs::write(imported.path(),b"corrupt").unwrap();assert!(Journal::load(&imported.path()).is_err());assert!(cache.read(&root,&CancellationToken::new()).unwrap().iter().any(|s|s.error.is_some()));
    fs::write(imported.path(),bytes).unwrap();imported=Journal::load(&imported.path()).unwrap();let errors=imported.rollback();assert!(!errors.is_empty(),"an independent restored copy must not gain the old ownership inode");assert_eq!(fs::read(&path).unwrap(),b"before");
}
#[test]
fn every_new_discovery_sees_rolled_back_and_repaired_ownership(){
    let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let (mut imported,path)=original(&root);
    let mut first=Journal::new(&root);first.replace_files(&[input(path.clone())]).unwrap();assert!(first.replacements[0].before_owner.is_some());assert!(first.rollback().is_empty());
    let mut next=Journal::new(&root);next.replace_files(&[input(path.clone())]).unwrap();assert!(next.replacements[0].before_owner.is_some());assert!(next.rollback().is_empty());assert!(imported.rollback().is_empty());
}
#[test]
fn cancelling_discovery_preserves_targets_and_creates_no_new_log(){
    let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let (_imported,path)=original(&root);let mut revised=Journal::new(&root);
    let cancel=CancellationToken::new();cancel.cancel();assert_eq!(revised.replace_files_cancellable(&[input(path.clone())],&cancel).unwrap_err().kind(),io::ErrorKind::Interrupted);
    assert_eq!(fs::read(path).unwrap(),b"before");assert!(!revised.path().exists());
}
#[test]
fn dropout_before_intent_does_not_create_a_shadow_root(){
    let t=tempfile::tempdir().unwrap();let root=t.path().join("library");fs::create_dir(&root).unwrap();let root=root.canonicalize().unwrap();let (_imported,path)=original(&root);let mut revised=Journal::new(&root);
    let moved=t.path().join("detached");
    let result=revised.replace_files_observed(&[input(path.clone())],&mut |point|{if point==ReplacementCheckpoint::Intent{fs::rename(&root,&moved)?;}Ok(())});
    assert!(result.is_err());assert!(!root.exists(),"{result:?}");assert_eq!(fs::read(moved.join("movie.nfo")).unwrap(),b"before");
}
#[test]
fn display_cache_refreshes_after_deletion_root_switch_and_root_replacement(){
    let a=tempfile::tempdir().unwrap();let b=tempfile::tempdir().unwrap();let mut cache=HistoryCache::default();let cancel=CancellationToken::new();let j=Journal::new(a.path());j.save().unwrap();
    assert_eq!(cache.read(a.path(),&cancel).unwrap().len(),1);assert!(cache.read(b.path(),&cancel).unwrap().is_empty());assert_eq!(cache.read(a.path(),&cancel).unwrap().len(),1);
    fs::remove_file(j.path()).unwrap();assert!(cache.read(a.path(),&cancel).unwrap().is_empty());assert_eq!(cache.retained().0,0);
    let root=b.path().join("library");fs::create_dir(&root).unwrap();let old=Journal::new(&root);old.save().unwrap();cache.read(&root,&cancel).unwrap();fs::rename(&root,b.path().join("removed")).unwrap();
    assert!(cache.read(&root,&cancel).is_err());assert!(!root.exists());fs::create_dir(&root).unwrap();assert!(cache.read(&root,&cancel).unwrap().is_empty());
}
#[test]
fn summary_semantics_match_full_inspection_for_pending_and_corrupt_records(){
    let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let (imported,path)=original(&root);let mut revised=Journal::new(&root);
    let _=revised.replace_files_observed(&[input(path)],&mut |point|if point==ReplacementCheckpoint::Intent{Err(io::Error::other("interrupt"))}else{Ok(())});
    fs::write(root.join(".kiroku/journal-1.json"),b"bad").unwrap();
    let rows=HistoryCache::default().read(&root,&CancellationToken::new()).unwrap();assert_eq!(rows.len(),3);
    let summary=rows.iter().find(|r|r.id==revised.id).unwrap();assert_eq!(summary.status,JournalStatus::Partial);assert_eq!(summary.pending_operations,1);
    assert!(rows.iter().find(|r|r.id==imported.id).unwrap().error.is_none());assert!(rows.iter().find(|r|r.id=="1").unwrap().error.is_some());
}

#[test]
fn invalid_owner_id_is_rejected_before_any_replacement_intent_is_written(){
 let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();let path=root.join("movie.nfo");let mut imported=Journal::new(&root);imported.id="invalid-owner".into();
 assert!(imported.execute(&Plan{ops:vec![Op::WriteFile{path:path.clone(),content:"before".into()}]},&mut |_|unreachable!(),&||false).error.is_none());imported.save().unwrap();
 let mut revised=Journal::new(&root);assert!(revised.replace_files(&[input(path.clone())]).is_err());assert!(!revised.path().exists());assert_eq!(fs::read(path).unwrap(),b"before");
}

#[test]
fn stale_loaded_pending_journal_never_removes_a_replacement_librarys_stage(){
 let t=tempfile::tempdir().unwrap();let root=t.path().join("library");fs::create_dir(&root).unwrap();let root=root.canonicalize().unwrap();let source=t.path().join("source.mp4");fs::write(&source,b"video").unwrap();let mut j=Journal::new(&root);
 let out=j.execute_observed(&Plan{ops:vec![Op::MoveFile{from:source.clone(),to:root.join("video.mp4")}]},&mut |_|unreachable!(),&||false,&mut |point|if point==kiroku_core::journal::Checkpoint::Staged{Err(io::Error::other("stop"))}else{Ok(())});assert!(out.error.is_some());
 let mut loaded=Journal::load(&j.path()).unwrap();let pending=loaded.pending.as_ref().unwrap().clone();fs::rename(&root,t.path().join("detached")).unwrap();fs::create_dir(&root).unwrap();
 let foreign_stage=root.join(pending.proof.stage.file_name().unwrap());fs::write(&foreign_stage,b"video").unwrap();assert!(loaded.recover_pending().is_err());assert!(foreign_stage.exists());assert!(source.exists());assert!(!root.join(".kiroku").exists());
}
