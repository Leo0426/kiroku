//! 真实站点冒烟检查:需要联网,默认忽略;只读请求,不写任何文件。
//! 要查询的文件名由运行者通过 `KIROKU_LIVE_FILES`(逗号分隔,如 `ABC-123.mp4,XYZ-456.mp4`)提供,仓库里不写真实编号。
//! `KIROKU_LIVE_FILES=… cargo test -p kiroku-core --test live_sources -- --ignored --nocapture --test-threads=1`
use kiroku_core::{code::parse_code, net::{Http, Solver}, source::Source, sources::{avbase::AvBase, jav321::Jav321, javlibrary::JavLibrary}};
use std::time::Duration;

fn live_files() -> Vec<String> {
    let files: Vec<String> = std::env::var("KIROKU_LIVE_FILES").expect("set KIROKU_LIVE_FILES")
        .split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    assert!(!files.is_empty(), "KIROKU_LIVE_FILES is empty");
    files
}

fn solver_http() -> Http {
    let endpoint = std::env::var("KIROKU_SOLVER").expect("set KIROKU_SOLVER");
    Http::new(Duration::from_secs(5), None).unwrap()
        .with_solver(Some(Solver::new(&endpoint, Duration::from_secs(90), None).unwrap()))
}

#[tokio::test]
#[ignore = "live network: jav321 default-route health"]
async fn jav321_still_parses_default_route_fields() {
    let http = Http::new(Duration::from_secs(5), None).unwrap();
    let source = Jav321::default();
    for file in live_files() {
        let code = parse_code(&file).unwrap();
        if !source.supports(&code) {
            println!("{file}: not routed to jav321, skipped");
            continue;
        }
        let m = source.fetch(&code, &http).await.unwrap_or_else(|e| panic!("{file}: {e:?}"));
        println!("{file}: title={:?} plot_chars={} runtime={:?} release={:?} studio={:?}",
            m.title, m.plot.as_deref().map_or(0, |p| p.chars().count()), m.runtime_min, m.release_date, m.studio);
        assert!(m.title.as_deref().is_some_and(|t| !t.is_empty()), "{file}: title missing");
        assert!(m.original_title.is_some(), "{file}: original_title missing");
        assert!(m.runtime_min.is_some(), "{file}: runtime missing");
    }
}

/// 需要用户本机运行 FlareSolverr:`KIROKU_SOLVER=http://127.0.0.1:8191/v1`(ADR-0007)。
#[tokio::test]
#[ignore = "live network + user FlareSolverr: JavLibrary through the solver"]
async fn javlibrary_fetches_through_the_user_solver() {
    let http = solver_http();
    let source = JavLibrary { base: "https://www.javlibrary.com".into(), cookie: String::new() };
    for file in live_files() {
        let code = parse_code(&file).unwrap();
        if !source.supports(&code) {
            continue;
        }
        let m = source.fetch(&code, &http).await.unwrap_or_else(|e| panic!("{file}: {e:?}"));
        println!("{file}: title={:?} release={:?} actors={:?} score={:?}", m.title, m.release_date, m.actors, m.score);
        assert!(m.release_date.is_some(), "{file}: release missing");
        assert!(m.title.as_deref().is_some_and(|t| !t.starts_with(&code.id)), "{file}: title keeps the code prefix");
    }
}

#[tokio::test]
#[ignore = "live network + user FlareSolverr: AVBase through the solver"]
async fn avbase_fetches_through_the_user_solver() {
    let http = solver_http();
    for file in live_files() {
        let code = parse_code(&file).unwrap();
        if !AvBase::default().supports(&code) {
            continue;
        }
        let m = AvBase::default().fetch(&code, &http).await.unwrap_or_else(|e| panic!("{file}: {e:?}"));
        println!("{file}: title={:?} release={:?} runtime={:?} tags={:?} url={:?}", m.title, m.release_date, m.runtime_min, m.tags, m.source_url);
        assert!(m.release_date.is_some(), "{file}: release missing");
    }
}
