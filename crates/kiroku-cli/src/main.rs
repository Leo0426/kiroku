use kiroku_core::{
    code::Code,
    journal::Journal,
    library::{Library, Query},
    organize::{self, Op},
    pipeline::{self, Scraper},
    settings::Settings,
    Metadata,
};
use std::{collections::HashMap, env, path::PathBuf, process::ExitCode, time::Duration};
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

fn usage() -> ExitCode {
    eprintln!(
        "kiroku <command>\n\n  nfo-export <影片目录> [更多目录] --root <库目录> --language current|zh-Hans|original [--apply]   原位预览并导出 NFO，可用日志撤销\n  catalog names <preview|apply> <库目录> <影片目录...>  应用名称规则（明确选择影片）\n  maintenance <库目录> [--history-days 90] [--log-days 90] [--apply]   预览安全清理\n  health <库目录>   只读媒体库检查（JSON）\n  sources-check [source-id]   检测当前启用来源\n  append <code> <path> --root <库目录> [--target <影片目录>] [--video <字幕目标视频>] [--mode move|copy|hardlink|symlink] [--apply]   安全追加，默认预览\n  parse <path>           扫描目录/文件，输出番号、内容类型、分集、字幕、马赛克、清晰度 (JSON lines)\n  poc <javdb|dmm|url> [n] [reqwest|wreq]   反爬 PoC(wreq 需 --features wreq)，代理用环境变量 KIROKU_PROXY\n  scrape <dir> --out <库目录> [--tpl <模板>] [--create-library] [--mode move|copy|hardlink|symlink] [--keep-versions] [--apply]   刮削并整理;默认 dry-run,--apply 才落盘;--keep-versions 把同分片的不同内容作为新版本并存\n  scrape-url <文件名或番号> <详情页URL> [--config <设置>]   用指定站点详情页刮削一部影片，只输出预览（JSON）
  rollback <journal.json>  按操作日志回滚一次整理
  index <库目录> [关键词]   从磁盘重建媒体库索引并列出前 10 部(调试用)
  template <tpl>         用示例元数据预览命名模板"
    );
    ExitCode::from(2)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("catalog") if args.get(1).map(String::as_str)==Some("names") && args.len()>=4 => catalog_names(&args[2..]),

        Some("nfo-export") if args.len()>=2 => export_nfo(&args[1..]),
        Some("maintenance") if args.len()>=2 => maintain(&args[1..]),
        Some("health") if args.len()>=2 => match kiroku_core::health::scan(std::path::Path::new(&args[1]),&cli_cancel(),&|_|{}){Ok(r)=>{println!("{}",serde_json::to_string_pretty(&r).unwrap());if r.issues.iter().any(|i|i.severity=="error"){ExitCode::FAILURE}else{ExitCode::SUCCESS}},Err(e)=>{eprintln!("{e}");ExitCode::FAILURE}},
        Some("sources-check") => {let settings=Settings::load(&Settings::default_path()).unwrap_or_default();let ids=args.get(1).map(|id|vec![id.clone()]);let token=cli_cancel();let checks=kiroku_core::connectivity::check(&settings,ids.as_deref(),&token).await;println!("{}",serde_json::to_string_pretty(&checks).unwrap());if checks.iter().any(|c|c.outcome.status=="failed"){ExitCode::FAILURE}else{ExitCode::SUCCESS}},
        Some("append") if args.len() >= 3 => append_files(&args[1..]),
        Some("links") if args.len() >= 2 => revalidate_links(&args[1..]),
        Some("parse") if args.len() >= 2 => {
            for e in WalkDir::new(&args[1]).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file()) {
                let p = e.path();
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                if !pipeline::VIDEO_EXT.contains(&ext.as_str()) { continue; }
                let info = kiroku_core::code::parse_file_info(&p.to_string_lossy());
                println!("{}", serde_json::json!({ "path": p, "info": info }));
            }
            ExitCode::SUCCESS
        }
        Some("poc") if args.len() >= 2 => poc(&args[1], args.get(2).and_then(|n| n.parse().ok()).unwrap_or(50), args.get(3).map(String::as_str).unwrap_or("reqwest")).await,
        Some("scrape") if args.len() >= 2 => scrape(&args[1..]).await,
        Some("scrape-url") if args.len() >= 3 => scrape_url(&args[1], &args[2], flag(&args, "--config")).await,
        Some("rollback") if args.len() >= 2 => rollback(&args[1]),
        Some("index") if args.len() >= 2 => index(&args[1], args.get(2).map(String::as_str)),
        Some("template") if args.len() >= 2 => {
            let m = kiroku_core::Metadata {
                code: "ABC-123".into(),
                fields: kiroku_core::PartialMetadata {
                    title: Some("Sample Title".into()),
                    actors: vec![kiroku_core::metadata::Actor::new("Actor", kiroku_core::metadata::Gender::Female)],
                    release_date: Some("2024-01-02".into()),
                    ..Default::default()
                },
                ..Default::default()
            };
            println!("{}", organize::render_template(&args[1], &m));
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

/// 预置目标：(url, 命中标记, 额外 Cookie)。成功 = 状态 200 且正文含标记。
fn target(name: &str) -> Option<(&'static str, &'static str, &'static str)> {
    match name {
        "javdb" => Some(("https://javdb.com/search?q=SSIS-001&f=all", "class=\"video-title\"", "")),
        "dmm" => Some(("https://video.dmm.co.jp/av/content/?id=ssis00001", "SSIS", "age_check_done=1; cklg=ja")),
        _ => None,
    }
}

async fn fetch_reqwest(c: &reqwest::Client, url: &str, cookie: &str) -> Result<(u16, String), String> {
    let mut r = c.get(url);
    if !cookie.is_empty() { r = r.header("Cookie", cookie); }
    let resp = r.send().await.map_err(|e| e.to_string())?;
    let st = resp.status().as_u16();
    Ok((st, resp.text().await.unwrap_or_default()))
}

#[cfg(feature = "wreq")]
async fn fetch_wreq(c: &wreq::Client, url: &str, cookie: &str) -> Result<(u16, String), String> {
    let mut r = c.get(url);
    if !cookie.is_empty() { r = r.header("Cookie", cookie); }
    let resp = r.send().await.map_err(|e| e.to_string())?;
    let st = resp.status().as_u16();
    Ok((st, resp.text().await.unwrap_or_default()))
}

/// poc <javdb|dmm|url> [n] [reqwest|wreq]；代理读环境变量 KIROKU_PROXY。
async fn poc(spec: &str, n: u32, engine: &str) -> ExitCode {
    let (url, marker, cookie) = target(spec).unwrap_or((spec, "", ""));
    let proxy = env::var("KIROKU_PROXY").ok();
    let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0 Safari/537.36";

    let rq = {
        let mut b = reqwest::Client::builder().user_agent(ua).timeout(Duration::from_secs(20));
        if let Some(p) = &proxy { b = b.proxy(reqwest::Proxy::all(p).unwrap()); }
        b.build().unwrap()
    };
    #[cfg(feature = "wreq")]
    let wq = {
        let mut b = wreq::Client::builder().emulation(wreq_util::Emulation::Chrome137).timeout(Duration::from_secs(20));
        if let Some(p) = &proxy { b = b.proxy(wreq::Proxy::all(p).unwrap()); }
        b.build().unwrap()
    };

    let (mut ok, mut fail) = (0u32, 0u32);
    for i in 1..=n {
        #[cfg(feature = "wreq")]
        let res = if engine == "reqwest" { fetch_reqwest(&rq, url, cookie).await } else { fetch_wreq(&wq, url, cookie).await };
        #[cfg(not(feature = "wreq"))]
        let res = if engine == "reqwest" {
            fetch_reqwest(&rq, url, cookie).await
        } else {
            eprintln!("wreq 引擎需要 `cargo run -p kiroku-cli --features wreq`");
            return ExitCode::from(2);
        };
        match res {
            Ok((200, body)) if marker.is_empty() || body.contains(marker) => ok += 1,
            Ok((st, body)) => {
                fail += 1;
                let hint = if body.contains("not-available-in-your-region") || body.contains("お住まいの地域") { "geo-blocked" }
                    else if body.contains("Just a moment") || body.contains("cf-chl") { "cloudflare-challenge" } else { "no-marker" };
                eprintln!("#{i} status {st} {hint}");
            }
            Err(e) => { fail += 1; eprintln!("#{i} error {e}"); }
        }
        let gap = env::var("KIROKU_POC_INTERVAL_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500u64);
        tokio::time::sleep(Duration::from_millis(gap)).await; // 对目标站限速
    }
    let rate = ok as f64 / n as f64 * 100.0;
    println!("[{engine}] {spec}: ok {ok}/{n} ({rate:.1}%), failed {fail}{}", if proxy.is_some() { " (via proxy)" } else { "" });
    if rate >= 95.0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// 用用户给出的某站详情页刮削一部影片,只预览;番号对不上时在 notes 里提示。
async fn scrape_url(name: &str, url: &str, config: Option<&str>) -> ExitCode {
    let cfg_path = config.map(PathBuf::from).unwrap_or_else(Settings::default_path);
    let mut settings = match Settings::load(&cfg_path) { Ok(s) => s, Err(e) => { eprintln!("设置无效 ({}): {e}", cfg_path.display()); return ExitCode::FAILURE } };
    if let Ok(p) = env::var("KIROKU_PROXY") { settings.proxy = Some(p); }
    let Some(code) = kiroku_core::code::parse_code(name).or_else(|| kiroku_core::code::parse_code(&format!("{name}.mp4"))) else { eprintln!("无法从「{name}」解析番号"); return ExitCode::from(2) };
    let scraper = match Scraper::new(settings) { Ok(s) => s.with_translation_cache(kiroku_core::translate::cache_path(&cfg_path)), Err(e) => { eprintln!("{e}"); return ExitCode::FAILURE } };
    match scraper.scrape_url(&code, url, &cli_cancel()).await {
        Ok(m) => {
            println!("{}", serde_json::to_string_pretty(&serde_json::json!({"code": m.code, "fields": m.fields, "provenance": m.provenance, "notes": m.scrape_notes})).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => { eprintln!("{}", e.message()); ExitCode::FAILURE }
    }
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

async fn scrape(args: &[String]) -> ExitCode {
    let src_dir = PathBuf::from(&args[0]);
    let Some(out) = flag(args, "--out").map(PathBuf::from) else {
        eprintln!("缺少 --out <库目录>");
        return ExitCode::from(2);
    };
    let cfg_path = flag(args, "--config").map(PathBuf::from).unwrap_or_else(Settings::default_path);
    let mut settings = match Settings::load(&cfg_path).and_then(|s| s.validate().map(|_| s).map_err(std::io::Error::other)) {
        Ok(s) => s,
        Err(e) => { eprintln!("设置无效 ({}): {e}", cfg_path.display()); return ExitCode::FAILURE; }
    };
    // 与客户端共用日志设置;CLI 没有 App 界面,app 一项不起作用
    kiroku_core::logging::init(kiroku_core::logging::log_dir(&cfg_path), &settings.log);
    if let Some(mode)=flag(args,"--mode") {settings.organize_mode=match mode {"move"=>kiroku_core::placement::OrganizeMode::Move,"copy"=>kiroku_core::placement::OrganizeMode::Copy,"hardlink"=>kiroku_core::placement::OrganizeMode::Hardlink,"symlink"=>kiroku_core::placement::OrganizeMode::Symlink,_=>{eprintln!("--mode 必须为 move/copy/hardlink/symlink");return ExitCode::from(2);}};}
    let tpl = flag(args, "--tpl").unwrap_or(&settings.dir_template).to_string();
    let apply = args.iter().any(|a| a == "--apply");
    // 环境变量 KIROKU_PROXY 可临时覆盖设置里的代理
    if let Ok(p) = env::var("KIROKU_PROXY") { settings.proxy = Some(p); }
    let scraper = match Scraper::new(settings) {
        Ok(s) => s.with_translation_cache(kiroku_core::translate::cache_path(&cfg_path)),
        Err(e) => { eprintln!("{e}"); return ExitCode::FAILURE; }
    };

    // Ctrl-C 取消:已完成的整理仍会写日志
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move { let _ = tokio::signal::ctrl_c().await; c.cancel(); });

    // 1. 扫描
    let filter = scraper.settings.video_filter();
    let scan = pipeline::scan(std::slice::from_ref(&src_dir), Some(&out), &filter, &cancel, &mut |_| {});
    for error in &scan.summary.errors { eprintln!("扫描失败 {}: {}", error.path.display(), error.message); }
    if !scan.summary.complete {
        eprintln!("扫描未完成，未继续刮削或整理；请修复目录错误后重试");
        return ExitCode::FAILURE;
    }
    for s in &scan.skipped {
        eprintln!("跳过(非正片 {:?}): {}", s.reason, s.path.display());
    }
    for i in scan.items.iter().filter(|i| i.code.is_none()) {
        eprintln!("跳过(无番号): {}", i.path.display());
    }
    println!("{} 个文件,{} 个番号", scan.items.len(), scan.codes);

    // 2. 刮削(按内容类型路由、按番号去重)
    let codes: Vec<Code> = scan.items.iter().filter_map(|i| i.code.clone()).collect();
    let previous = match kiroku_core::append::existing_metadata_cancellable(&out, &codes.iter().map(|c|c.id.clone()).collect::<Vec<_>>(),&cancel) { Ok(metadata)=>metadata,Err(error)=>{eprintln!("读取已有影片失败: {error}");return ExitCode::FAILURE;} };
    let results = pipeline::scrape_missing(&scraper, &codes, &previous, &cancel, &|ev| {
        eprintln!("[{}/{}] {} {:?}", ev.done, ev.total, ev.code, ev.status);
    })
    .await;
    let mut metas: HashMap<String, Metadata> = previous;
    for (code, r) in results {
        match r {
            Ok(m) => { metas.insert(code, m); }
            Err(e) => eprintln!("{code}: {}", e.message()),
        }
    }

    // 3. 计划 → dry-run 打印 | 执行
    let pairs: Vec<_> = scan.items.iter().filter_map(|i| i.code.as_ref().map(|c| (i, c))).collect();
    let mut plan_opts=scraper.settings.plan_opts();plan_opts.create_library=args.iter().any(|a|a=="--create-library");
    let plans = match pipeline::build_plans_cancellable(&pairs, &metas, &out, &tpl, plan_opts, &cancel) {
        Ok(plans) => plans,
        Err(error) => { eprintln!("生成整理计划失败: {error}"); return ExitCode::FAILURE; }
    };
    for p in &plans {
        for notice in &p.notices{println!("  {notice}");}
        if let Some(c) = p.conflict { println!("冲突 {c:?}(默认跳过): {}", p.from.display()); }
        for c in &p.conflicts { println!("  {:?}: {}", c.kind, c.path.display()); }
        for op in &p.ops {
            match op {
                Op::MoveFile { from, to } => println!("MOVE  {} -> {}", from.display(), to.display()),
                Op::WriteFile { path, .. } => println!("WRITE {}", path.display()),
                op if op.retained()=>println!("{}",op.describe()),
                Op::Download { url, path, .. } => println!("GET   {url} -> {}", path.display()),
                _=>unreachable!(),
            }
        }
    }
    if !apply {
        println!("\n(dry-run;加 --apply 执行)");
        return ExitCode::SUCCESS;
    }
    let had_conflicts = plans.iter().any(|p| p.conflict.is_some());
    let root_guards:Vec<_>=plans.iter().filter_map(|p|p.root_guard.clone()).collect();
    let plans_were_missing=plans.is_empty();
    // 新版本并存(ADR-0008)默认不执行,需显式 --keep-versions
    let keep_versions = args.iter().any(|a| a == "--keep-versions");
    let chosen: Vec<_> = plans.into_iter().filter(|p| p.selected || (keep_versions && p.new_version && p.conflict.is_none())).collect();
    if chosen.is_empty() { if !plans_were_missing && !had_conflicts {println!("相同文件已存在，未移动源文件");return ExitCode::SUCCESS} eprintln!("没有可执行项目（未找到元数据或目标存在冲突）"); return ExitCode::FAILURE; }
    let roots = [src_dir.clone()];
    let cleanup = scraper.settings.cleanup_source.then_some(pipeline::Cleanup { roots: &roots, trash: pipeline::trash_to_system, filter: &filter });
    let outcome = pipeline::apply_with_progress(chosen, &scraper.http, &out, cleanup, &cancel, &|ev| {
        if let Some(e) = &ev.error { eprintln!("[{}/{}] {} 失败: {e}", ev.done, ev.total, ev.code); }
        for w in &ev.warnings { eprintln!("  警告 {}: {w}", ev.code); }
    }, &|progress| {
        if progress.phase == pipeline::ApplyPhase::Images {
            eprintln!("图片下载 {}/{}{}{}", progress.done, progress.total,
                progress.code.as_ref().map(|c| format!(" {c}")).unwrap_or_default(),
                if progress.failed > 0 { format!("（{} 张失败）", progress.failed) } else { String::new() });
        }
    })
    .await;
    let index_errors=if !out.is_dir()||root_guards.iter().any(|g|g.verify().is_err()){vec!["媒体库不可用，保留缓存".into()]}else{match Library::open(&out){Ok(lib)=>outcome.journal.items.iter().filter_map(|item|{let dir=out.canonicalize().ok().and_then(|r|item.dir.strip_prefix(r).ok().map(|p|out.join(p))).unwrap_or_else(||item.dir.clone());lib.reindex_dir(&dir).err().map(|e|e.to_string())}).collect::<Vec<_>>(),Err(e)=>vec![e.to_string()]}};
    for error in &index_errors{eprintln!("索引更新失败：{error}");}
    println!(
        "完成 {} 项,失败 {},{}回滚日志: {}",
        outcome.applied.len(), outcome.failed.len(), if outcome.cancelled { "已取消," } else { "" },
        outcome.journal.path().display()
    );
    for d in &outcome.cleaned { println!("已移到废纸篓: {}", d.display()); }
    if outcome.failed.is_empty() && !had_conflicts && index_errors.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn rollback(path: &str) -> ExitCode {
    let mut j = match Journal::load(std::path::Path::new(path)) {
        Ok(j) => j,
        Err(e) => { eprintln!("无法读取日志 {path}: {e}"); return ExitCode::FAILURE; }
    };
    let mut dirs: std::collections::HashSet<_> = j.items.iter().map(|item| item.dir.clone()).collect();
    for op in j.ops.iter().chain(j.pending.iter().map(|pending| &pending.op)) {
        if let Some(parent) = op.target().parent() { dirs.insert(parent.to_path_buf()); }
    }
    let failures = j.rollback();
    let index_errors = match Library::open(&j.root) {
        Ok(lib) => dirs.iter().filter_map(|dir| lib.reindex_dir(dir).err().map(|e| format!("{}: {e}",dir.display()))).collect::<Vec<_>>(),
        Err(e) => vec![e.to_string()],
    };
    for error in &index_errors { eprintln!("文件撤销结果已记录,索引更新失败: {error}"); }
    for f in &failures { eprintln!("回滚失败: {f}"); }
    println!("已回滚 {} 项操作,失败 {}", j.ops.len(), failures.len());
    if failures.is_empty() && index_errors.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn index(root: &str, text: Option<&str>) -> ExitCode {
    let lib = match Library::open(std::path::Path::new(root)) {
        Ok(l) => l,
        Err(e) => { eprintln!("{e}"); return ExitCode::FAILURE; }
    };
    let n = match lib.rebuild(&CancellationToken::new(), &mut |_, _| {}) {
        Ok(n) => n,
        Err(e) => { eprintln!("重建失败: {e}"); return ExitCode::FAILURE; }
    };
    let page = lib.list(&Query { text: text.map(String::from), limit: 10, ..Default::default() }).expect("list");
    println!("索引 {n} 部,匹配 {} 部", page.total);
    for c in page.items {
        println!("  {} | {} | {:?} | {} | poster:{}", c.code, c.title.unwrap_or_default(), c.year, c.actors.join("、"), c.poster.is_some());
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cli_rollback_reindexes_the_affected_movie() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("library");
        let source = d.path().join("ABC-123.mp4");
        std::fs::write(&source, b"video").unwrap();
        let code = kiroku_core::parse_code("ABC-123.mp4").unwrap();
        let meta = kiroku_core::Metadata { code: code.id.clone(), ..Default::default() };
        let plan = kiroku_core::organize::plan(&source,&code,&meta,&root,"{code}");
        let mut j = Journal::new(&root);
        assert!(j.execute(&plan,&mut |_| unreachable!(),&|| false).error.is_none());
        let lib = Library::open(&root).unwrap();
        lib.reindex_dir(&root.join("ABC-123")).unwrap();
        assert_eq!(lib.list(&Default::default()).unwrap().total,1);
        assert_eq!(rollback(j.path().to_str().unwrap()),ExitCode::SUCCESS);
        assert_eq!(lib.list(&Default::default()).unwrap().total,0);
        assert_eq!(std::fs::read(source).unwrap(),b"video");
    }
}

fn append_files(args:&[String])->ExitCode {let run=||->Result<(),Box<dyn std::error::Error>>{let arg=|name:&str|args.iter().position(|s|s==name).and_then(|i|args.get(i+1));let root=arg("--root").ok_or("需要 --root <库目录>")?;let target=arg("--target").map(std::path::Path::new);let video=arg("--video").map(std::path::Path::new);let token=cli_cancel();let mut p=kiroku_core::append::preview_with_video_cancellable(std::path::Path::new(root),&args[0],&[PathBuf::from(&args[1])],target,video,&token)?;let settings=Settings::load(&Settings::default_path())?;let mode=arg("--mode").map(|m|serde_json::from_value::<kiroku_core::placement::OrganizeMode>(serde_json::Value::String(m.clone()))).transpose()?.unwrap_or(settings.organize_mode);p.set_mode(mode)?;if args.iter().any(|s|s=="--apply"){let r=kiroku_core::append::apply(&p,&token)?;println!("{}",serde_json::to_string_pretty(&r)?);if !r.failed.is_empty(){return Err(r.failed.join("; ").into())}}else{println!("{}",serde_json::to_string_pretty(&p)?)}Ok(())};match run(){Ok(())=>ExitCode::SUCCESS,Err(e)=>{eprintln!("{e}");ExitCode::FAILURE}}}

fn maintain(args:&[String])->ExitCode{let run=||->Result<(),Box<dyn std::error::Error>>{let get=|name:&str,default:u32|->Result<u32,Box<dyn std::error::Error>>{if let Some(i)=args.iter().position(|s|s==name){Ok(args.get(i+1).ok_or("保留期缺少数值")?.parse()?)}else{Ok(default)}};let settings=Settings::load(&Settings::default_path()).unwrap_or_default();let token=cli_cancel();let policy=kiroku_core::maintenance::RetentionPolicy{history_days:get("--history-days",settings.retention.history_days)?,log_days:get("--log-days",settings.retention.log_days)?};let p=kiroku_core::maintenance::preview_cancellable(std::path::Path::new(&args[0]),&kiroku_core::logging::log_dir(&Settings::default_path()),&policy,&token)?;if args.iter().any(|s|s=="--apply"){let r=kiroku_core::maintenance::apply(&p,&p.candidates.iter().map(|c|c.id.clone()).collect::<Vec<_>>(),&pipeline::trash_to_system,&token)?;println!("{}",serde_json::to_string_pretty(&r)?);if !r.failed.is_empty(){return Err(r.failed.join("; ").into())}}else{println!("{}",serde_json::to_string_pretty(&p)?)}Ok(())};match run(){Ok(())=>ExitCode::SUCCESS,Err(e)=>{eprintln!("{e}");ExitCode::FAILURE}}}

fn cli_cancel()->CancellationToken{let token=CancellationToken::new();let signal=token.clone();tokio::spawn(async move{if tokio::signal::ctrl_c().await.is_ok(){signal.cancel()}});token}

fn catalog_names(args: &[String]) -> ExitCode {
    let apply = args[0] == "apply";
    if !apply && args[0] != "preview" { return usage(); }
    let root=PathBuf::from(&args[1]);
    let dirs=args[2..].iter().map(PathBuf::from).collect::<Vec<_>>();
    if dirs.is_empty() { eprintln!("请指定要处理的影片目录"); return ExitCode::from(2); }
    let settings=match Settings::load(&Settings::default_path()) {Ok(s)=>s,Err(e)=>{eprintln!("无法读取设置: {e}");return ExitCode::FAILURE}};
    let cancel=cli_cancel();
    let preview=match kiroku_core::catalog::preview(&root,&dirs,&settings.name_rules,&cancel){Ok(p)=>p,Err(e)=>{eprintln!("{e}");return ExitCode::FAILURE}};
    if !apply {println!("{}",serde_json::to_string_pretty(&preview).unwrap());return ExitCode::SUCCESS;}
    match kiroku_core::catalog::apply(&preview,&settings.name_rules,&cancel,&|p|eprintln!("{}/{} {}",p.done,p.total,p.code)) {
      Ok(out)=> {if let Ok(lib)=Library::open(&root){for item in &out.items {if item.journal_id.is_some(){if let Err(e)=lib.reindex_dir(&item.dir){eprintln!("索引更新失败: {e}");return ExitCode::FAILURE;}}}}else{eprintln!("索引未打开，请重建索引");}
        println!("{}",serde_json::to_string_pretty(&out).unwrap());if out.cancelled || out.items.iter().any(|i|i.error.is_some()){ExitCode::FAILURE}else{ExitCode::SUCCESS}}
      Err(e)=>{eprintln!("{e}");ExitCode::FAILURE}
    }
}

fn revalidate_links(args:&[String])->ExitCode {let run=||->Result<(),Box<dyn std::error::Error>>{let root=PathBuf::from(flag(args,"--root").ok_or("需要 --root")?);let target=flag(args,"--target").map(PathBuf::from);let dirs=kiroku_core::append::targets(&root,&args[0])?;let dir=target.or_else(||(dirs.len()==1).then(||dirs[0].clone())).ok_or("需要明确 --target 影片目录")?;if !dirs.contains(&dir.canonicalize()?){return Err("目标影片不匹配".into())}let preview=kiroku_core::placement::revalidation_preview(&root,&dir,&args[0])?;if args.iter().any(|a|a=="--apply"){let out=kiroku_core::revision::apply(&preview)?;if let Some(e)=out.error{return Err(e.into())}Library::open(&root)?.reindex_dir(&dir)?;println!("日志 {}",out.journal.id);}else{println!("{}",serde_json::to_string_pretty(&preview)?);}Ok(())};match run(){Ok(())=>ExitCode::SUCCESS,Err(e)=>{eprintln!("{e}");ExitCode::FAILURE}}}

fn export_nfo(args:&[String])->ExitCode {
 let run=||->Result<(),Box<dyn std::error::Error>>{
  let get=|name:&str|args.iter().position(|s|s==name).and_then(|i|args.get(i+1));
  let root=std::path::Path::new(get("--root").ok_or("需要 --root <库目录>")?);
  let language=match get("--language").map(String::as_str).unwrap_or("current"){"current"=>kiroku_core::localization::NfoLanguage::Current,"zh-Hans"=>kiroku_core::localization::NfoLanguage::ZhHans,"original"=>kiroku_core::localization::NfoLanguage::Original,_=>return Err("语言只能为 current、zh-Hans 或 original".into())};
  let cancel=cli_cancel();let mut dirs=vec![];let mut i=0;while i<args.len(){match args[i].as_str(){"--root"|"--language"=>i+=2,"--apply"=>i+=1,s if s.starts_with("--")=>return Err(format!("未知参数: {s}").into()),_=>{dirs.push(PathBuf::from(&args[i]));i+=1;}}}
  let mut previews=vec![];let mut seen=std::collections::HashSet::new();for dir in dirs{let dir=dir.canonicalize()?;if !seen.insert(dir.clone()){continue;}let meta:Metadata=match std::fs::read(dir.join(organize::SIDECAR)){Ok(bytes)=>serde_json::from_slice(&bytes)?,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{let nfos=std::fs::read_dir(&dir)?.filter_map(Result::ok).map(|e|e.path()).filter(|p|p.extension().is_some_and(|e|e.eq_ignore_ascii_case("nfo"))).collect::<Vec<_>>();if nfos.len()!=1{return Err("影片目录必须有一个明确 NFO".into());}let fallback=kiroku_core::code::parse_code(&dir.file_name().unwrap_or_default().to_string_lossy()).map(|c|c.id).unwrap_or_default();let meta=kiroku_core::nfo_read::parse_nfo(&std::fs::read_to_string(&nfos[0])?,&fallback).ok_or("无法解析 NFO")?;if meta.code.is_empty(){return Err("NFO 和目录中都没有可识别番号".into());}meta},Err(e)=>return Err(e.into())};previews.push(kiroku_core::localization::export_preview(root,&dir,&meta.code,language)?);}
  if args.iter().any(|s|s=="--apply"){let lib=Library::open(root)?;for preview in previews{if cancel.is_cancelled(){break;}let out=kiroku_core::revision::apply(&preview)?;println!("{}",serde_json::json!({"code":preview.code,"journalId":out.journal.id,"warnings":preview.warnings,"error":out.error}));lib.reindex_dir(&preview.dir)?;if let Some(e)=out.error{return Err(e.into());}}}else{println!("{}",serde_json::to_string_pretty(&previews)?);}Ok(())
 };match run(){Ok(())=>ExitCode::SUCCESS,Err(e)=>{eprintln!("{e}");ExitCode::FAILURE}}
}
