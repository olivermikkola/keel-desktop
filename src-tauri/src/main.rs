#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod arena;
mod commands;
mod productivity;
mod updater;

use commands::AppState;
use keel_core::runtime::RuntimeManager;
use keel_core::sessions::{default_config_dir, default_data_dir, load_config};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Listener, Manager, RunEvent};
use tokio::sync::Mutex;

fn extract_workspace_arg(args: &[String]) -> Option<String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--smoke-test" || arg == "--ui-smoke-test" {
            continue;
        }
        if let Some(val) = arg
            .strip_prefix("--root=")
            .or_else(|| arg.strip_prefix("--workspace="))
        {
            if !val.trim().is_empty() {
                return Some(val.to_string());
            }
            continue;
        }
        if arg == "--root" || arg == "--workspace" {
            if let Some(val) = iter.next() {
                if !val.trim().is_empty() {
                    return Some(val.clone());
                }
            }
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        return Some(arg.clone());
    }
    None
}

fn resolve_startup_workspace(args: &[String]) -> Option<String> {
    let raw = extract_workspace_arg(args)?;
    let path = std::path::PathBuf::from(&raw);
    match keel_core::tools::validate_workspace_root(&path) {
        Ok(canon) => Some(canon.to_string_lossy().into_owned()),
        Err(error) => {
            eprintln!("Ignoring startup folder `{raw}`: {error}");
            None
        }
    }
}

fn main() {
    let cli_args: Vec<String> = std::env::args().collect();
    if cli_args.iter().any(|a| a == "--smoke-test") {
        println!(
            "keel-desktop {} smoke-test ok (core={}, models={})",
            env!("CARGO_PKG_VERSION"),
            keel_core::version(),
            keel_core::models::default_catalog().len()
        );
        return;
    }

    let ui_smoke = cli_args.iter().any(|a| a == "--ui-smoke-test");
    let initial_workspace = if ui_smoke {
        None
    } else {
        resolve_startup_workspace(&cli_args)
    };
    let smoke_root = ui_smoke
        .then(|| std::env::temp_dir().join(format!("keel_ui_smoke_{}", uuid::Uuid::new_v4())));
    let config_dir = smoke_root
        .as_ref()
        .map(|p| p.join("config"))
        .unwrap_or_else(default_config_dir);
    let data_dir = smoke_root
        .as_ref()
        .map(|p| p.join("data"))
        .unwrap_or_else(default_data_dir);
    let mut cfg = load_config(&config_dir);
    if let Some(ref ws) = initial_workspace {
        cfg.recent_workspaces.retain(|w| w != ws);
        cfg.recent_workspaces.insert(0, ws.clone());
        cfg.recent_workspaces.truncate(10);
        let _ = keel_core::sessions::save_config(&config_dir, &cfg);
    }
    if cfg
        .external_base_url
        .as_deref()
        .is_some_and(keel_core::runtime::is_default_managed_loopback)
    {
        cfg.external_base_url = None;
        cfg.external_model_name = None;
        let _ = keel_core::sessions::save_config(&config_dir, &cfg);
    }
    if let Err(error) = keel_core::sessions::recover_interrupted(&data_dir) {
        eprintln!("Could not recover interrupted conversations: {error}");
    }

    let mut runtime = RuntimeManager::default();
    if let Some(ref url) = cfg.external_base_url {
        runtime.set_external_url(url, cfg.external_model_name.clone());
    }
    let runtime_arc = Arc::new(Mutex::new(runtime));
    let runtime_for_exit = Arc::clone(&runtime_arc);

    let state = AppState {
        images: keel_core::images::ImageService::new(&data_dir)
            .expect("failed to load image job history"),
        videos: keel_core::video::VideoService::new(&data_dir)
            .expect("failed to load video job history"),
        turn_lock: Arc::new(Mutex::new(())),
        config_dir,
        data_dir,
        initial_workspace,
        config: Arc::new(Mutex::new(cfg)),
        runtime: runtime_arc,
        approvals: Arc::new(Mutex::new(HashMap::new())),
        always_allowed: Arc::new(Mutex::new(HashMap::new())),
        active_abort: Arc::new(Mutex::new(None)),
        download_lock: Mutex::new(()),
        setup_pause: Mutex::new(None),
        active_cli_stop: Mutex::new(None),
        placement_cache: Mutex::new(None),
        mcp_cache: Mutex::new(None),
    };

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .setup(move |app| {
            if ui_smoke {
                let exit_handle = app.handle().clone();
                app.listen("keel://native-smoke-result", move |event| {
                    let payload: serde_json::Value = serde_json::from_str(event.payload()).unwrap_or_default();
                    let passed = payload["ready"] == true && payload["composer"] == true && payload["preset"] == "chat";
                    if passed { println!("PASS: native Tauri window bootstrapped through real IPC, created a Chat session, and rendered the composer."); }
                    else { eprintln!("Native frontend startup failed: {payload}"); }
                    exit_handle.exit(if passed {0} else {1});
                });
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    for _ in 0..30 {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        if let Some(window) = handle.get_webview_window("main") {
                            let _ = window.eval("if (window.__KEEL_READY__ || window.__KEEL_STARTUP_ERROR__) { window.__TAURI__.event.emit('keel://native-smoke-result', {ready: Boolean(window.__KEEL_READY__), composer: Boolean(document.getElementById('prompt-input')), preset: window.__KEEL_READY__?.preset, error: window.__KEEL_STARTUP_ERROR__}); }");
                        }
                    }
                    eprintln!("Native frontend startup timed out");
                    handle.exit(1);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            arena::pick_arena_results_file,
            arena::open_arena_compare,
            productivity::pick_documents,
            productivity::is_directory,
            productivity::import_document_from_bytes,
            productivity::import_documents_from_paths,
            productivity::open_models_folder,
            productivity::save_project,
            productivity::get_project,
            productivity::list_library,
            productivity::delete_project,
            productivity::search_threads,
            productivity::update_thread,
            productivity::fork_thread,
            productivity::list_trashed_threads,
            productivity::empty_trash,
            productivity::restore_thread,
            productivity::save_personal_context,
            productivity::export_thread,
            productivity::export_html_file,
            productivity::open_external_link,
            productivity::get_setup_plan,
            productivity::setup_local_ai,
            productivity::pause_local_setup,
            updater::check_app_update,
            updater::install_app_update,
            commands::connect_image_server,
            commands::disconnect_image_server,
            commands::configure_sd_cli_images,
            commands::clear_sd_cli_images,
            commands::detect_local_media_tools,
            commands::list_image_jobs,
            commands::start_image_job,
            commands::cancel_image_job,
            commands::read_workspace_png,
            commands::read_workspace_image,
            commands::list_video_jobs,
            commands::start_video_job,
            commands::cancel_video_job,
            commands::read_workspace_video,
            commands::detect_video_tools,
            commands::generate_video_script,
            commands::build_photo_slides,
            commands::list_photo_slides,
            commands::read_photo_slides_html,
            commands::generate_photo_slides_text,
            commands::run_document_check,
            commands::export_document_check_csv,
            commands::pick_workspace_folder,
            commands::prepare_sample_workspace,
            commands::set_workspace_folder,
            commands::list_workspace_files_cmd,
            commands::read_workspace_file_raw,
            commands::create_session,
            commands::import_keel_session,
            commands::reveal_keel_session_log,
            commands::export_keel_session_log,
            commands::load_session_cmd,
            commands::delete_session_cmd,
            commands::start_model,
            commands::auto_start_model,
            commands::check_current_model,
            commands::stop_runtime,
            commands::prepare_pack_vram,
            commands::list_capability_packs,
            commands::download_capability_pack,
            commands::remove_capability_pack,
            commands::get_runtime_status,
            commands::set_external_endpoint,
            commands::clear_external_endpoint,
            commands::download_catalog_model,
            commands::delete_model,
            commands::download_huggingface_gguf,
            commands::discover_models_cmd,
            commands::scan_user_models_folder,
            commands::import_local_gguf,
            commands::pick_model_file,
            commands::pick_audio_file,
            commands::send_message,
            commands::stop_generation,
            commands::respond_approval,
            commands::apply_staged_edit_cmd,
            commands::discard_staged_edit_cmd,
            commands::apply_all_staged_edits,
            commands::create_code_worktree,
            commands::review_code_worktree,
            commands::run_code_test,
            commands::apply_code_worktree,
            commands::undo_code_worktree,
            commands::remove_code_worktree,
            commands::inspect_publish_status,
            commands::publish_code_changes,
            commands::save_customize_settings,
            commands::save_mcp_server,
            commands::toggle_mcp_server,
            commands::remove_mcp_server,
            commands::set_workspace_mcp_trust,
            commands::refresh_mcp_servers,
            commands::list_audio_packs,
            commands::list_audio_artifacts,
            commands::clear_audio_artifacts,
            commands::register_audio_pack_runner,
            commands::register_whisper_cpp_runner,
            commands::register_piper_runner,
            commands::register_audio_cpp_runner,
            commands::download_audio_pack,
            commands::remove_audio_pack,
            commands::prepare_audio_pack_vram,
            commands::run_audio_stt,
            commands::run_audio_tts,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Keel Desktop");

    app.run(move |_handle, event| {
        if let RunEvent::Exit = event {
            if let Ok(mut rt) = runtime_for_exit.try_lock() {
                rt.stop_sync();
            } else {
                let rt_arc = runtime_for_exit.clone();
                tauri::async_runtime::block_on(async move {
                    if let Ok(mut rt) =
                        tokio::time::timeout(std::time::Duration::from_secs(2), rt_arc.lock()).await
                    {
                        rt.stop().await;
                    }
                });
            }
        }
    });
    if let Some(root) = smoke_root {
        let _ = std::fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_workspace_arg, resolve_startup_workspace};

    #[test]
    fn extract_workspace_arg_handles_flags_and_positional() {
        assert_eq!(
            extract_workspace_arg(&["keel-desktop".into(), "/tmp/demo".into()]),
            Some("/tmp/demo".into())
        );
        assert_eq!(
            extract_workspace_arg(&["keel-desktop".into(), "--root".into(), "/tmp/demo".into()]),
            Some("/tmp/demo".into())
        );
        assert_eq!(
            extract_workspace_arg(&["keel-desktop".into(), "--workspace=/tmp/demo".into()]),
            Some("/tmp/demo".into())
        );
        assert_eq!(
            extract_workspace_arg(&["keel-desktop".into(), "--smoke-test".into()]),
            None
        );
    }

    #[test]
    fn resolve_startup_workspace_rejects_root_and_accepts_temp_dir() {
        assert_eq!(
            resolve_startup_workspace(&["keel-desktop".into(), "/".into()]),
            None
        );
        let dir = std::env::temp_dir();
        let resolved =
            resolve_startup_workspace(&["keel-desktop".into(), dir.to_string_lossy().into_owned()]);
        assert!(resolved.is_some());
    }
}
