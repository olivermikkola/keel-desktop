#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod productivity;

use commands::AppState;
use keel_core::runtime::RuntimeManager;
use keel_core::sessions::{default_config_dir, default_data_dir, load_config};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{Listener, Manager, RunEvent};
use tokio::sync::Mutex;

fn main() {
    if std::env::args().any(|a| a == "--smoke-test") {
        println!(
            "keel-desktop {} smoke-test ok (core={}, models={})",
            env!("CARGO_PKG_VERSION"),
            keel_core::version(),
            keel_core::models::default_catalog().len()
        );
        return;
    }

    let ui_smoke = std::env::args().any(|a| a == "--ui-smoke-test");
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
    let cfg = load_config(&config_dir);
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
        turn_lock: Arc::new(Mutex::new(())),
        config_dir,
        data_dir,
        config: Arc::new(Mutex::new(cfg)),
        runtime: runtime_arc,
        approvals: Arc::new(Mutex::new(HashMap::new())),
        always_allowed: Arc::new(Mutex::new(HashMap::new())),
        active_abort: Arc::new(Mutex::new(None)),
        download_lock: Mutex::new(()),
        setup_pause: Mutex::new(None),
        active_cli_stop: Mutex::new(None),
    };

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
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
            productivity::pick_documents,
            productivity::save_project,
            productivity::get_project,
            productivity::list_library,
            productivity::delete_project,
            productivity::search_threads,
            productivity::update_thread,
            productivity::fork_thread,
            productivity::restore_thread,
            productivity::save_personal_context,
            productivity::export_thread,
            productivity::open_external_link,
            productivity::get_setup_plan,
            productivity::setup_local_ai,
            productivity::pause_local_setup,
            commands::connect_image_server,
            commands::list_image_jobs,
            commands::start_image_job,
            commands::cancel_image_job,
            commands::read_workspace_png,
            commands::pick_workspace_folder,
            commands::prepare_sample_workspace,
            commands::set_workspace_folder,
            commands::list_workspace_files_cmd,
            commands::read_workspace_file_raw,
            commands::create_session,
            commands::import_keel_session,
            commands::load_session_cmd,
            commands::delete_session_cmd,
            commands::start_model,
            commands::stop_runtime,
            commands::get_runtime_status,
            commands::set_external_endpoint,
            commands::download_catalog_model,
            commands::download_huggingface_gguf,
            commands::import_local_gguf,
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
            commands::save_customize_settings,
            commands::save_mcp_server,
            commands::remove_mcp_server,
            commands::refresh_mcp_servers,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Keel Desktop");

    app.run(move |_handle, event| {
        if let RunEvent::Exit = event {
            if let Ok(mut rt) = runtime_for_exit.try_lock() {
                rt.stop_sync();
            }
        }
    });
    if let Some(root) = smoke_root {
        let _ = std::fs::remove_dir_all(root);
    }
}
