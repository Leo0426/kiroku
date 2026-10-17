use kiroku_core::{journal::{Checkpoint,Journal,JournalStatus},organize::{Op,Plan}};
use std::{fs,io::{self,Write},path::Path};

fn moves(root:&Path,count:usize)->(Journal,Plan){
    let lib=root.join("library");let mut ops=vec![];
    for i in 0..count {let from=root.join(format!("{i}.mp4"));fs::write(&from,format!("video-{i}")).unwrap();ops.push(Op::MoveFile{from,to:lib.join(format!("{i}.mp4"))});}
    (Journal::new(&lib),Plan{ops})
}
fn record_starts(bytes:&[u8])->Vec<usize>{
    let mut pos=b"KIROKU-JOURNAL\x01\n".len();let mut starts=vec![];
    while pos<bytes.len(){starts.push(pos);let size=u64::from_le_bytes(bytes[pos..pos+8].try_into().unwrap()) as usize;pos+=72+size;}
    assert_eq!(pos,bytes.len());starts
}
#[test]
fn torn_last_commit_recovers_durable_intent_and_rolls_back_every_completed_item(){
    for tail in [0,1,8,40,71,72,100] {
        let t=tempfile::tempdir().unwrap();let (mut j,plan)=moves(t.path(),2);
        assert!(j.execute(&plan,&mut |_|unreachable!(),&||false).error.is_none());
        let mut bytes=fs::read(j.path()).unwrap();let start=*record_starts(&bytes).last().unwrap();bytes.truncate(start+tail);fs::write(j.path(),bytes).unwrap();
        let mut loaded=Journal::load(&j.path()).unwrap();assert_eq!(loaded.status,JournalStatus::Partial);assert!(loaded.pending.is_some());
        assert!(loaded.rollback().is_empty(),"cut={tail}");
        for i in 0..2 {assert_eq!(fs::read(t.path().join(format!("{i}.mp4"))).unwrap(),format!("video-{i}").as_bytes());assert!(!t.path().join(format!("library/{i}.mp4")).exists());}
        assert_eq!(Journal::load(&j.path()).unwrap().status,JournalStatus::RolledBack);
        assert!(serde_json::from_slice::<Journal>(&fs::read(j.path()).unwrap()).is_ok(),"terminal rollback is classic JSON");
    }
}
#[test]
fn complete_record_or_length_corruption_is_reported_and_never_authorizes_undo(){
    for corrupt_header in [false,true]{
        let t=tempfile::tempdir().unwrap();let (mut j,plan)=moves(t.path(),1);assert!(j.execute(&plan,&mut |_|unreachable!(),&||false).error.is_none());
        let mut bytes=fs::read(j.path()).unwrap();let start=*record_starts(&bytes).last().unwrap();let at=if corrupt_header{start}else{bytes.len()-1};bytes[at]^=1;fs::write(j.path(),bytes).unwrap();
        assert_eq!(Journal::load(&j.path()).unwrap_err().kind(),io::ErrorKind::InvalidData);
        let history=Journal::inspect(&j.root).unwrap();assert_eq!(history.len(),1);assert!(history[0].error.is_some());assert!(history[0].journal.is_none());assert!(plan.ops[0].target().exists());
    }
}
#[test]
fn stale_clone_cannot_replace_newer_item_progress(){
    let t=tempfile::tempdir().unwrap();let (mut j,plan)=moves(t.path(),2);
    assert!(j.execute(&Plan{ops:vec![plan.ops[0].clone()]},&mut |_|unreachable!(),&||false).error.is_none());
    let stale=j.clone();assert!(j.execute(&Plan{ops:vec![plan.ops[1].clone()]},&mut |_|unreachable!(),&||false).error.is_none());
    assert!(stale.save().is_err());assert_eq!(Journal::load(&j.path()).unwrap().ops.len(),2);
    assert!(j.rollback().is_empty());
}
#[test]
fn interrupted_append_stops_before_next_item_and_reload_can_resume_rollback(){
    let t=tempfile::tempdir().unwrap();let (mut j,plan)=moves(t.path(),2);let path=j.path();
    let out=j.execute_observed(&plan,&mut |_|unreachable!(),&||false,&mut |point| {
        if point==Checkpoint::Published {fs::OpenOptions::new().append(true).open(&path)?.write_all(b"torn")?;}Ok(())
    });
    assert!(out.error.is_some());assert!(!plan.ops[1].target().exists());
    let mut loaded=Journal::load(&path).unwrap();assert!(loaded.pending.is_some());assert!(loaded.rollback().is_empty());assert!(t.path().join("0.mp4").exists());assert!(t.path().join("1.mp4").exists());
}
#[test]
fn classic_object_and_bare_vector_logs_remain_readable_and_undoable(){
    for bare in [false,true] {
        let t=tempfile::tempdir().unwrap();let mut j=Journal::new(t.path());
        let generated=t.path().join("movie.nfo");fs::write(&generated,b"metadata").unwrap();j.push_ops([Op::WriteFile{path:generated.clone(),content:String::new()}]);j.version=1;j.save().unwrap();
        if bare {fs::write(j.path(),serde_json::to_vec(&j.ops).unwrap()).unwrap();}
        let mut loaded=Journal::load(&j.path()).unwrap();
        // A leftover regular old atomic-write temp is reusable, not a permanent recovery block.
        fs::write(j.path().with_extension("json.tmp"),b"old interrupted snapshot").unwrap();
        assert!(loaded.rollback().is_empty());assert!(!generated.exists());assert_eq!(Journal::load(&j.path()).unwrap().status,JournalStatus::RolledBack);
    }
}

#[test]
fn repeated_item_checkpoints_have_linear_persisted_growth_and_remain_reloadable() {
    use kiroku_core::organize::Plan;
    let root=tempfile::tempdir().unwrap();let mut j=Journal::new(root.path());
    let mut cumulative=0u64;
    for i in 0..100 {
        let previous=std::fs::read(j.path()).unwrap_or_default();
        let before=previous.len() as u64;
        let result=j.execute(&Plan{ops:vec![Op::WriteFile{path:root.path().join(format!("{i}.nfo")),content:"metadata".into()}]},&mut |_|unreachable!(),&||false);
        assert!(result.error.is_none());
        let bytes=std::fs::read(j.path()).unwrap();let size=bytes.len() as u64;
        assert!(bytes.starts_with(&previous),"item checkpoint must retain the preceding durable records");
        cumulative+=size.saturating_sub(before);
        assert_eq!(Journal::load(&j.path()).unwrap(),j);
        // A complete second checkpoint may add only this item's intent and commit.
        assert!(size-before<4096,"item {i} rewrote history: {size} vs {before}");
    }
    assert!(cumulative<400_000);
    assert!(j.rollback().is_empty());assert_eq!(Journal::load(&j.path()).unwrap(),j);
}

#[cfg(unix)]
#[test]
fn linked_atomic_temp_never_truncates_the_durable_journal_or_foreign_data(){
    for shared in [false,true] {
        let t=tempfile::tempdir().unwrap();let generated=t.path().join("movie.nfo");fs::write(&generated,b"nfo").unwrap();
        let mut j=Journal::new(t.path());j.push_ops([Op::WriteFile{path:generated.clone(),content:String::new()}]);j.save().unwrap();
        let before=fs::read(j.path()).unwrap();let tmp=j.path().with_extension("json.tmp");let foreign=t.path().join("foreign");fs::write(&foreign,b"user content").unwrap();
        if shared {fs::hard_link(j.path(),&tmp).unwrap();}else{std::os::unix::fs::symlink(&foreign,&tmp).unwrap();}
        let mut loaded=Journal::load(&j.path()).unwrap();assert!(!loaded.rollback().is_empty());assert!(generated.exists());
        assert_eq!(fs::read(j.path()).unwrap(),before);assert_eq!(fs::read(foreign).unwrap(),b"user content");
        fs::remove_file(tmp).unwrap();assert!(loaded.rollback().is_empty());
    }
}

#[cfg(unix)]
#[test]
fn reading_many_histories_does_not_hold_a_descriptor_per_journal(){
    if std::env::var_os("KIROKU_JOURNAL_FD_CHILD").is_none(){
        let output=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","reading_many_histories_does_not_hold_a_descriptor_per_journal","--test-threads=1","--nocapture"]).env("KIROKU_JOURNAL_FD_CHILD","1").output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stdout));return;
    }
    let root=tempfile::tempdir().unwrap();
    for _ in 0..250 {Journal::new(root.path()).save().unwrap();}
    let descriptors=||fs::read_dir("/dev/fd").unwrap().count();
    let before=descriptors();let histories=Journal::inspect(root.path()).unwrap();let after=descriptors();
    assert_eq!(histories.len(),250);assert!(histories.iter().all(|h|h.journal.is_some()));
    println!("history descriptors before={before} after={after}");
    assert!(after<=before+8,"historical models retained {} file descriptors",after-before);
}
