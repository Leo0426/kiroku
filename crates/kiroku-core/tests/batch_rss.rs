//! 大批次端到端内存测量(issue 10)。每次只跑一个规模,让进程峰值 RSS 不受前一轮影响:
//! `KIROKU_BATCH=1000 cargo test -p kiroku-core --test batch_rss -- --ignored --nocapture`
//! 合成小视频(1 KiB)和本地假来源;不触碰真实媒体,不联网,没有图片下载。
//! 峰值 RSS 用 getrusage 读取,只在 Unix 上编译。
#![cfg(unix)]
use kiroku_core::{
    code::Code, metadata::{Actor, Gender, PartialMetadata}, net::Http, organize::PlanOpts, pipeline::{self, Scraper},
    settings::{Settings, SourceCfg}, source::{Source, SourceError}, video_filter::VideoFilter,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::HashMap,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct Heap;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() { let n = LIVE.fetch_add(l.size(), Ordering::Relaxed) + l.size(); PEAK.fetch_max(n, Ordering::Relaxed); }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) }; LIVE.fetch_sub(l.size(), Ordering::Relaxed); }
}
#[global_allocator]
static ALLOCATOR: Heap = Heap;

/// 进程峰值 RSS(字节)。macOS 的 ru_maxrss 单位是字节,Linux 是 KiB。
fn max_rss() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let raw = usage.ru_maxrss as u64;
    if cfg!(target_os = "macos") { raw } else { raw * 1024 }
}
const MIB: f64 = 1024.0 * 1024.0;

struct Fake;
#[async_trait::async_trait]
impl Source for Fake {
    fn id(&self) -> &'static str { "fake" }
    fn supports(&self, _: &Code) -> bool { true }
    async fn fetch(&self, code: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
        // 字段规模接近真实条目:标题、约 600 字简介、几位演员和标签,无图片 URL
        Ok(PartialMetadata {
            title: Some(format!("{} 合成标题 {}", code.id, "长".repeat(40))),
            plot: Some("简介".repeat(300)),
            release_date: Some("2024-01-02".into()),
            runtime_min: Some(120),
            studio: Some("Studio".into()),
            actors: (0..3).map(|i| Actor::new(format!("演员{i}"), Gender::Female)).collect(),
            tags: (0..8).map(|i| format!("标签{i}")).collect(),
            ..Default::default()
        })
    }
}

#[tokio::test]
#[ignore = "end-to-end batch RSS; run alone with KIROKU_BATCH"]
async fn batch_rss() {
    let n: usize = std::env::var("KIROKU_BATCH").ok().and_then(|v| v.parse().ok()).unwrap_or(1000);
    let t = tempfile::tempdir().unwrap();
    let source = t.path().join("source");
    let library = t.path().join("library");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(&library).unwrap();
    for i in 0..n { std::fs::write(source.join(format!("ABC-{i:05}.mp4")), vec![0u8; 1024]).unwrap(); }
    let report = |stage: &str, started: Instant| println!(
        "n={n} {stage:<8} {:>7} ms  live={:>7.1} MiB  heap_peak={:>7.1} MiB  rss_peak={:>7.1} MiB",
        started.elapsed().as_millis(), LIVE.load(Ordering::Relaxed) as f64 / MIB, PEAK.load(Ordering::Relaxed) as f64 / MIB, max_rss() as f64 / MIB);
    let baseline = Instant::now();
    report("baseline", baseline);
    let cancel = CancellationToken::new();

    let started = Instant::now();
    let scanned = pipeline::scan(&[source.clone()], Some(&library), &VideoFilter::default(), &cancel, &mut |_| {});
    assert_eq!(scanned.items.len(), n);
    report("scan", started);

    let mut settings = Settings { min_interval_secs: 0, ..Settings::default() };
    settings.routes.insert(kiroku_core::code::ContentType::Censored, vec!["fake".into()]);
    settings.sources.push(SourceCfg { id: "fake".into(), enabled: true, base: "https://fake.invalid".into(), cookie: String::new(), api_base: String::new(), proxy: Default::default(), solver: false });
    let scraper = Scraper::with_sources(settings, vec![Box::new(Fake)]).unwrap();
    let codes: Vec<Code> = scanned.items.iter().filter_map(|i| i.code.clone()).collect();
    let started = Instant::now();
    let metas: HashMap<_, _> = pipeline::scrape_all(&scraper, &codes, &cancel, &|_| {}).await.into_iter().map(|(k, v)| (k, v.unwrap())).collect();
    report("scrape", started);

    let started = Instant::now();
    let pairs: Vec<_> = scanned.items.iter().filter_map(|i| i.code.as_ref().map(|c| (i, c))).collect();
    let plans = pipeline::build_plans(&pairs, &metas, &library, "{code}", PlanOpts::default());
    assert_eq!(plans.len(), n);
    report("plan", started);

    let started = Instant::now();
    let http = Http::new(Duration::ZERO, None).unwrap();
    let outcome = pipeline::apply(plans, &http, &library, None, &cancel, &|_| {}).await;
    assert!(outcome.failed.is_empty(), "{:?}", outcome.failed.first());
    assert_eq!(outcome.applied.len(), n);
    report("apply", started);
    // 桌面会话在整理后仍保留扫描结果和元数据;这里一起保留,反映常驻量
    drop(outcome);
    report("retained", baseline);
    drop((scanned, metas, codes));
    report("released", baseline);
}
