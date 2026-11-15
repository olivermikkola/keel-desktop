#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
fn get_version() -> &'static str {
    keel_core::version()
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![get_version])
        .run(tauri::generate_context!())
        .expect("error running Keel Desktop");
}
