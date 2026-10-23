//! Run explicitly on macOS; cargo test never launches this native window.
//! Reuse the actual event handler/state/bridge, with no settings or media access.
#[cfg(target_os = "macos")]
include!("../apps/desktop/src-tauri/src/lib.rs");

#[cfg(target_os = "macos")]
fn main() {
    use std::time::{Duration, Instant};
    let mode = std::env::args().nth(1).expect("mode: close | native | combined | exit | idle | fast");
    assert!(["close", "native", "combined", "exit", "idle", "fast"].contains(&mode.as_str()));
    let started = Instant::now();
    // Only use Tauri's inert context/assets helper; Builder::default is real Wry,
    // not MockRuntime. No production setup, commands, settings, or files run.
    let app = tauri::Builder::default()
        .manage(AppState::new(Settings::default()))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::App("index.html".into()))
        .title("kiroku · 临时退出验收").inner_size(640.0, 420.0).build().unwrap();
    macos_shutdown::install(app.handle()).unwrap();
    if mode != "idle" {
        let (id, cancel) = app.state::<AppState>().begin_job("smoke-commit").unwrap();
        let guard = state::JobGuard { app: app.handle().clone(), id };
        let handle = app.handle().clone();
        let fast = mode == "fast";
        tauri::async_runtime::spawn(async move {
            cancel.cancelled().await;
            if !fast {
                tokio::time::sleep(Duration::from_secs(6)).await;
                assert_eq!(handle.state::<AppState>().current_job_id(), Some(id));
                assert!(handle.state::<AppState>().begin_job("unwanted").is_err());
                assert!(handle.get_webview_window("main").unwrap().is_visible().unwrap());
                println!("SMOKE alive_after_6s cancelled=true new_job=rejected window=visible");
            }
            println!("SMOKE commit_finished elapsed_ms={}", started.elapsed().as_millis());
            drop(guard);
        });
    }
    let mut closes = 0;
    let mut exits = 0;
    app.run(move |handle, event| {
        match &event {
            RunEvent::Ready => {
                println!("SMOKE ready mode={mode}");
                if mode == "close" || mode == "combined" {
                    handle.get_webview_window("main").unwrap().close().unwrap();
                }
                if mode == "exit" { handle.exit(23); }
                if ["native", "combined", "idle", "fast"].contains(&mode.as_str()) {
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        // Terminate only after applicationDidFinishLaunching returns.
                        for _ in 0..2 {
                            tokio::time::sleep(Duration::from_millis(400)).await;
                            handle.run_on_main_thread(|| unsafe {
                                println!("SMOKE native_terminate");
                                let app: *mut objc2::runtime::AnyObject = objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
                                // Schedule the same selector used by Cmd+Q/menu/Dock
                                // Quit outside Tao's callback lock. Calling terminate:
                                // synchronously here would reenter that lock on Exit.
                                let _: () = objc2::msg_send![app,
                                    performSelector: objc2::sel!(terminate:),
                                    withObject: std::ptr::null::<objc2::runtime::AnyObject>(),
                                    afterDelay: 0.0f64
                                ];
                            }).unwrap();
                        }
                    });
                }
            }
            RunEvent::WindowEvent { event: WindowEvent::CloseRequested { .. }, .. } => {
                closes += 1;
                println!("SMOKE close_requested count={closes}");
                if closes == 1 { handle.get_webview_window("main").unwrap().close().unwrap(); }
            }
            RunEvent::ExitRequested { code, .. } => {
                exits += 1;
                println!("SMOKE exit_requested count={exits} code={code:?} job={:?}", handle.state::<AppState>().current_job_id());
            }
            RunEvent::Exit => println!(
                "SMOKE exited elapsed_ms={} closes={closes} exits={exits} job_running={}",
                started.elapsed().as_millis(), handle.state::<AppState>().job_running(),
            ),
            _ => {}
        }
        handle_run_event(handle, event);
    });
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("native_shutdown_smoke requires macOS");
    std::process::exit(1);
}
