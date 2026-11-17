mod commands;
mod state;
mod pending;
mod history;
#[cfg(target_os = "macos")]
mod macos_shutdown;

use kiroku_core::{logging, settings::Settings};
use state::{AppState, Shutdown};
use std::path::Path;
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 设置文件损坏时不覆盖它:用默认值启动,保存时才会写入
    let settings = Settings::load(&Settings::default_path()).unwrap_or_default();
    logging::init(logging::log_dir(&Settings::default_path()), &settings.log);
    if let Err(e)=kiroku_core::nas_ops::configure_file(&Settings::default_path()){log::error!("NAS 配置加载失败：{e}");}

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new(settings))
        .setup(|app| {
            // 默认展示更多内容，同时避开小屏幕的菜单栏、Dock 和窗口标题栏。
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(Some(monitor)) = window.current_monitor() {
                    let area = monitor.work_area();
                    let scale = monitor.scale_factor();
                    let width = (area.size.width as f64 / scale - 32.0).clamp(320.0, 1280.0);
                    let height = (area.size.height as f64 / scale - 64.0).clamp(240.0, 860.0);
                    window.set_min_size(Some(tauri::LogicalSize::new(width.min(720.0), height.min(520.0))))?;
                    window.set_size(tauri::LogicalSize::new(width, height))?;
                    window.center()?;
                }
            }
            #[cfg(target_os = "macos")]
            macos_shutdown::install(app.handle())?;
            // App 内日志:转成 log:line 事件,带上当前任务的 jobId(无任务时为 0)
            let handle = app.handle().clone();
            logging::set_app_sink(Box::new(move |line| {
                let job = handle.state::<AppState>().current_job_id().unwrap_or(0);
                state::emit(&handle, "log:line", job, line.clone());
            }));
            // 启动时恢复媒体库
            if let Some(root) = app.state::<AppState>().library_root().filter(|r| r.is_dir()) {
                if let Err(e) = commands::open_library(app.handle(), &root) {
                    eprintln!("打开媒体库失败 {}: {e}", Path::new(&root).display());
                }
            }
            Ok(())
        })
        .invoke_handler(command_handler())
        .build(app_context())
        .expect("error while building kiroku");

    app.run(handle_run_event);
}

fn app_context() -> tauri::Context<tauri::Wry> {
    tauri::generate_context!()
}

/// Shared with native smoke checks so command names and argument decoding stay real.
fn command_handler() -> impl Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
            commands::get_settings,
            commands::catalog_inventory,
            commands::catalog_preview,
            commands::catalog_apply,
            commands::template_preview,
            commands::batch::batch_edit_preview,
            commands::batch::batch_edit_apply,
            commands::batch::batch_edit_discard,
            commands::nfo_export_preview,
            commands::nfo_export_apply,
            commands::pick_media_files,
            commands::append_targets,
            commands::append_target_videos,
            commands::append_preview,
            commands::append_apply,
            commands::sources_check,
            commands::library_health,
            commands::maintenance_preview,
            commands::maintenance_apply,
            commands::reveal_library_path,
            commands::default_settings,
            commands::settings_path,
            commands::save_settings,
            commands::set_library_root,
            commands::pick_directory,
            commands::media_dirs,
            commands::plex_test,
            commands::media_server_test,
            commands::ai_test,
            commands::inspect_directory,
            commands::classify_paths,
            pending::pending_list,
            pending::pending_summary,
            pending::pending_cancel,
            commands::set_source_dirs,
            commands::scan_start,
            commands::set_item_code,
            commands::scrape_start,
            commands::scrape_url_start,
            commands::translation_cache_clear,
            commands::import_cache_clear,
            commands::notify_finish,
            commands::tag_preset_merge,
            commands::name_translation,
            commands::plan_build,
            commands::apply_start,
            commands::job_cancel,
            history::history_list,
            history::history_detail,
            history::history_cancel,
            commands::recovery_inspect,
            commands::reveal_journal,
            commands::rollback,
            commands::revision_preview,
            commands::artwork_open,
            commands::artwork_preview,
            commands::metadata_update_preview,
            commands::metadata_source_candidates,
            commands::revision_save,
            commands::open_source_url,
            commands::library_list,
            commands::library_overview,
            commands::library_export_csv,
            commands::library_prune_missing,
            commands::library_get,
            commands::actor_image,
            commands::library_trash,
            commands::repair_images_start,
            commands::library_facets,
            commands::library_rebuild,
            commands::play,
            commands::reveal,
            commands::open_log_dir,
        ]
}

fn handle_run_event(handle: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::WindowEvent { label, event: WindowEvent::CloseRequested { api, .. }, .. }
            if label == "main" => {
                if defer_shutdown(handle, 0) { api.prevent_close(); }
            }
        RunEvent::ExitRequested { code, api, .. } => {
            if defer_shutdown(handle, code.unwrap_or(0)) { api.prevent_exit(); }
        }
        _ => {}
    }
}

/// Keep the window/event loop alive until the owning guard finishes disk work.
/// The close decision also prevents any new job from entering the slot.
fn defer_shutdown(handle: &AppHandle, exit_code: i32) -> bool {
    match handle.state::<AppState>().request_shutdown() {
        Shutdown::Ready => false,
        Shutdown::Pending => true,
        Shutdown::Wait => {
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.set_title("kiroku · 正在结束任务，请稍候");
            }
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                handle.state::<AppState>().wait_for_shutdown().await;
                handle.exit(exit_code);
            });
            true
        }
    }
}
