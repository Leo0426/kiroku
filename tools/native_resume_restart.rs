//! Two-process production UI check. Only an explicitly supplied empty temp root is used.
#[cfg(target_os = "macos")]
include!("../apps/desktop/src-tauri/src/lib.rs");

#[cfg(target_os = "macos")]
mod restart {
    use super::*;
    use kiroku_core::{
        code::{Code, ContentType},
        journal::{Checkpoint, Journal, JournalItem, JournalStatus},
        metadata::PartialMetadata,
        net::Http,
        organize::Plan,
        pipeline,
        source::{Source, SourceError, SourceId},
    };
    use std::{fs, path::PathBuf};
    use tauri::State;
    use tokio_util::sync::CancellationToken;
    struct Neutral;
    #[async_trait::async_trait]
    impl Source for Neutral {
        fn id(&self) -> SourceId {
            "javbus"
        }
        fn supports(&self, _: &Code) -> bool {
            true
        }
        async fn fetch(&self, code: &Code, _: &Http) -> Result<PartialMetadata, SourceError> {
            Ok(PartialMetadata {
                title: Some(format!("Neutral {}", code.id)),
                ..Default::default()
            })
        }
    }
    pub async fn prepare(root: &std::path::Path) {
        assert!(
            root.is_dir() && fs::read_dir(root).unwrap().next().is_none(),
            "requires empty isolated directory"
        );
        fs::write(root.join("test-owner"), b"kiroku-neutral-restart").unwrap();
        let source = root.join("source");
        let library = root.join("library");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&library).unwrap();
        for code in ["RST-001", "RST-002", "RST-003"] {
            fs::write(source.join(format!("{code}.mp4")), b"neutral restart video").unwrap();
        }
        let mut settings = Settings::default();
        settings.library_root = Some(library.display().to_string());
        settings.source_dirs = vec![source.display().to_string()];
        settings.scan.min_video_mb = 0;
        settings.cleanup_source = false;
        settings.ai.enabled = false;
        settings.notify_on_finish = false;
        settings.plex.enabled = false;
        settings.media_server.enabled = false;
        settings.dir_template = "{code}".into();
        settings.file_template = "{code}".into();
        settings.routes.clear();
        settings
            .routes
            .insert(ContentType::Censored, vec!["javbus".into()]);
        settings.sources.retain(|s| s.id == "javbus");
        settings.sources[0].enabled = true;
        settings.sources[0].base = "https://neutral.invalid".into();
        settings.sources[0].cookie.clear();
        settings.proxy = None;
        settings.sources[0].proxy = kiroku_core::settings::SourceProxy::Direct;
        settings.save(&Settings::default_path()).unwrap();
        let cancel = CancellationToken::new();
        let scan = pipeline::scan(
            &[source],
            Some(&library),
            &settings.video_filter(),
            &cancel,
            &mut |_| {},
        );
        let mut items = scan.items;
        items.sort_by_key(|i| i.code.as_ref().unwrap().id.clone());
        let scraper = pipeline::Scraper::with_sources(settings.clone(), vec![Box::new(Neutral)])
            .unwrap()
            .with_import_cache(kiroku_core::import_cache::cache_path(
                &Settings::default_path(),
            ));
        let codes = items
            .iter()
            .map(|i| i.code.clone().unwrap())
            .collect::<Vec<_>>();
        let metas =
            pipeline::scrape_missing(&scraper, &codes, &Default::default(), &cancel, &|_| {})
                .await
                .into_iter()
                .map(|(c, m)| (c, m.unwrap()))
                .collect();
        let pairs = items
            .iter()
            .map(|i| (i, i.code.as_ref().unwrap()))
            .collect::<Vec<_>>();
        let mut plans =
            pipeline::build_plans(&pairs, &metas, &library, "{code}", settings.plan_opts());
        let first = plans.remove(0);
        let second = plans.remove(0);
        let out =
            pipeline::apply(vec![first], &scraper.http, &library, None, &cancel, &|_| {}).await;
        assert!(out.failed.is_empty() && out.applied.len() == 1);
        let mut journal = Journal::load(&out.journal.path()).unwrap();
        journal.status = JournalStatus::Partial;
        journal.items.push(JournalItem {
            code: second.code,
            dir: second.to.parent().unwrap().into(),
        });
        let result = journal.execute_observed(
            &Plan { ops: second.ops },
            &mut |_| unreachable!(),
            &|| false,
            &mut |p| {
                if p == Checkpoint::Intent {
                    Err(std::io::Error::other("restart fixture interruption"))
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.error.is_some() && journal.pending.is_some());
        println!("PASS restart_fixture_persisted");
    }
    pub struct Probe {
        root: PathBuf,
    }
    #[tauri::command]
    pub async fn restart_report(
        app: AppHandle,
        state: State<'_, AppState>,
        probe: State<'_, Probe>,
        message: String,
    ) -> Result<(), String> {
        if message == "DONE" {
            let busy = state.job_running();
            let root = probe.root.clone();
            let verify = tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
                if busy {
                    return Err("active job at finish".into());
                }
                let library = root.join("library");
                let index =
                    kiroku_core::library::Library::open(&library).map_err(|e| e.to_string())?;
                for code in ["RST-001", "RST-002", "RST-003"] {
                    if fs::read(root.join("source").join(format!("{code}.mp4")))
                        .map_err(|e| e.to_string())?
                        != b"neutral restart video"
                        || library.join(code).exists()
                        || index.get(code).map_err(|e| e.to_string())?.is_some()
                    {
                        return Err(format!("restart restore mismatch: {code}"));
                    }
                }
                let journals = Journal::list(&library);
                if journals.len() != 2
                    || journals
                        .iter()
                        .any(|j| j.status != JournalStatus::RolledBack || j.pending.is_some())
                {
                    return Err("restart journals not restored".into());
                }
                Ok(())
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            match verify {
                Ok(()) => {
                    println!("PASS restart_production_ui_resume_undo");
                    app.exit(0)
                }
                Err(e) => {
                    eprintln!("RESTART FAILED: {e}");
                    app.exit(1)
                }
            }
        } else {
            eprintln!("RESTART: {message}");
            if message.starts_with("ERROR:") {
                app.exit(1)
            }
        }
        Ok(())
    }
    fn controls() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
        tauri::generate_handler![restart_report]
    }
    pub fn run(root: PathBuf) {
        assert_eq!(
            fs::read(root.join("test-owner")).unwrap(),
            b"kiroku-neutral-restart"
        );
        let settings = Settings::load(&Settings::default_path()).unwrap();
        assert_eq!(
            settings.library_root.as_deref(),
            root.join("library").to_str()
        );
        let state = AppState::new(settings);
        state.replace_library(Some(
            kiroku_core::library::Library::open(&root.join("library")).unwrap(),
        ));
        let production = command_handler();
        let controls = controls();
        let mut context = app_context();
        context.config_mut().app.windows.clear();
        context.config_mut().build.dev_url = None;
        let app = tauri::Builder::default()
            .manage(state)
            .manage(Probe { root })
            .invoke_handler(move |invoke| {
                if invoke.message.command() == "restart_report" {
                    controls(invoke)
                } else {
                    production(invoke)
                }
            })
            .build(context)
            .unwrap();
        let script = include_str!("native_resume_restart.js");
        tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
            .incognito(true)
            .initialization_script(script)
            .title("kiroku · restart acceptance")
            .inner_size(1100.0, 800.0)
            .build()
            .unwrap();
        let handle = app.handle().clone();
        let (stop, wait) = std::sync::mpsc::channel();
        let watcher = std::thread::spawn(move || {
            if wait
                .recv_timeout(std::time::Duration::from_secs(45))
                .is_err()
            {
                handle.exit(1)
            }
        });
        let code = app.run_return(|_, _| {});
        let _ = stop.send(());
        watcher.join().unwrap();
        std::process::exit(code);
    }
}
#[cfg(target_os = "macos")]
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.len() == 2 && matches!(args[0].as_str(), "--prepare" | "--run"));
    let root = std::fs::canonicalize(&args[1]).unwrap();
    std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
    assert_eq!(
        Settings::default_path(),
        root.join("config/kiroku/settings.json"),
        "isolated config path required"
    );
    if args[0] == "--prepare" {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(restart::prepare(&root))
    } else {
        restart::run(root)
    }
}
#[cfg(not(target_os = "macos"))]
fn main() {
    panic!("macOS only")
}
