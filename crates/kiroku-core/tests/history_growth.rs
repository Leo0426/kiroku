//! Run each ignored measurement alone, without other I/O workloads.
use kiroku_core::{journal::{HistoryCache,Journal},organize::Op};
use std::{alloc::{GlobalAlloc,Layout,System},sync::atomic::{AtomicUsize,Ordering},time::Instant};
use tokio_util::sync::CancellationToken;
struct Heap;static LIVE:AtomicUsize=AtomicUsize::new(0);static PEAK:AtomicUsize=AtomicUsize::new(0);
unsafe impl GlobalAlloc for Heap{unsafe fn alloc(&self,l:Layout)->*mut u8{let p=unsafe{System.alloc(l)};if !p.is_null(){let n=LIVE.fetch_add(l.size(),Ordering::Relaxed)+l.size();PEAK.fetch_max(n,Ordering::Relaxed);}p}unsafe fn dealloc(&self,p:*mut u8,l:Layout){unsafe{System.dealloc(p,l)};LIVE.fetch_sub(l.size(),Ordering::Relaxed);}}
#[global_allocator]static ALLOCATOR:Heap=Heap;
fn measure<T>(f:impl FnOnce()->T)->(T,usize,usize,u128){let base=LIVE.load(Ordering::Relaxed);PEAK.store(base,Ordering::Relaxed);let start=Instant::now();let value=f();(value,LIVE.load(Ordering::Relaxed).saturating_sub(base),PEAK.load(Ordering::Relaxed).saturating_sub(base),start.elapsed().as_millis())}
#[test]
#[ignore="full inspection versus display summary heap and elapsed growth"]
fn history_heap_growth(){
 for (histories,ops) in [(10,100),(100,1000),(500,1000)] {
  let t=tempfile::tempdir().unwrap();for h in 0..histories{let mut j=Journal::new(t.path());j.push_ops((0..ops).map(|i|Op::WriteFile{path:t.path().join(format!("{h}-{i}.nfo")),content:String::new()}));j.save().unwrap();}
  let (full,full_live,full_peak,full_ms)=measure(||Journal::inspect(t.path()).unwrap());let ids=full.iter().map(|e|e.id.clone()).collect::<Vec<_>>();drop(full);
  let mut cache=HistoryCache::default();let cancel=CancellationToken::new();
  let (cold,cold_live,cold_peak,cold_ms)=measure(||cache.read(t.path(),&cancel).unwrap());assert_eq!(cold.iter().map(|s|s.id.clone()).collect::<Vec<_>>(),ids);drop(cold);
  let (hot,hot_live,hot_peak,hot_ms)=measure(||cache.read(t.path(),&cancel).unwrap());assert_eq!(hot.len(),histories);assert!(cache.retained().1<=4*1024*1024);
  assert!(cold_peak<full_peak,"cold {cold_peak} full {full_peak}");
  println!("history-heap n={histories} ops={ops} full={full_live}B/{full_peak}B/{full_ms}ms cold={cold_live}B/{cold_peak}B/{cold_ms}ms hot={hot_live}B/{hot_peak}B/{hot_ms}ms retained={:?}",cache.retained());
 }
}

#[test]
#[ignore="actual replacement discovery heap growth; run alone"]
fn ownership_heap_growth(){
 use kiroku_core::{journal::ReplacementInput,organize::Plan};use sha2::{Digest,Sha256};
 for (histories,ops,inputs) in [(10,100,10),(100,1000,20),(500,1000,20)] {
  let t=tempfile::tempdir().unwrap();let root=t.path().canonicalize().unwrap();
  for h in 0..histories{let mut j=Journal::new(&root);j.push_ops((0..ops).map(|i|Op::WriteFile{path:root.join(format!("unused-{h}-{i}.nfo")),content:String::new()}));j.save().unwrap();}
  let mut owned=Journal::new(&root);let plan=Plan{ops:(0..inputs).map(|i|Op::WriteFile{path:root.join(format!("target-{i}.nfo")),content:"before".into()}).collect()};assert!(owned.execute(&plan,&mut |_|unreachable!(),&||false).error.is_none());owned.save().unwrap();
  let (baseline,old_live,old_peak,old_ms)=measure(||Journal::list(&root));assert_eq!(baseline.len(),histories+1);drop(baseline);
  let requests=plan.ops.iter().map(|op|ReplacementInput{path:op.target().to_path_buf(),expected:Some(format!("{:x}",Sha256::digest(b"before"))),bytes:b"after".to_vec()}).collect::<Vec<_>>();
  let mut revised=Journal::new(&root);let (_,live,peak,ms)=measure(||revised.replace_files(&requests).unwrap());assert!(revised.replacements.iter().all(|r|r.before_owner.is_some()));if histories>=100{assert!(peak<old_peak/2);}
  println!("owner-heap histories={histories} ops={ops} inputs={inputs} old-list={old_live}B/{old_peak}B/{old_ms}ms replacement={live}B/{peak}B/{ms}ms");
  assert!(revised.rollback().is_empty());for (i,op) in plan.ops.iter().enumerate(){assert!(same_file::is_same_file(op.target(),&owned.proofs[&i].stage).unwrap());}assert!(owned.rollback().is_empty());
 }
}
