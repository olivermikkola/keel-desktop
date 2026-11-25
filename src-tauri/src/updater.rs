use crate::commands::AppState;
use keel_core::updates::{
    can_install_in_place_on_this_host, fetch_github_latest_release, is_newer_version,
    AppUpdateInfo, RELEASES_PAGE_URL,
};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::UpdaterExt;

#[derive(Debug, Clone, Serialize)]
pub struct AppUpdateProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub done: bool,
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<AppUpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let in_place = can_install_in_place_on_this_host();

    if let Ok(updater) = app.updater() {
        match updater.check().await {
            Ok(Some(update)) => {
                if is_newer_version(&current, &update.version) {
                    let release_url = format!(
                        "https://github.com/olivermikkola/keel-desktop/releases/tag/v{}",
                        update.version.trim_start_matches('v')
                    );
                    return Ok(AppUpdateInfo {
                        current_version: current,
                        available: true,
                        version: Some(update.version),
                        notes: update.body,
                        release_url,
                        can_install_in_place: in_place,
                    });
                }
                return Ok(AppUpdateInfo {
                    current_version: current,
                    available: false,
                    version: None,
                    notes: None,
                    release_url: RELEASES_PAGE_URL.to_string(),
                    can_install_in_place: in_place,
                });
            }
            Ok(None) => {
                return Ok(AppUpdateInfo {
                    current_version: current,
                    available: false,
                    version: None,
                    notes: None,
                    release_url: RELEASES_PAGE_URL.to_string(),
                    can_install_in_place: in_place,
                });
            }
            Err(_) => {
                // Fall back to GitHub's /releases/latest API when latest.json is missing
                // for this platform or release (e.g. .deb/.rpm installs or pre-updater tags).
            }
        }
    }

    fetch_github_latest_release(&current)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn install_app_update(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    if !can_install_in_place_on_this_host() {
        return Err(
            "Automatic in-place updates are supported on macOS, Windows, and Linux AppImage. Use Download installer to update this package."
                .to_string(),
        );
    }

    let updater = app
        .updater()
        .map_err(|e| format!("Updater could not start: {e}"))?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("Could not check for updates: {e}"))?
        .ok_or_else(|| "Keel Desktop is already up to date.".to_string())?;

    // Stop any local llama-server child before replacing/restarting the app.
    state.runtime.lock().await.stop().await;

    let app_for_progress = app.clone();
    let app_for_finish = app.clone();
    let downloaded = Arc::new(AtomicU64::new(0));
    let downloaded_for_finish = Arc::clone(&downloaded);

    update
        .download_and_install(
            move |chunk_len, total_bytes| {
                let next =
                    downloaded.fetch_add(chunk_len as u64, Ordering::Relaxed) + chunk_len as u64;
                let _ = app_for_progress.emit(
                    "keel://app-update-progress",
                    AppUpdateProgress {
                        downloaded_bytes: next,
                        total_bytes,
                        done: false,
                    },
                );
            },
            move || {
                let final_bytes = downloaded_for_finish.load(Ordering::Relaxed);
                let _ = app_for_finish.emit(
                    "keel://app-update-progress",
                    AppUpdateProgress {
                        downloaded_bytes: final_bytes,
                        total_bytes: Some(final_bytes),
                        done: true,
                    },
                );
            },
        )
        .await
        .map_err(|e| format!("Update installation failed: {e}"))?;

    app.restart();
}
