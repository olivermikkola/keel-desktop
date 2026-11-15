#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use commands::AppState;
use keel_core::runtime::RuntimeManager;
use keel_core::sessions::{default_config_dir, default_data_dir, load_config};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::RunEvent;
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

    let config_dir = default_config_dir();
    let data_dir = default_data_dir();
    let cfg = load_config(&config_dir);

    let mut runtime = RuntimeManager::default();
    if let Some(ref url) = cfg.external_base_url {
        runtime.set_external_url(url, cfg.external_model_name.clone());
    }
    let runtime_arc = Arc::new(Mutex::new(runtime));
    let runtime_for_exit = Arc::clone(&runtime_arc);

    let state = AppState {
        config_dir,
        data_dir,
        config: Arc::new(Mutex::new(cfg)),
        runtime: runtime_arc,
        approvals: Arc::new(Mutex::new(HashMap::new())),
        always_allowed: Arc::new(Mutex::new(HashSet::new())),
        active_abort: Arc::new(Mutex::new(None)),
    };

    let app = tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::get_bootstrap_state,
            commands::pick_workspace_folder,
            commands::set_workspace_folder,
            commands::list_workspace_files_cmd,
            commands::read_workspace_file_raw,
            commands::create_session,
            commands::load_session_cmd,
            commands::delete_session_cmd,
            commands::start_model,
            commands::stop_runtime,
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
}
