mod commands;
mod state;
mod pending;
mod history;

use kiroku_core::{logging, settings::Settings};
use state::AppState;
use std::{path::Path, time::Duration};
use tauri::{Manager, RunEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 设置文件损坏时不覆盖它:用默认值启动,保存时才会写入
    let settings = Settings::load(&Settings::default_path()).unwrap_or_default();
    logging::init(logging::log_dir(&Settings::default_path()), &settings.log);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new(settings))
        .setup(|app| {
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
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::catalog_inventory,
            commands::catalog_preview,
            commands::catalog_apply,
            commands::template_preview,
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
            commands::notify_finish,
            commands::tag_preset_merge,
            commands::plan_build,
            commands::apply_start,
            commands::job_cancel,
            history::history_list,
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
            commands::library_get,
            commands::library_trash,
            commands::repair_images_start,
            commands::library_facets,
            commands::library_rebuild,
            commands::play,
            commands::reveal,
            commands::open_log_dir,
        ])
        .build(tauri::generate_context!())
        .expect("error while building kiroku");

    app.run(|handle, event| {
        // 用户关闭窗口时若任务仍在运行:先取消并等日志落盘(最多 5 秒)再退出
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            let st = handle.state::<AppState>();
            if st.job_running() {
                api.prevent_exit();
                st.cancel_job();
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    for _ in 0..50 {
                        if !h.state::<AppState>().job_running() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    h.exit(0);
                });
            }
        }
    });
}
