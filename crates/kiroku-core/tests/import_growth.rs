//! Run growth measurements alone: cargo test -p kiroku-core --test import_growth -- --ignored --nocapture --test-threads=1
use kiroku_core::{
    journal::Journal,
    metadata::{Metadata, PartialMetadata},
    organize::{Op, PlanOpts},
    pipeline,
    video_filter::VideoFilter,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};
use tokio_util::sync::CancellationToken;

struct Heap;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}
#[global_allocator]
static ALLOCATOR: Heap = Heap;

fn measure<T>(f: impl FnOnce() -> T) -> (T, usize, usize, u128) {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let start = Instant::now();
    let value = f();
    (
        value,
        LIVE.load(Ordering::Relaxed).saturating_sub(baseline),
        PEAK.load(Ordering::Relaxed).saturating_sub(baseline),
        start.elapsed().as_millis(),
    )
}

fn journal(root: &std::path::Path, n: usize) -> Journal {
    let mut j = Journal::new(root);
    j.push_ops((0..n).map(|i| Op::MoveFile {
        from: root.join(format!("source/ABC-{i:05}.mp4")),
        to: root.join(format!("library/ABC-{i:05}/ABC-{i:05}.mp4")),
    }));
    j
}

// Running alone keeps allocation measurements independent from other test threads.
#[test]
fn saving_large_journal_has_bounded_extra_heap() {
    let root = tempfile::tempdir().unwrap();
    let j = journal(root.path(), 5_000);
    let (path, _, peak, _) = measure(|| j.save().unwrap());
    let size = std::fs::metadata(&path).unwrap().len();
    println!("save peak={peak} json={size}");
    assert!(size > 1_000_000);
    assert!(
        peak < 128 * 1024,
        "save allocated {peak} extra heap bytes for {size} JSON bytes"
    );
    assert_eq!(Journal::load(&path).unwrap(), j);
}

#[test]
#[ignore = "isolated journal-save growth measurement; run alone"]
fn synthetic_journal_save_growth() {
    for n in [100, 1_000, 5_000] {
        let root = tempfile::tempdir().unwrap();
        let (j, live, peak, ms) = measure(|| journal(root.path(), n));
        println!("n={n} journal live={live} peak={peak} ms={ms}");
        let (path, live, peak, ms) = measure(|| j.save().unwrap());
        println!(
            "n={n} save live={live} peak={peak} ms={ms} json={}",
            std::fs::metadata(path).unwrap().len()
        );
    }
}

#[test]
#[ignore = "synthetic disk and heap growth measurement; run alone"]
fn synthetic_import_growth() {
    // Warm parser caches before taking measurements. All media are one-byte temp fixtures.
    let _ = kiroku_core::code::parse_code("ABC-123.mp4");
    for n in [100, 1_000, 5_000] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let library = temp.path().join("library");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        for i in 0..n {
            std::fs::write(source.join(format!("ABC-{i:05}.mp4")), b"v").unwrap();
        }
        let filter = VideoFilter::none();
        let cancel = CancellationToken::new();
        let (scan, live, peak, ms) =
            measure(|| pipeline::scan(&[source], None, &filter, &cancel, &mut |_| {}));
        assert_eq!(scan.items.len(), n);
        assert!(scan.summary.complete);
        println!(
            "n={n} scan live={live} peak={peak} ms={ms} json={}",
            serde_json::to_vec(&scan).unwrap().len()
        );
        let pairs: Vec<_> = scan
            .items
            .iter()
            .map(|item| (item, item.code.as_ref().unwrap()))
            .collect();
        let metas: HashMap<_, _> = pairs
            .iter()
            .map(|(_, code)| {
                (
                    code.id.clone(),
                    Metadata {
                        code: code.id.clone(),
                        fields: PartialMetadata {
                            title: Some(format!("{} title", code.id)),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                )
            })
            .collect();
        let (plans, live, peak, ms) = measure(|| {
            pipeline::build_plans(&pairs, &metas, &library, "{code}", PlanOpts::default())
        });
        assert_eq!(plans.len(), n);
        assert!(plans.iter().all(|p| p.selected));
        println!(
            "n={n} plans live={live} peak={peak} ms={ms} json={} op_text={}",
            serde_json::to_vec(&plans).unwrap().len(),
            plans
                .iter()
                .flat_map(|p| &p.ops)
                .map(|op| match op {
                    Op::WriteFile { content, .. } => content.len(),
                    _ => 0,
                })
                .sum::<usize>()
        );
        let (j, live, peak, ms) = measure(|| journal(&library, n));
        println!("n={n} journal live={live} peak={peak} ms={ms}");
        let (path, live, peak, ms) = measure(|| j.save().unwrap());
        println!(
            "n={n} save live={live} peak={peak} ms={ms} json={}",
            std::fs::metadata(path).unwrap().len()
        );
    }
}

#[test]
#[ignore = "full-library target discovery and heap measurement; run alone"]
fn synthetic_full_library_target_growth() {
    for (library_count,incoming) in [(100,20),(1000,20),(5000,20),(5000,100)] {
        let temp=tempfile::tempdir().unwrap();let library=temp.path().join("library");let source=temp.path().join("source");
        std::fs::create_dir(&library).unwrap();std::fs::create_dir(&source).unwrap();
        for i in 0..library_count {
            let code=format!("ABC-{i:05}");let directory=library.join(&code);std::fs::create_dir(&directory).unwrap();
            let metadata=Metadata{code:code.clone(),fields:PartialMetadata{title:Some(format!("Existing {code}")),..Default::default()},..Default::default()};
            std::fs::write(directory.join("kiroku.json"),serde_json::to_vec(&metadata).unwrap()).unwrap();std::fs::write(directory.join(format!("{code}-CD1.mp4")),b"old").unwrap();
            if i<incoming{std::fs::write(source.join(format!("{code}-CD2.mp4")),b"new").unwrap();}
        }
        let scan=pipeline::scan(&[source],None,&VideoFilter::none(),&CancellationToken::new(),&mut|_|{});
        let pairs=scan.items.iter().map(|item|(item,item.code.as_ref().unwrap())).collect::<Vec<_>>();
        let metas=pairs.iter().map(|(_,code)|(code.id.clone(),Metadata{code:code.id.clone(),..Default::default()})).collect();
        // Reference repeats a fresh full discovery for each input; this is a
        // conservative baseline (the old append branch also walked a second time).
        let (reference,_,old_peak,old_ms)=measure(||pairs.iter().flat_map(|pair|pipeline::build_plans(std::slice::from_ref(pair),&metas,&library,"{code}",PlanOpts::default())).collect::<Vec<_>>());
        let (plans,live,peak,ms)=measure(||pipeline::build_plans(&pairs,&metas,&library,"{code}",PlanOpts::default()));
        assert!(plans.iter().all(|plan|plan.selected&&plan.append_preview.is_some()));
        for (plan,old)in plans.iter().zip(&reference){assert_eq!(plan.ops,old.ops);assert_eq!(plan.to,old.to);assert_eq!(plan.selected,old.selected);}
        println!("library={library_count} incoming={incoming}; repeated discovery={old_ms}ms peak={old_peak}B; batch={ms}ms live={live}B peak={peak}B; index is not opened; measured heap includes complete plans and excludes OS cache/native allocations");
    }
}

/// Real allocated media (not sparse); run alone, without other allocator activity.
#[test]
#[ignore = "large allocated media timing and heap measurement"]
fn large_video_import_growth() {
    use kiroku_core::{organize, placement::{self,OrganizeMode}};
    use std::io::Write;
    let sizes=std::env::var("KIROKU_VIDEO_MIB").ok().map(|s|vec![s.parse::<u64>().expect("positive MiB size")]).unwrap_or_else(||vec![64,256,1024]);
    for mib in sizes {
        for mode in [OrganizeMode::Move,OrganizeMode::Copy,OrganizeMode::Hardlink,OrganizeMode::Symlink] {
            let t=tempfile::tempdir().unwrap();
            let from=t.path().join("ABC-123.mp4");
            let mut file=std::fs::File::create(&from).unwrap();
            let chunk=[37u8;64*1024];
            for _ in 0..mib*16 {file.write_all(&chunk).unwrap();}
            file.sync_all().unwrap();drop(file);
            let before=placement::source_proof(&from).unwrap();
            let root=t.path().join("library");std::fs::create_dir(&root).unwrap();
            let code=kiroku_core::code::parse_code("ABC-123.mp4").unwrap();
            let meta=Metadata{code:code.id.clone(),..Default::default()};
            let ((mut journal,target),live,peak,ms)=measure(|| {
                let mut plan=organize::plan_with(&from,&code,&meta,&root,"{code}",true,PlanOpts{mode,..Default::default()});
                let target=plan.ops[0].target().to_path_buf();
                let mut journal=Journal::new(&root);
                placement::bind_plan(&mut plan,&journal).unwrap();
                let result=placement::execute(&mut journal,&plan,&mut |_|unreachable!(),&||false);
                assert!(result.error.is_none(),"{mode:?}: {:?}",result.error);
                (journal,target)
            });
            assert_eq!(std::fs::metadata(&target).unwrap().len(),mib*1024*1024);
            println!("large-video mode={mode:?} size={mib}MiB elapsed={ms}ms peak={peak}B live={live}B");
            assert!(peak<2*1024*1024,"media must not enter heap: {peak}");
            assert!(journal.rollback().is_empty());
            assert!(!target.exists());
            assert_eq!(placement::source_proof(&from).unwrap().rsplit(':').next(),before.rsplit(':').next());
        }
    }
}

#[test]
#[ignore="actual many-item journal and checkpoint heap growth; run alone"]
fn synthetic_incremental_checkpoint_heap_growth(){
    use kiroku_core::organize::Plan;
    for n in [100,1000,5000] {
        let root=tempfile::tempdir().unwrap();let mut j=Journal::new(root.path());
        let (_,live,peak,ms)=measure(||{
            for i in 0..n {
                let ex=j.execute(&Plan{ops:vec![Op::WriteFile{path:root.path().join(format!("{i}.nfo")),content:"fixture".into()}]},&mut |_|unreachable!(),&||false);
                assert!(ex.error.is_none(),"{:?}",ex.error);
            }
        });
        let appended=std::fs::metadata(j.path()).unwrap().len();
        let (_,extra,checkpoint_peak,checkpoint_ms)=measure(||j.execute(&Plan{ops:vec![]},&mut |_|unreachable!(),&||false));
        assert!(checkpoint_peak<128*1024,"checkpoint allocated {checkpoint_peak}");
        let (_,_,compact_peak,compact_ms)=measure(||j.save().unwrap());assert!(compact_peak<128*1024);
        let compact=std::fs::metadata(j.path()).unwrap().len();
        assert_eq!(Journal::load(&j.path()).unwrap(),j);
        println!("checkpoint-heap n={n} live={live}B peak={peak}B elapsed={ms}ms append={appended}B compact={compact}B checkpoint-live={extra}B checkpoint-peak={checkpoint_peak}B checkpoint-ms={checkpoint_ms} compact-peak={compact_peak}B compact-ms={compact_ms}");
    }
}
