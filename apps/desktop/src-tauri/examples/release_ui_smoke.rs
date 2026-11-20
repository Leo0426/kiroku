//! Explicit macOS check of bundled assets, production CSP and native IPC.
#[cfg(target_os = "macos")]
include!("../src/lib.rs");

#[cfg(target_os = "macos")]
#[path = "release_smoke/linked_crop.rs"]
mod linked_crop;

#[cfg(target_os = "macos")]
#[tauri::command]
fn release_report(app: tauri::AppHandle, message: String) {
    eprintln!("RELEASE UI: {message}");
    if message.starts_with("READY:") {
        let expected = if std::env::args().nth(1).as_deref() == Some("--linked-crop") { "linked crop roundtrip" } else if std::env::args().nth(1).as_deref() == Some("--posters") { "posters decoded" } else if std::env::args().nth(1).as_deref() == Some("--transfer") { "package roundtrip" } else if app.state::<AppState>().settings().library_root.is_some() { "library rendered" } else { "onboarding rendered" };
        app.exit(if message.ends_with(expected) { 0 } else { 1 });
    }
    if message.starts_with("ERROR:") || message.starts_with("REJECTION:") { app.exit(1); }
}

#[cfg(target_os = "macos")]
fn report_handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![release_report, linked_crop::release_linked_files, linked_crop::release_scan_fixture]
}

#[cfg(target_os = "macos")]
fn main() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", temp.path());
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let transfer = args == ["--transfer"];
    let posters = args == ["--posters"];
    let linked = args == ["--linked-crop"];
    assert!(args.is_empty() || args == ["--library"] || transfer || posters || linked, "expected no arguments, --library, --transfer, --posters or --linked-crop");
    let mut settings = Settings::default();
    if !args.is_empty() { settings.library_root = Some(temp.path().join("library").display().to_string()); }
    if linked {
        settings.source_dirs = vec![temp.path().join("source").display().to_string()];
        settings.plex.naming = true;
        settings.badges.enabled = true;
    }
    let state = AppState::new(settings.clone());
    if let Some(root) = &settings.library_root {
        let lib = kiroku_core::library::Library::open(std::path::Path::new(root)).unwrap();
        if linked { linked_crop::seed(temp.path(), &lib, &settings); }
        if transfer {
            let dir = std::path::Path::new(root).join("TEST-101");
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("TEST-101.mp4"), b"synthetic video").unwrap();
            std::fs::write(dir.join("movie.nfo"), b"<movie><num>TEST-101</num><title>Fixture</title></movie>").unwrap();
            lib.reindex_dir(&dir).unwrap();
        }
        if posters {
            for (code, folder) in [
                ("TEST-101", "simple"),
                ("TEST-102", "[样片] 50% #1 ? 与 空格"),
                ("TEST-103", "old-location"),
                ("TEST-104", "absent-location"),
            ] {
                let dir = std::path::Path::new(root).join(folder);
                std::fs::create_dir(&dir).unwrap();
                std::fs::write(dir.join(format!("{code}.mp4")), b"synthetic video").unwrap();
                std::fs::write(dir.join("movie.nfo"), format!("<movie><num>{code}</num><title>Fixture</title></movie>")).unwrap();
                std::fs::write(dir.join("poster.png"), include_bytes!("../icons/32x32.png")).unwrap();
                lib.reindex_dir(&dir).unwrap();
            }
            // Simulate a poster seen before the source directory was moved offline.
            let version = lib.get("TEST-104").unwrap().unwrap().card.poster_version.unwrap();
            let cache = Settings::default_path().parent().unwrap().join("cache/posters-v1");
            std::fs::create_dir_all(&cache).unwrap();
            std::fs::copy(std::path::Path::new(root).join("absent-location/poster.png"), cache.join(format!("{version}.png"))).unwrap();
            let library = std::path::Path::new(root);
            std::fs::rename(library.join("old-location"), library.join("relocated")).unwrap();
            std::fs::rename(library.join("absent-location"), temp.path().join("outside-library")).unwrap();
        }
        state.replace_library(Some(lib));
    }
    let mut context = app_context();
    context.config_mut().app.windows.clear();
    context.config_mut().build.dev_url = None;
    let production = command_handler();
    let probe = report_handler();
    let app = tauri::Builder::default()
        .manage(state)
        .manage(linked_crop::ScanFixture::default())
        .invoke_handler(move |invoke| {
            if matches!(invoke.message.command(), "release_report" | "release_linked_files" | "release_scan_fixture") { probe(invoke) } else { production(invoke) }
        })
        .build(context).unwrap();
    if posters || linked {
        // Production registers the library root when opening its index.
        state::allow_library_assets(app.handle(), std::path::Path::new(settings.library_root.as_ref().unwrap()));
    }
    let script = r#"
        const report = message => window.__TAURI_INTERNALS__.invoke('release_report', {message}).catch(()=>{});
        window.addEventListener('error', e => report('ERROR: '+(e.message || e.target?.src || e.target?.href)), true);
        window.addEventListener('unhandledrejection', e => report('REJECTION: '+String(e.reason)));
        window.addEventListener('DOMContentLoaded', () => {
            report('DOM loaded '+location.href);
            const probe=setInterval(() => {
                const library = document.querySelector('[aria-label="主导航"]') && document.body.innerText.includes('媒体库还是空的');
                if(library || document.body.innerText.includes('欢迎使用 kiroku')) {clearInterval(probe);report('READY: production '+(library?'library':'onboarding')+' rendered');}
            },100);
            setTimeout(() => report('BODY: '+document.body.innerText.slice(0,300)),3000);
        });
    "#;
    // Exercise the production bridge with an isolated source movie and an empty export target.
    let script = if linked {
        include_str!("release_smoke/linked_crop.js").to_owned()
    } else if transfer {
        let transfer_js = r#"
          async function transferPackage(){
            const invoke=(name,params)=>window.__TAURI_INTERNALS__.invoke(name,params)
            const options={codes:['TEST-101'],kinds:['nfo','video'],requestId:'fixture-preview'}
            const result=await invoke('package_preview',{...options,directory:__LIBRARY__}).then(
              ()=>'', error=>String(error))
            if(!result.includes('媒体库之外'))throw Error('expected the library target to be rejected: '+result)
            const preview=await invoke('package_preview',{...options,directory:__DIRECTORY__})
            if(preview.movies!==1||preview.files!==2)throw Error('incorrect file preview')
            const copied=await invoke('package_export',{token:preview.token,requestId:'fixture-copy'})
            if(copied.files!==2||copied.destination!==preview.destination)throw Error('incorrect copy result')
            const review=await invoke('package_restore_preview',{directory:copied.destination,rules:{},requestId:'fixture-read'})
            if(review.items.length!==1||review.items[0].status!=='unchanged')throw Error('inconsistent roundtrip: '+JSON.stringify(review.items))
            report('READY: production package roundtrip')
          }
          function waitForLibrary(){
            if(!document.body.innerText.includes('TEST-101')){setTimeout(waitForLibrary,100);return}
            transferPackage().catch(e=>report('ERROR: '+String(e)))
          }
          document.addEventListener('DOMContentLoaded', waitForLibrary)
        "#;
        let common = script.split("window.addEventListener('DOMContentLoaded'").next().unwrap();
        format!("{common}{}",transfer_js.replace("__LIBRARY__", &serde_json::to_string(settings.library_root.as_ref().unwrap()).unwrap())
          .replace("__DIRECTORY__", &serde_json::to_string(temp.path()).unwrap()))
    } else if posters {
        let common = script.split("window.addEventListener('error'").next().unwrap();
        let check = r#"
          window.addEventListener('error', e => {
            if (e.target instanceof HTMLImageElement &&
                /old-location|absent-location/.test(e.target.src)) return;
            report('ERROR: '+(e.message || e.target?.src || e.target?.href));
          }, true);
          window.addEventListener('unhandledrejection', e => report('REJECTION: '+String(e.reason)));
          async function runPosters() {
            const invoke=(name,args)=>window.__TAURI_INTERNALS__.invoke(name,args);
            const card = code => document.querySelector('button.pcard[aria-label^="'+code+' "]');
            async function until(code, stage, condition) {
              for (let n=0; n<80; n++) {
                const target=card(code);
                const image=target?.querySelector('.poster img');
                target?.scrollIntoView({block:'center'});
                if (image && image.loading!=='eager') image.loading='eager';
                if (condition(image)) return;
                await new Promise(resolve => setTimeout(resolve,60));
              }
              const image=card(code)?.querySelector('.poster img');
              throw Error(stage+' timed out for '+code+': complete='+image?.complete+
                          ', width='+image?.naturalWidth+', src='+image?.src);
            }
            await until('TEST-104','offline poster',image =>
              image?.naturalWidth === 32 && decodeURIComponent(image.src).includes('/cache/posters-v1/'));
            const versions=await invoke('library_get',{code:'TEST-104'});
            if(await invoke('cached_poster',{code:'TEST-104',version:'0'.repeat(64)}))
              throw Error('unexpected access to unrelated cache key');
            const expectedPath=await invoke('cached_poster',{code:'TEST-104',version:versions.posterVersion});
            if(!expectedPath?.endsWith(versions.posterVersion+'.png'))
              throw Error('expected a verified offline poster');

            const refresh = codes => invoke('library_prune_missing',{
              root:__ROOT__,automatic:false,removeMissing:codes
            });
            const first = await refresh([]);
            if (first.relocated!==1 || first.removed!==0 || first.missingCodes?.join()!=='TEST-104')
              throw Error('incorrect refresh preview '+JSON.stringify(first));
            const missing = await invoke('library_get',{code:'TEST-104'});
            if (!missing) throw Error('unconfirmed missing record was removed');
            const result = await refresh(first.missingCodes);
            if (result.removed!==1) throw Error('expected confirmed removal');
            await until('TEST-103','refreshed library',image =>
              !card('TEST-104') && image?.src.includes('relocated'));
            for (const code of ['TEST-101','TEST-102','TEST-103']) {
              const image = card(code).querySelector('.poster img');
              if (!image) throw Error('poster missing: '+code);
              image.loading='eager';
              await image.decode();
              if (image.naturalWidth!==32) throw Error('invalid decoded poster: '+code);
              const url = new URL(image.src);
              if (!url.searchParams.has('v')) throw Error('expected versioned poster URL');
              url.searchParams.delete('v');
              const plain = new Image();
              plain.src=url.href;
              await plain.decode();
              if (plain.naturalWidth!==image.naturalWidth) throw Error('unversioned poster differs');
            }
            report('READY: production posters decoded');
          }
          document.addEventListener('DOMContentLoaded', () => runPosters().catch(e=>report('ERROR: '+e)));
        "#;
        format!("{common}{}",check.replace("__ROOT__", &serde_json::to_string(settings.library_root.as_ref().unwrap()).unwrap()))
    } else { script.to_string() };
    tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
        .incognito(true).initialization_script(script)
        .title("kiroku · production smoke").build().unwrap();
    let timeout = app.handle().clone();
    let (stop, waiting) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if waiting.recv_timeout(std::time::Duration::from_secs(if linked { 30 } else { 10 })).is_err() { timeout.exit(1); }
    });
    let code = app.run_return(|_,_|{});
    let _ = stop.send(());
    watchdog.join().unwrap();
    temp.close().unwrap();
    std::process::exit(code);
}

#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("release_ui_smoke requires macOS"); }
