use crate::commands::AppState;
use keel_core::{
    documents::Document,
    projects::{self, Project},
    sessions::{self, ChatSession, SessionSummary},
};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn pick_documents(app: AppHandle) -> Result<Vec<Document>, String> {
    tokio::task::spawn_blocking(move || {
        let files = app
            .dialog()
            .file()
            .add_filter(
                "Documents & Images",
                &[
                    "pdf", "docx", "txt", "md", "csv", "json", "rs", "py", "js", "ts", "html",
                    "css", "sql", "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff",
                ],
            )
            .blocking_pick_files()
            .unwrap_or_default();
        if files.len() > 8 {
            return Err("Choose up to 8 documents at a time".to_string());
        }
        files
            .into_iter()
            .map(|file| {
                let path = file.into_path().map_err(|e| e.to_string())?;
                keel_core::documents::import_document(&path).map_err(|e| {
                    format!(
                        "{}: {e}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    )
                })
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn save_project(
    state: State<'_, AppState>,
    project: Project,
) -> Result<Vec<Project>, String> {
    if let Some(folder) = &project.workspace_path {
        keel_core::tools::validate_workspace_root(std::path::Path::new(folder))
            .map_err(|e| e.to_string())?;
    }
    projects::save_project(&state.data_dir, &project).map_err(|e| e.to_string())?;
    Ok(projects::list_projects(&state.data_dir))
}

#[tauri::command]
pub async fn delete_project(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<Project>, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before removing its project")?;
    projects::valid_id(&id).map_err(|e| e.to_string())?;
    let path = state.data_dir.join("projects").join(format!("{id}.json"));
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    // Threads and their files remain accessible outside the project.
    for summary in sessions::list_sessions(&state.data_dir) {
        if summary.project_id.as_deref() == Some(&id) {
            let mut session =
                sessions::load_session(&state.data_dir, &summary.id).map_err(|e| e.to_string())?;
            session.project_id = None;
            sessions::save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
        }
    }
    Ok(projects::list_projects(&state.data_dir))
}

#[tauri::command]
pub async fn search_threads(
    state: State<'_, AppState>,
    query: String,
    project_id: Option<String>,
) -> Result<Vec<SessionSummary>, String> {
    let query = query.trim().to_lowercase();
    Ok(sessions::list_sessions(&state.data_dir)
        .into_iter()
        .filter(|s| {
            if project_id
                .as_ref()
                .is_some_and(|id| s.project_id.as_ref() != Some(id))
            {
                return false;
            }
            query.is_empty()
                || s.title.to_lowercase().contains(&query)
                || sessions::load_session(&state.data_dir, &s.id).is_ok_and(|session| {
                    session.messages.iter().any(|m| {
                        m.content.to_lowercase().contains(&query)
                            || m.attachments
                                .iter()
                                .any(|a| a.name.to_lowercase().contains(&query))
                    })
                })
        })
        .take(100)
        .collect())
}

#[tauri::command]
pub async fn update_thread(
    state: State<'_, AppState>,
    id: String,
    title: String,
    pinned: bool,
    project_id: Option<String>,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish")?;
    if title.trim().is_empty() || title.chars().count() > 120 {
        return Err("Use a title of 1–120 characters".into());
    }
    if let Some(id) = &project_id {
        projects::load_project(&state.data_dir, id).map_err(|e| e.to_string())?;
    }
    let mut session = sessions::load_session(&state.data_dir, &id).map_err(|e| e.to_string())?;
    session.title = title.trim().to_string();
    session.pinned = pinned;
    session.project_id = project_id;
    sessions::save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn fork_thread(
    state: State<'_, AppState>,
    id: String,
    before_message_id: String,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish")?;
    let mut session = sessions::load_session(&state.data_dir, &id).map_err(|e| e.to_string())?;
    let index = session
        .messages
        .iter()
        .position(|m| m.id == before_message_id)
        .ok_or("Could not find that message")?;
    session.messages.truncate(index);
    session.id = uuid::Uuid::new_v4().to_string();
    session.title = format!(
        "{} (edited)",
        session.title.chars().take(100).collect::<String>()
    );
    session.pinned = false;
    session.keel_session_log = None;
    session.code_checkout = None;
    session.applied_code_patch = None;
    session.last_code_test = None;
    session.staged_edits.clear();
    for message in &mut session.messages {
        for tool in &mut message.tool_calls {
            tool.edit_id = None;
        }
    }
    sessions::save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn save_personal_context(state: State<'_, AppState>, text: String) -> Result<(), String> {
    if text.chars().count() > 4000 {
        return Err("Keep saved preferences under 4000 characters".into());
    }
    let mut cfg = state.config.lock().await;
    cfg.personal_context = text;
    sessions::save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_thread(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<String>, String> {
    let session = sessions::load_session(&state.data_dir, &id).map_err(|e| e.to_string())?;
    let text = keel_core::context::export_markdown(&session);
    tokio::task::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .set_file_name("keel-conversation.md")
            .add_filter("Markdown", &["md"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let path = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_setup_plan(
    state: State<'_, AppState>,
) -> Result<keel_core::setup::SetupPlan, String> {
    let cfg = state.config.lock().await;
    Ok(keel_core::setup::plan(
        &state.data_dir,
        cfg.llama_server_path.as_deref(),
    ))
}

#[tauri::command]
pub async fn setup_local_ai(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: Option<String>,
) -> Result<keel_core::runtime::RuntimeStatus, String> {
    let _download = state
        .download_lock
        .try_lock()
        .map_err(|_| "Another download is running")?;
    let cfg = state.config.lock().await.clone();
    let plan = keel_core::setup::plan(&state.data_dir, cfg.llama_server_path.as_deref());
    let model =
        keel_core::setup::resolve_model(&plan, model_id.as_deref()).map_err(|e| e.to_string())?;
    let (sender, mut pause) = tokio::sync::watch::channel(false);
    *state.setup_pause.lock().await = Some(sender);
    let runner = prepare_guided_downloads(&app, &state, &cfg, plan, &model, &mut pause).await;
    *state.setup_pause.lock().await = None;
    let runner = runner?;
    {
        let mut cfg = state.config.lock().await;
        cfg.llama_server_path = Some(runner.to_string_lossy().into_owned());
        sessions::save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    }
    drop(_download);
    let _ = app.emit("keel://setup-stage", "start");
    crate::commands::start_model(app, state, model.id).await
}

#[tauri::command]
pub async fn pause_local_setup(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state
        .setup_pause
        .lock()
        .await
        .as_ref()
        .is_some_and(|pause| pause.send(true).is_ok()))
}

async fn prepare_guided_downloads(
    app: &AppHandle,
    state: &AppState,
    cfg: &sessions::DesktopConfig,
    plan: keel_core::setup::SetupPlan,
    model: &keel_core::models::CatalogModel,
    pause: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<std::path::PathBuf, String> {
    use keel_core::models::{catalog_download_url, download_with_pause, list_models, models_dir};

    let runner = if plan.runtime_asset.is_some() {
        let _ = app.emit("keel://setup-stage", "runner");
        let _ = app.emit(
            "keel://setup-progress",
            "Downloading and verifying the model runner…",
        );
        keel_core::setup::install_runtime_with_pause(&state.data_dir, Some(pause), |progress| {
            let _ = app.emit("keel://download-progress", progress);
        })
        .await
        .map_err(|e| e.to_string())?
    } else if let Some(path) = plan.runtime_path {
        let path = std::path::PathBuf::from(path);
        keel_core::runtime::check_runner_starts(&path).map_err(|e| e.to_string())?;
        path
    } else {
        return Err("Automatic setup is not available for this platform. Connect an existing model server instead.".into());
    };
    let models = list_models(&state.data_dir, &cfg.custom_model_paths);
    if !models.iter().any(|m| m.id == model.id && m.installed) {
        let _ = app.emit("keel://setup-stage", "model");
        let _ = app.emit(
            "keel://setup-progress",
            "Downloading and verifying your recommended model…",
        );
        let destination = models_dir(&state.data_dir).join(&model.file_name);
        keel_core::setup::require_download_space(&destination, model.bytes, 0)
            .map_err(|e| e.to_string())?;
        download_with_pause(
            &catalog_download_url(model),
            &destination,
            model.sha256.as_deref(),
            &model.id,
            Some(pause),
            |progress| {
                let _ = app.emit("keel://download-progress", progress);
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(runner)
}

#[tauri::command]
pub async fn get_project(state: State<'_, AppState>, id: String) -> Result<Project, String> {
    projects::load_project(&state.data_dir, &id).map_err(|e| e.to_string())
}

#[derive(Clone, serde::Serialize)]
pub struct LibraryItem {
    pub id: String,
    pub name: String,
    pub path: String,
    pub workspace_path: String,
    pub project_id: Option<String>,
    pub kind: String,
    pub updated_at: u64,
}
#[tauri::command]
pub async fn list_library(state: State<'_, AppState>) -> Result<Vec<LibraryItem>, String> {
    let mut items = std::collections::HashMap::new();
    for summary in sessions::list_sessions(&state.data_dir) {
        let Ok(session) = sessions::load_session(&state.data_dir, &summary.id) else {
            continue;
        };
        let Some(workspace) = session.workspace_path else {
            continue;
        };
        for edit in session
            .staged_edits
            .iter()
            .filter(|edit| edit.status == keel_core::tools::StagedEditStatus::Applied)
        {
            let path = edit.new_path.as_ref().unwrap_or(&edit.path).clone();
            let key = format!("{workspace}:{path}");
            let kind = std::path::Path::new(&path)
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let item = LibraryItem {
                id: key.clone(),
                name: path.clone(),
                path,
                workspace_path: workspace.clone(),
                project_id: session.project_id.clone(),
                kind,
                updated_at: session.updated_at,
            };
            items.entry(key).or_insert(item);
        }
    }
    for job in state.images.list().await {
        if let Some(path) = job.artifact_path {
            let key = format!("{}:{path}", job.workspace_path);
            items.entry(key.clone()).or_insert(LibraryItem {
                id: key,
                name: path.clone(),
                path,
                workspace_path: job.workspace_path,
                project_id: None,
                kind: "png".into(),
                updated_at: job.created_at,
            });
        }
    }
    let mut result: Vec<_> = items.into_values().collect();
    result.retain(|item| {
        keel_core::tools::confine_path(std::path::Path::new(&item.workspace_path), &item.path)
            .is_ok_and(|p| p.is_file())
    });
    result.sort_by_key(|item| std::cmp::Reverse(item.updated_at));
    Ok(result)
}

#[tauri::command]
pub async fn restore_thread(state: State<'_, AppState>, id: String) -> Result<ChatSession, String> {
    projects::valid_id(&id).map_err(|e| e.to_string())?;
    let path = state.data_dir.join("trash").join(format!("{id}.json"));
    let session: ChatSession =
        serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    sessions::save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    std::fs::remove_file(path).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn import_document_from_bytes(
    name: String,
    base64_data: String,
) -> Result<Document, String> {
    tokio::task::spawn_blocking(move || {
        keel_core::documents::import_document_base64(&name, &base64_data)
            .map_err(|e| format!("{name}: {e}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn open_models_folder(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = keel_core::models::models_dir(&state.data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_external_link(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only web links can be opened from a response".into());
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}
