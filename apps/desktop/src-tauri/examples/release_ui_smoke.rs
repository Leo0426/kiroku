//! Explicit macOS check of bundled assets, production CSP and native IPC.
#[cfg(target_os = "macos")]
include!("../src/lib.rs");

#[cfg(target_os = "macos")]
#[tauri::command]
fn release_report(app: tauri::AppHandle, message: String) {
    eprintln!("RELEASE UI: {message}");
    if message.starts_with("READY:") {
        let expected = if app.state::<AppState>().settings().library_root.is_some() { "library rendered" } else { "onboarding rendered" };
        app.exit(if message.ends_with(expected) { 0 } else { 1 });
    }
    if message.starts_with("ERROR:") || message.starts_with("REJECTION:") { app.exit(1); }
}

#[cfg(target_os = "macos")]
fn report_handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![release_report]
}

#[cfg(target_os = "macos")]
fn main() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", temp.path());
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.is_empty() || args == ["--library"], "expected no arguments or --library");
    let mut settings = Settings::default();
    if !args.is_empty() { settings.library_root = Some(temp.path().join("library").display().to_string()); }
    let state = AppState::new(settings.clone());
    if let Some(root) = &settings.library_root { state.replace_library(Some(kiroku_core::library::Library::open(Path::new(root)).unwrap())); }
    let mut context = app_context();
    context.config_mut().app.windows.clear();
    context.config_mut().build.dev_url = None;
    let production = command_handler();
    let probe = report_handler();
    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(move |invoke| {
            if invoke.message.command() == "release_report" { probe(invoke) } else { production(invoke) }
        })
        .build(context).unwrap();
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
    tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
        .incognito(true).initialization_script(script)
        .title("kiroku · production smoke").build().unwrap();
    let timeout = app.handle().clone();
    let (stop, waiting) = std::sync::mpsc::channel();
    let watchdog = std::thread::spawn(move || {
        if waiting.recv_timeout(std::time::Duration::from_secs(10)).is_err() { timeout.exit(1); }
    });
    let code = app.run_return(|_,_|{});
    let _ = stop.send(());
    watchdog.join().unwrap();
    temp.close().unwrap();
    std::process::exit(code);
}

#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("release_ui_smoke requires macOS"); }
