use keel_core::agent::{
    run_agent_turn, AgentEvent, ApprovalDecision, EndpointUnreachable, TurnParams,
};
use keel_core::apps::{AppPreset, AppPresetInfo};
use keel_core::harnesses::{
    code_execution_boundary, detect_harnesses, run_external_harness, run_keel_cli_turn,
    CodeExecutionBoundary, HarnessOption, KeelCliTurn,
};
use keel_core::mcp::{
    inspect_mcp_servers_with_trust, load_global_mcp_config, load_mcp_configs_with_trust,
    load_workspace_mcp_config, save_global_mcp_config, save_workspace_mcp_config, McpScope,
    McpServerConfig, McpServerStatus, McpToolInfo,
};
use keel_core::model_check::{self, ModelCheckReport};
use keel_core::models::{
    candidate_models_for_auto_start, catalog_download_url, delete_model_file, download_gguf,
    list_models, models_dir, parse_huggingface_gguf_url, CatalogModel,
};
use keel_core::runtime::{
    check_runner_starts, find_llama_server, ModelLaunch, RuntimeManager, RuntimeStatus,
};
use keel_core::sessions::{
    delete_session, list_sessions, load_session, save_config, save_session, ChatSession,
    DesktopConfig, SessionSummary, UiMessage,
};
use keel_core::tools::{
    apply_staged_edit, list_workspace_files, validate_workspace_root, StagedEditStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{oneshot, watch, Mutex};
use tokio::task::AbortHandle;

pub struct AppState {
    pub images: keel_core::images::ImageService,
    pub turn_lock: Arc<Mutex<()>>,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config: Arc<Mutex<DesktopConfig>>,
    pub runtime: Arc<Mutex<RuntimeManager>>,
    pub approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub always_allowed: Arc<Mutex<HashMap<String, HashSet<String>>>>,
    pub active_abort: Arc<Mutex<Option<AbortHandle>>>,
    pub download_lock: Mutex<()>,
    pub setup_pause: Mutex<Option<watch::Sender<bool>>>,
    pub active_cli_stop: Mutex<Option<watch::Sender<bool>>>,
    pub(crate) placement_cache: Mutex<Option<PlacementCache>>,
}

pub(crate) struct PlacementCache {
    run_where: String,
    graphics_share: String,
    model_bytes: u64,
    graphics_installed: bool,
    advice: keel_core::hardware::ComputerAdvice,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapState {
    pub version: String,
    pub config: DesktopConfig,
    pub models: Vec<CatalogModel>,
    pub runtime: RuntimeStatus,
    pub sessions: Vec<SessionSummary>,
    pub projects: Vec<keel_core::projects::ProjectInfo>,
    pub presets: Vec<AppPresetInfo>,
    pub harnesses: Vec<HarnessOption>,
    pub code_execution: CodeExecutionBoundary,
    pub mcp_servers: Vec<McpServerStatus>,
    pub ocr_available: bool,
    pub system_ram_gb: u64,
    pub computer: keel_core::hardware::ComputerAdvice,
    pub model_checks: std::collections::BTreeMap<String, ModelCheckReport>,
    pub offered_model_id: String,
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[tauri::command]
pub async fn get_bootstrap_state(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
) -> Result<BootstrapState, String> {
    let cfg = state.config.lock().await.clone();
    let models = list_models(&state.data_dir, &cfg.custom_model_paths);
    let runtime = state.runtime.lock().await.status();
    let sessions = list_sessions(&state.data_dir);
    let presets = AppPreset::all().iter().map(|p| p.info()).collect();
    let harnesses = detect_harnesses();
    let ws = workspace_path.as_deref().map(Path::new);
    let ws_trusted = ws.map(|p| cfg.is_mcp_workspace_trusted(p)).unwrap_or(false);
    let loaded_mcp = load_mcp_configs_with_trust(&state.config_dir, ws, ws_trusted);
    let mcp_servers = inspect_mcp_servers_with_trust(&loaded_mcp, ws).await;
    let selected_bytes = models
        .iter()
        .find(|model| model.id == cfg.selected_model_id)
        .map(|model| model.bytes)
        .unwrap_or(0);
    let mut computer = cached_advice(&state, selected_bytes).await;
    let model_checks = model_check::load_checks(&state.data_dir);
    let failed: Vec<String> = model_checks
        .iter()
        .filter(|(_, report)| !report.passed)
        .map(|(id, _)| id.clone())
        .collect();
    let offered_model_id = model_check::offered_model_id(&computer.recommended_model_id, &failed);
    computer.recommended_model_id = offered_model_id.clone();

    Ok(BootstrapState {
        version: keel_core::version().to_string(),
        config: cfg,
        models,
        runtime,
        sessions,
        projects: keel_core::projects::project_info(&state.data_dir),
        presets,
        harnesses,
        code_execution: code_execution_boundary(),
        mcp_servers,
        ocr_available: keel_core::documents::ocr_available(),
        system_ram_gb: keel_core::setup::system_ram_gb(),
        computer,
        model_checks,
        offered_model_id,
    })
}

#[tauri::command]
pub async fn pick_workspace_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Open a folder")
            .blocking_pick_folder()
            .map(|selection| {
                selection
                    .into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn prepare_sample_workspace(state: State<'_, AppState>) -> Result<String, String> {
    use std::io::Write;
    let folder = state.data_dir.join("Sample Work");
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    for (name, content) in [
        ("handoff.md", include_str!("../samples/handoff.md")),
        ("budget.md", include_str!("../samples/budget.md")),
        ("orders.csv", include_str!("../samples/orders.csv")),
    ] {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.join(name))
        {
            Ok(mut file) => file
                .write_all(content.as_bytes())
                .map_err(|error| error.to_string())?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(folder.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn set_workspace_folder(
    state: State<'_, AppState>,
    path: String,
    session_id: Option<String>,
) -> Result<String, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before opening a folder")?;
    let canon = validate_workspace_root(Path::new(&path)).map_err(|e| e.to_string())?;
    let canon_str = canon.to_string_lossy().to_string();
    if let Some(id) = session_id {
        let mut session = load_session(&state.data_dir, &id).map_err(|error| error.to_string())?;
        if session.workspace_path.as_deref() != Some(&canon_str) {
            if !session.messages.is_empty() || !session.staged_edits.is_empty() {
                return Err("Open a new thread to work in a different folder".to_string());
            }
            session.workspace_path = Some(canon_str.clone());
            session.updated_at = now_epoch();
            save_session(&state.data_dir, &session).map_err(|error| error.to_string())?;
        }
    }
    let mut cfg = state.config.lock().await;
    cfg.recent_workspaces.retain(|w| w != &canon_str);
    cfg.recent_workspaces.insert(0, canon_str.clone());
    cfg.recent_workspaces.truncate(8);
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(canon_str)
}

#[tauri::command]
pub async fn list_workspace_files_cmd(workspace_path: String) -> Result<Vec<String>, String> {
    let raw = list_workspace_files(Path::new(&workspace_path), None).map_err(|e| e.to_string())?;
    if raw == "(workspace is empty)" {
        return Ok(Vec::new());
    }
    Ok(raw
        .lines()
        .filter(|l| !l.starts_with("... (truncated"))
        .map(String::from)
        .collect())
}

#[tauri::command]
pub async fn read_workspace_file_raw(
    workspace_path: String,
    rel_path: String,
) -> Result<String, String> {
    let root = Path::new(&workspace_path);
    let target = keel_core::tools::confine_path(root, &rel_path).map_err(|e| e.to_string())?;
    let meta = std::fs::metadata(&target).map_err(|e| e.to_string())?;
    if meta.len() > 1024 * 1024 {
        return Err("File is larger than 1 MB preview limit".to_string());
    }
    let bytes = std::fs::read(&target).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[tauri::command]
pub async fn create_session(
    state: State<'_, AppState>,
    preset: AppPreset,
    workspace_path: Option<String>,
    project_id: Option<String>,
) -> Result<ChatSession, String> {
    if let Some(id) = &project_id {
        keel_core::projects::load_project(&state.data_dir, id).map_err(|e| e.to_string())?;
    }
    let session = ChatSession {
        id: uuid::Uuid::new_v4().to_string(),
        title: format!("New {} thread", preset.title()),
        project_id,
        pinned: false,
        app_preset: preset,
        workspace_path,
        keel_session_log: None,
        code_checkout: None,
        applied_code_patch: None,
        last_code_test: None,
        messages: Vec::new(),
        staged_edits: Vec::new(),
        updated_at: now_epoch(),
    };
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn import_keel_session(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_path: String,
) -> Result<Option<ChatSession>, String> {
    use tauri_plugin_dialog::DialogExt;
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish")?;
    let root =
        validate_workspace_root(Path::new(&workspace_path)).map_err(|error| error.to_string())?;
    let selected = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Open a Keel CLI session")
            .add_filter("Keel session logs", &["jsonl"])
            .blocking_pick_file()
    })
    .await
    .map_err(|error| error.to_string())?;
    let Some(file) = selected else {
        return Ok(None);
    };
    let path = file
        .into_path()
        .map_err(|error| error.to_string())?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let messages =
        keel_core::harnesses::read_keel_session(&path).map_err(|error| error.to_string())?;
    let title = messages
        .iter()
        .find(|message| message.role == "user")
        .map(|message| message.content.chars().take(48).collect())
        .unwrap_or_else(|| "Keel CLI session".to_string());
    let session = ChatSession {
        id: uuid::Uuid::new_v4().to_string(),
        title,
        project_id: None,
        pinned: false,
        app_preset: AppPreset::Code,
        workspace_path: Some(root.to_string_lossy().into_owned()),
        keel_session_log: Some(path),
        code_checkout: None,
        applied_code_patch: None,
        last_code_test: None,
        messages,
        staged_edits: Vec::new(),
        updated_at: now_epoch(),
    };
    save_session(&state.data_dir, &session).map_err(|error| error.to_string())?;
    Ok(Some(session))
}

#[tauri::command]
pub async fn load_session_cmd(
    state: State<'_, AppState>,
    id: String,
) -> Result<ChatSession, String> {
    load_session(&state.data_dir, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_session_cmd(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<SessionSummary>, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before changing this thread")?;
    delete_session(&state.data_dir, &id).map_err(|e| e.to_string())?;
    Ok(list_sessions(&state.data_dir))
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelFallbackNotice {
    pub from_id: String,
    pub from_name: String,
    pub to_id: String,
    pub to_name: String,
    pub reason: String,
}

/// Ensures Keel's built-in local AI engine is available, automatically downloading
/// and verifying the pinned runner when missing on a supported platform.
async fn resolve_or_install_engine(app: &AppHandle, state: &AppState) -> Result<PathBuf, String> {
    let explicit_path = state.config.lock().await.llama_server_path.clone();

    if let Some(installed) = keel_core::setup::installed_runtime(&state.data_dir) {
        match check_runner_starts(&installed) {
            Ok(()) => {
                let mut cfg = state.config.lock().await;
                let next = Some(installed.to_string_lossy().into_owned());
                if cfg.llama_server_path != next {
                    cfg.llama_server_path = next;
                    let _ = save_config(&state.config_dir, &cfg);
                }
                return Ok(installed);
            }
            Err(error) => {
                let msg = error.to_string();
                if msg.contains("needs a system library") {
                    return Err(msg);
                }
            }
        }
    }

    if let Some(ref p) = explicit_path {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            if let Ok(()) = check_runner_starts(&pb) {
                return Ok(pb);
            }
        }
    }

    if let Some(found) = find_llama_server(None) {
        if check_runner_starts(&found).is_ok() {
            let mut cfg = state.config.lock().await;
            cfg.llama_server_path = Some(found.to_string_lossy().into_owned());
            let _ = save_config(&state.config_dir, &cfg);
            return Ok(found);
        }
    }

    let plan = keel_core::setup::plan(&state.data_dir, explicit_path.as_deref());
    if plan.runtime_asset.is_some() {
        let _download = state.download_lock.lock().await;
        if let Some(installed) = keel_core::setup::installed_runtime(&state.data_dir) {
            if check_runner_starts(&installed).is_ok() {
                let mut cfg = state.config.lock().await;
                cfg.llama_server_path = Some(installed.to_string_lossy().into_owned());
                let _ = save_config(&state.config_dir, &cfg);
                return Ok(installed);
            }
        }
        let installed = keel_core::setup::install_runtime(&state.data_dir, |prog| {
            let _ = app.emit("keel://download-progress", prog);
        })
        .await
        .map_err(|e| e.to_string())?;
        let mut cfg = state.config.lock().await;
        cfg.llama_server_path = Some(installed.to_string_lossy().into_owned());
        let _ = save_config(&state.config_dir, &cfg);
        return Ok(installed);
    }

    Err("Keel's built-in AI engine isn't available on this computer yet. You can connect Ollama or LM Studio in Models.".to_string())
}

async fn cached_advice(state: &AppState, model_bytes: u64) -> keel_core::hardware::ComputerAdvice {
    let cfg = state.config.lock().await.clone();
    let graphics_installed =
        keel_core::setup::installed_graphics_runtime(&state.data_dir).is_some();
    {
        let cache = state.placement_cache.lock().await;
        if let Some(hit) = cache.as_ref() {
            if hit.run_where == cfg.run_where
                && hit.graphics_share == cfg.graphics_share
                && hit.model_bytes == model_bytes
                && hit.graphics_installed == graphics_installed
            {
                return hit.advice.clone();
            }
        }
    }
    let devices = keel_core::setup::installed_graphics_runtime(&state.data_dir)
        .map(|bin| keel_core::hardware::probe_devices(&bin))
        .unwrap_or_default();
    let advice = keel_core::hardware::advise(
        &keel_core::hardware::detect_machine(),
        &cfg.run_where,
        &cfg.graphics_share,
        model_bytes,
        &devices,
    );
    *state.placement_cache.lock().await = Some(PlacementCache {
        run_where: cfg.run_where,
        graphics_share: cfg.graphics_share,
        model_bytes,
        graphics_installed,
        advice: advice.clone(),
    });
    advice
}

struct PreparedEngine {
    path: PathBuf,
    devices: Vec<keel_core::hardware::GraphicsDevice>,
}

/// Picks the graphics build when that is the fast path, and the processor
/// build otherwise. A failed graphics install stays on the processor.
async fn prepare_launch_engine(
    app: &AppHandle,
    state: &AppState,
) -> Result<PreparedEngine, String> {
    let cfg = state.config.lock().await.clone();
    let machine = keel_core::hardware::detect_machine();
    let already = keel_core::setup::installed_graphics_runtime(&state.data_dir).is_some();
    let want = keel_core::hardware::should_prepare_graphics(
        &cfg.run_where,
        machine.graphics_chip(),
        already,
    );
    let path = if want {
        match prepare_graphics_engine(app, state).await {
            Ok(path) => path,
            Err(_) => resolve_or_install_engine(app, state).await?,
        }
    } else {
        resolve_or_install_engine(app, state).await?
    };
    let devices = keel_core::hardware::probe_devices(&path);
    Ok(PreparedEngine { path, devices })
}

async fn prepare_graphics_engine(app: &AppHandle, state: &AppState) -> Result<PathBuf, String> {
    if let Some(path) = keel_core::setup::installed_graphics_runtime(&state.data_dir) {
        if check_runner_starts(&path).is_ok() {
            return Ok(path);
        }
    }
    let explicit = state.config.lock().await.llama_server_path.clone();
    if let Some(path) = keel_core::hardware::graphics_binary(explicit.as_deref()) {
        if check_runner_starts(&path).is_ok() {
            return Ok(path);
        }
    }
    if keel_core::setup::graphics_runtime_asset().is_none() {
        return Err("The graphics chip can't be used on this computer yet.".into());
    }
    let _download = state.download_lock.lock().await;
    if let Some(path) = keel_core::setup::installed_graphics_runtime(&state.data_dir) {
        if check_runner_starts(&path).is_ok() {
            return Ok(path);
        }
    }
    keel_core::setup::install_graphics_runtime(&state.data_dir, |prog| {
        let _ = app.emit("keel://download-progress", prog);
    })
    .await
    .map_err(|error| error.to_string())
}

fn launch_for<'a>(
    model_id: &'a str,
    model_path: &'a Path,
    context_size: u32,
    bin: &'a str,
    advice: &'a keel_core::hardware::ComputerAdvice,
    cfg: &'a DesktopConfig,
) -> ModelLaunch<'a> {
    ModelLaunch {
        model_id,
        model_path,
        context_size,
        custom_bin: Some(bin),
        offload_args: &advice.args,
        run_where: &cfg.run_where,
        graphics_share: &cfg.graphics_share,
        settings_bin: cfg.llama_server_path.as_deref(),
    }
}

/// Keeps the managed runner when the model and the graphics-or-processor choice
/// have not changed. The next message does not pay for another load.
async fn loaded_model_can_stay(state: &AppState) -> Option<RuntimeStatus> {
    let cfg = state.config.lock().await.clone();
    if cfg.external_base_url.is_some() {
        return None;
    }
    let models = list_models(&state.data_dir, &cfg.custom_model_paths);
    let model = models
        .iter()
        .find(|m| m.id == cfg.selected_model_id && m.installed && !m.damaged)?;
    let path = model.local_path.as_deref()?;
    let ctx = keel_core::setup::effective_context_size(
        model.context_size,
        cfg.context_window,
        keel_core::setup::system_ram_gb(),
    );
    let mut rt = state.runtime.lock().await;
    if rt.still_loaded(
        &model.id,
        path,
        ctx,
        &cfg.run_where,
        &cfg.graphics_share,
        cfg.llama_server_path.as_deref(),
    ) {
        Some(rt.status())
    } else {
        None
    }
}

async fn auto_start_local_model(
    app: &AppHandle,
    state: &AppState,
) -> Result<RuntimeStatus, String> {
    {
        let mut rt = state.runtime.lock().await;
        let current = rt.status();
        if current.state == "external" {
            return Ok(current);
        }
    }
    if let Some(status) = loaded_model_can_stay(state).await {
        return Ok(status);
    }
    let cfg_snapshot = state.config.lock().await.clone();
    let models = list_models(&state.data_dir, &cfg_snapshot.custom_model_paths);
    let failed = model_check::failed_model_ids(&state.data_dir);
    let preferred_id = if failed
        .iter()
        .any(|id| id == &cfg_snapshot.selected_model_id)
    {
        model_check::offered_model_id(&cfg_snapshot.selected_model_id, &failed)
    } else {
        cfg_snapshot.selected_model_id.clone()
    };
    let preferred = models.iter().find(|m| m.id == preferred_id).cloned();
    let mut pool: Vec<CatalogModel> = models
        .iter()
        .filter(|model| !failed.iter().any(|id| id == &model.id))
        .cloned()
        .collect();
    let pool_can_start = pool
        .iter()
        .any(|model| model.installed && !model.damaged && model.local_path.is_some());
    if !pool_can_start {
        pool.clone_from(&models);
    }
    let candidates = candidate_models_for_auto_start(&pool, &preferred_id);
    if candidates.is_empty() {
        if models.iter().any(|m| m.damaged) {
            return Err("Your downloaded model looks damaged. Open Models and click Repair to download a clean copy.".to_string());
        }
        return Err("Your AI isn't set up yet. Click Set up my AI to get started.".to_string());
    }

    let prepared = prepare_launch_engine(app, state).await?;
    let engine_str = prepared.path.to_string_lossy().into_owned();
    let mut last_error: Option<String> = None;

    for cand in candidates {
        let Some(path_str) = cand.local_path.as_deref() else {
            continue;
        };
        let mut cfg_now = state.config.lock().await;
        let mut rt = state.runtime.lock().await;
        if rt.status().state == "external" {
            return Ok(rt.status());
        }
        let ram_gb = keel_core::setup::system_ram_gb();
        let ctx_size = keel_core::setup::effective_context_size(
            cand.context_size,
            cfg_now.context_window,
            ram_gb,
        );
        let advice = keel_core::hardware::advise(
            &keel_core::hardware::detect_machine(),
            &cfg_now.run_where,
            &cfg_now.graphics_share,
            cand.bytes,
            &prepared.devices,
        );
        let started = rt
            .start_model(launch_for(
                &cand.id,
                Path::new(path_str),
                ctx_size,
                &engine_str,
                &advice,
                &cfg_now,
            ))
            .await;
        match started {
            Ok(status) => {
                if cand.id != preferred_id {
                    if let Some(prev) = preferred
                        .as_ref()
                        .filter(|p| p.damaged || p.installed || p.local_path.is_some())
                    {
                        let reason = prev
                            .damage_reason
                            .clone()
                            .or_else(|| last_error.clone())
                            .unwrap_or_else(|| "couldn't start on this computer".to_string());
                        let _ = app.emit(
                            "keel://model-fallback",
                            ModelFallbackNotice {
                                from_id: prev.id.clone(),
                                from_name: prev.name.clone(),
                                to_id: cand.id.clone(),
                                to_name: cand.name.clone(),
                                reason,
                            },
                        );
                    }
                }
                cfg_now.selected_model_id = cand.id;
                cfg_now.external_base_url = None;
                cfg_now.external_model_name = None;
                let _ = keel_core::remote::clear_external_api_key(&state.data_dir);
                save_config(&state.config_dir, &cfg_now).map_err(|e| e.to_string())?;
                return Ok(status);
            }
            Err(error) => {
                let msg = error.to_string();
                if msg.contains("needs a system library") {
                    return Err(msg);
                }
                last_error = Some(msg);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        "None of your downloaded models could start on this computer. Try a lighter model in Models."
            .to_string()
    }))
}

#[tauri::command]
pub async fn start_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<RuntimeStatus, String> {
    let cfg_snapshot = state.config.lock().await.clone();
    let models = list_models(&state.data_dir, &cfg_snapshot.custom_model_paths);
    let model = models
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("Unknown model: {model_id}"))?;
    if model.damaged {
        return Err(format!(
            "{} looks damaged. Click Repair in Models to download a clean copy.",
            model.name
        ));
    }
    let path_str = model
        .local_path
        .ok_or_else(|| format!("{} isn't downloaded yet.", model.name))?;

    let prepared = prepare_launch_engine(&app, &state).await?;
    let engine_str = prepared.path.to_string_lossy().into_owned();
    let advice = keel_core::hardware::advise(
        &keel_core::hardware::detect_machine(),
        &cfg_snapshot.run_where,
        &cfg_snapshot.graphics_share,
        model.bytes,
        &prepared.devices,
    );

    let ram_gb = keel_core::setup::system_ram_gb();
    let ctx_size = keel_core::setup::effective_context_size(
        model.context_size,
        cfg_snapshot.context_window,
        ram_gb,
    );
    let mut cfg = state.config.lock().await;
    let mut rt = state.runtime.lock().await;
    let status = rt
        .start_model(launch_for(
            &model.id,
            Path::new(&path_str),
            ctx_size,
            &engine_str,
            &advice,
            &cfg_snapshot,
        ))
        .await
        .map_err(|e| e.to_string())?;
    cfg.selected_model_id = model.id;
    cfg.external_base_url = None;
    cfg.external_model_name = None;
    let _ = keel_core::remote::clear_external_api_key(&state.data_dir);
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(status)
}

#[tauri::command]
pub async fn auto_start_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<RuntimeStatus, String> {
    auto_start_local_model(&app, &state).await
}

#[tauri::command]
pub async fn check_current_model(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelCheckReport, String> {
    let _turn = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish.".to_string())?;
    let mut status = state.runtime.lock().await.status();
    if !matches!(status.state.as_str(), "running" | "external") {
        status = auto_start_local_model(&app, &state).await?;
    }
    let model_id = status
        .active_model_id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| {
            "Your AI isn't ready yet. Start it in Models, then try the check again.".to_string()
        })?;
    // External endpoints may require the stored Bearer key; local keep-alive does not.
    let api_key = if status.state == "external" {
        keel_core::remote::load_external_api_key(&state.data_dir, &status.base_url)
    } else {
        None
    };
    let report =
        model_check::run_model_check_with_auth(&status.base_url, &model_id, api_key.as_deref())
            .await
            .map_err(|_| {
                "This model didn't answer the check. Make sure your AI is running and try again."
                    .to_string()
            })?;
    if report
        .items
        .iter()
        .all(|item| item.detail == "This question didn't finish.")
    {
        return Err(
            "This model didn't answer the check. Make sure your AI is running and try again."
                .into(),
        );
    }
    model_check::save_report(&state.data_dir, &report)
        .map_err(|_| "The check finished, but Keel couldn't save it.".to_string())?;
    Ok(report)
}

#[tauri::command]
pub async fn stop_runtime(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    // Use the same lock order as start_model.
    let mut cfg = state.config.lock().await;
    let mut rt = state.runtime.lock().await;
    rt.stop().await;
    cfg.external_base_url = None;
    cfg.external_model_name = None;
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    keel_core::remote::clear_external_api_key(&state.data_dir).map_err(|e| e.to_string())?;
    Ok(rt.status())
}

#[tauri::command]
pub async fn set_external_endpoint(
    state: State<'_, AppState>,
    base_url: String,
    model_name: Option<String>,
    confirm_remote: Option<bool>,
    api_key: Option<String>,
) -> Result<RuntimeStatus, String> {
    let base_url = base_url.trim().trim_end_matches('/');
    keel_core::remote::validate_external_connect(base_url, confirm_remote.unwrap_or(false))
        .map_err(|e| e.to_string())?;
    let mut cfg = state.config.lock().await;
    let key_for_request = match api_key.as_deref().map(str::trim) {
        Some("") => None,
        Some(key) => Some(key.to_string()),
        None => keel_core::remote::load_external_api_key(&state.data_dir, base_url),
    };
    if key_for_request.is_some() {
        keel_core::remote::validate_external_key_transport(base_url).map_err(|e| e.to_string())?;
    }
    let probed =
        keel_core::runtime::probe_openai_models_with_auth(base_url, key_for_request.as_deref())
            .await
            .map_err(|_| {
                if keel_core::runtime::is_loopback_url(base_url) {
                    "Couldn't connect. Make sure Ollama or LM Studio is open and running, then try again."
                        .to_string()
                } else {
                    "Couldn't connect to that server. Check the address, API key, and that you confirmed a remote server."
                        .to_string()
                }
            })?;
    let chosen = model_name
        .filter(|s| !s.trim().is_empty())
        .or_else(|| probed.into_iter().next())
        .unwrap_or_else(|| "default".to_string());

    let mut rt = state.runtime.lock().await;
    let mut updated = cfg.clone();
    updated.external_base_url = Some(base_url.to_string());
    updated.external_model_name = Some(chosen.clone());
    let previous_url = cfg.external_base_url.as_deref().unwrap_or_default();
    let previous_key = keel_core::remote::load_external_api_key(&state.data_dir, previous_url);
    keel_core::remote::save_external_api_key(&state.data_dir, base_url, key_for_request.as_deref())
        .map_err(|e| e.to_string())?;
    if let Err(error) = save_config(&state.config_dir, &updated) {
        keel_core::remote::save_external_api_key(
            &state.data_dir,
            previous_url,
            previous_key.as_deref(),
        )
        .map_err(|restore| format!("{error}. Could not restore the previous API key: {restore}"))?;
        return Err(error.to_string());
    }
    *cfg = updated;
    rt.set_external_url(base_url, Some(chosen));
    let status = rt.status();
    Ok(status)
}

#[tauri::command]
pub async fn clear_external_endpoint(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    stop_runtime(state).await
}

#[tauri::command]
pub async fn download_catalog_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<Vec<CatalogModel>, String> {
    let _download = state
        .download_lock
        .try_lock()
        .map_err(|_| "Another download is running")?;
    let cfg = state.config.lock().await.clone();
    let models = list_models(&state.data_dir, &cfg.custom_model_paths);
    let model = models
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("Unknown model: {model_id}"))?;
    let url = catalog_download_url(&model);
    let dest = models_dir(&state.data_dir).join(&model.file_name);
    if model.damaged && dest.is_file() {
        let _ = std::fs::remove_file(&dest);
    }

    download_gguf(&url, &dest, model.sha256.as_deref(), &model.id, |prog| {
        let _ = app.emit("keel://download-progress", prog);
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(list_models(&state.data_dir, &cfg.custom_model_paths))
}

#[tauri::command]
pub async fn delete_model(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<Vec<CatalogModel>, String> {
    let _download = state
        .download_lock
        .try_lock()
        .map_err(|_| "Wait for the current download to finish before removing a model")?;
    let _turn = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before removing a model")?;
    let mut cfg = state.config.lock().await;
    let mut rt = state.runtime.lock().await;
    if rt.active_model_id.as_deref() == Some(model_id.as_str()) && rt.is_managed() {
        rt.stop().await;
    }
    delete_model_file(&state.data_dir, &model_id, &mut cfg.custom_model_paths)
        .map_err(|e| e.to_string())?;
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(list_models(&state.data_dir, &cfg.custom_model_paths))
}

#[tauri::command]
pub async fn pick_model_file(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose a local AI model file")
            .add_filter("Local AI models", &["gguf"])
            .blocking_pick_file()
            .map(|selection| {
                selection
                    .into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn download_huggingface_gguf(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Result<Vec<CatalogModel>, String> {
    let _download = state
        .download_lock
        .try_lock()
        .map_err(|_| "Another download is running")?;
    let (_repo, _rev, file_name, resolve_url) =
        parse_huggingface_gguf_url(&url).map_err(|e| e.to_string())?;
    keel_core::models::ensure_non_catalog_filename(&file_name).map_err(|e| e.to_string())?;
    let dest = models_dir(&state.data_dir).join(&file_name);
    let id = format!("local:{file_name}");

    download_gguf(&resolve_url, &dest, None, &id, |prog| {
        let _ = app.emit("keel://download-progress", prog);
    })
    .await
    .map_err(|e| e.to_string())?;

    let cfg = state.config.lock().await;
    Ok(list_models(&state.data_dir, &cfg.custom_model_paths))
}

#[tauri::command]
pub async fn import_local_gguf(
    state: State<'_, AppState>,
    path: String,
) -> Result<Vec<CatalogModel>, String> {
    let pb = PathBuf::from(path.trim());
    if !pb.is_file() {
        return Err("That file could not be found.".to_string());
    }
    keel_core::models::validate_gguf_file(&pb, None)?;
    let file_name = pb
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("custom.gguf")
        .to_string();
    let id = format!("imported:{file_name}");

    let mut cfg = state.config.lock().await;
    cfg.custom_model_paths
        .insert(id, pb.to_string_lossy().to_string());
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(list_models(&state.data_dir, &cfg.custom_model_paths))
}

#[derive(Debug, Deserialize)]
pub struct SendMessageInput {
    pub session_id: String,
    pub prompt: String,
    pub enable_thinking: bool,
    #[serde(default)]
    pub attachments: Vec<keel_core::documents::Document>,
    #[serde(default)]
    pub selected_file: Option<String>,
    pub workspace_path: Option<String>,
}

#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SendMessageInput,
) -> Result<ChatSession, String> {
    let _turn_guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "A reply is already running. Stop it or wait for it to finish.")?;
    let mut session =
        load_session(&state.data_dir, &input.session_id).map_err(|e| e.to_string())?;
    if let Some(path) = input
        .workspace_path
        .as_deref()
        .filter(|_| session.app_preset != AppPreset::Chat)
    {
        let root = validate_workspace_root(Path::new(path)).map_err(|error| error.to_string())?;
        let root = root.to_string_lossy();
        if session
            .workspace_path
            .as_deref()
            .is_some_and(|old| old != root)
            && (!session.messages.is_empty() || !session.staged_edits.is_empty())
        {
            return Err("Open a new thread to work in a different folder".into());
        }
        session.workspace_path = Some(root.into_owned());
    }

    if input.attachments.len() > 8
        || input
            .attachments
            .iter()
            .any(|d| d.text.chars().count() > keel_core::documents::MAX_TEXT_CHARS)
    {
        return Err("Attach up to 8 documents per message".into());
    }
    if input.prompt.chars().count() > 20000 {
        return Err("This message is too long. Attach the text as a document instead, or keep the message under 20,000 characters.".into());
    }
    if input.prompt.trim().is_empty() {
        return Err("Write a message first".to_string());
    }
    let cfg = {
        let mut cfg = state.config.lock().await;
        if keel_core::context::remember_requested(&mut cfg, &input.prompt)
            .map_err(|e| e.to_string())?
        {
            save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
            let _ = app.emit("keel://preference-saved", cfg.personal_context.clone());
        }
        cfg.clone()
    };
    let mut rt_status = state.runtime.lock().await.status();
    let use_external_harness = cfg.harness_mode != "keel" && session.app_preset == AppPreset::Code;
    let use_keel_cli = use_external_harness && cfg.harness_mode == "keel-cli";
    if session.app_preset != AppPreset::Chat && session.workspace_path.is_none() {
        return Err("Choose a workspace folder for Work or Code first".to_string());
    }
    if (!use_external_harness || use_keel_cli)
        && !matches!(rt_status.state.as_str(), "running" | "external")
    {
        if cfg.external_base_url.is_none() {
            rt_status = auto_start_local_model(&app, &state).await?;
        } else {
            return Err(
                "Couldn't reach your connected AI app. Make sure it is running, or switch back to Keel's built-in AI in Models."
                    .to_string(),
            );
        }
    }
    let cli_log = if use_keel_cli {
        if !input.attachments.is_empty() {
            return Err("Open the attachment's folder and refer to its filename, or use Work for attachments.".into());
        }
        if !session.messages.is_empty() && session.keel_session_log.is_none() {
            return Err("Start a new Code thread to use the Keel CLI. This thread belongs to the previous engine.".into());
        }
        let log = session.keel_session_log.get_or_insert_with(|| {
            state
                .data_dir
                .join("cli-sessions")
                .join(format!("{}.jsonl", session.id))
        });
        Some(log.clone())
    } else {
        None
    };

    if session.messages.is_empty() && !input.prompt.trim().is_empty() {
        let snippet: String = input.prompt.trim().chars().take(48).collect();
        session.title = snippet;
    }

    session.messages.push(UiMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: "user".to_string(),
        content: input.prompt.clone(),
        thinking: None,
        tool_calls: Vec::new(),
        attachments: input.attachments.clone(),
        status: "complete".into(),
    });
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;

    if let Some(log) = cli_log {
        return stream_keel_cli(&app, &state, session, &input.prompt, &log, &rt_status).await;
    }

    // External harnesses are explicitly a Code capability; Chat never runs them.
    if use_external_harness {
        let ws = session.active_workspace().ok_or_else(|| {
            "Select a workspace folder before using an external CLI harness".to_string()
        })?;
        let harness_mode = cfg.harness_mode.clone();
        let prompt_owned = input.prompt.clone();
        let base_url = rt_status.base_url.clone();
        let model_id = rt_status.active_model_id.clone();
        let started = Instant::now();
        let handle = tokio::spawn(async move {
            run_external_harness(
                &harness_mode,
                &ws,
                &prompt_owned,
                Some(&base_url),
                model_id.as_deref(),
            )
            .await
        });
        {
            let mut abort_slot = state.active_abort.lock().await;
            *abort_slot = Some(handle.abort_handle());
        }
        let run_res = handle.await;
        {
            let mut abort_slot = state.active_abort.lock().await;
            *abort_slot = None;
        }
        let text = match run_res {
            Ok(Ok(text)) => text,
            Ok(Err(e)) => return Err(e.to_string()),
            Err(_) => {
                return Err("Stopped external harness run.".to_string());
            }
        };

        session.messages.push(UiMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: "assistant".to_string(),
            content: text.clone(),
            thinking: None,
            tool_calls: Vec::new(),
            attachments: Vec::new(),
            status: "complete".into(),
        });
        session.updated_at = now_epoch();
        save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
        let _ = app.emit(
            "keel://agent-event",
            AgentEvent::TurnFinished {
                content: text,
                thinking: None,
                prompt_tokens: 0,
                completion_tokens: 0,
                tokens_per_sec: 0.0,
                elapsed_secs: started.elapsed().as_secs_f64(),
            },
        );
        return Ok(session);
    }

    let mut base_url = rt_status.base_url.clone();
    let mut model_name = rt_status
        .active_model_id
        .clone()
        .unwrap_or_else(|| cfg.selected_model_id.clone());

    let ram_gb = keel_core::setup::system_ram_gb();
    let catalog = list_models(&state.data_dir, &cfg.custom_model_paths);
    let configured_ctx = catalog
        .iter()
        .find(|m| m.id == model_name)
        .map(|m| {
            keel_core::setup::effective_context_size(m.context_size, cfg.context_window, ram_gb)
                as usize
        })
        .unwrap_or_else(|| {
            keel_core::setup::effective_context_size(8192, cfg.context_window, ram_gb) as usize
        });
    // When running our managed llama-server, always budget against the actual -c window it booted with.
    let context_size = if rt_status.state == "running" {
        rt_status
            .active_context_size
            .map(|c| c as usize)
            .unwrap_or(configured_ctx)
    } else {
        configured_ctx
    };

    let ws_path = if session.app_preset == AppPreset::Chat {
        None
    } else {
        session.active_workspace()
    };
    let ws_trusted = session
        .workspace_path
        .as_deref()
        .map(Path::new)
        .map(|p| cfg.is_mcp_workspace_trusted(p))
        .unwrap_or(false)
        || ws_path
            .as_deref()
            .map(|p| cfg.is_mcp_workspace_trusted(p))
            .unwrap_or(false);
    let loaded_mcp = load_mcp_configs_with_trust(&state.config_dir, ws_path.as_deref(), ws_trusted);
    let mcp_cfg = loaded_mcp.active.clone();
    let mcp_statuses = if session.app_preset == AppPreset::Chat {
        Vec::new()
    } else {
        inspect_mcp_servers_with_trust(&loaded_mcp, ws_path.as_deref()).await
    };
    let mcp_tools: Vec<McpToolInfo> = mcp_statuses.into_iter().flat_map(|s| s.tools).collect();

    let project = session
        .project_id
        .as_deref()
        .map(|id| keel_core::projects::load_project(&state.data_dir, id))
        .transpose()
        .map_err(|e| e.to_string())?;
    let vision = session.app_preset == AppPreset::Chat;
    let history = keel_core::context::history_for_turn_with_vision(
        &session,
        &cfg,
        project.as_ref(),
        context_size,
        vision,
    );

    let allowance_key = format!(
        "{}:{}",
        session.id,
        session.workspace_path.as_deref().unwrap_or("")
    );
    let mut allowed = state
        .always_allowed
        .lock()
        .await
        .get(&allowance_key)
        .cloned()
        .unwrap_or_default();

    let mut custom_instructions = keel_core::context::instructions(&cfg, project.as_ref());
    if let (Some(file), Some(root)) = (&input.selected_file, &ws_path) {
        keel_core::tools::confine_path(root, file).map_err(|e| e.to_string())?;
        custom_instructions.push_str(&format!("\n\nThe user currently has `{file}` selected in the workspace inspector. Use it as context when the request refers to this file, and inspect it before editing."));
    }

    let mut final_allowed = allowed.clone();
    let mut final_session = session.clone();

    for attempt in 0..2 {
        let external_api_key = if rt_status.state == "external" {
            keel_core::remote::load_external_api_key(&state.data_dir, &base_url)
        } else {
            None
        };
        let params = TurnParams {
            base_url: base_url.clone(),
            model_name: model_name.clone(),
            context_size,
            app_preset: session.app_preset,
            custom_instructions: custom_instructions.clone(),
            workspace_root: ws_path.clone(),
            auto_apply_edits: cfg.auto_apply_edits,
            enable_thinking: input.enable_thinking,
            history: history.clone(),
            mcp_config: mcp_cfg.clone(),
            mcp_tools: mcp_tools.clone(),
            max_steps: 12,
            api_key: external_api_key,
        };

        let mut recorder =
            keel_core::recording::TurnRecorder::new(&state.data_dir, session.clone())
                .map_err(|e| e.to_string())?;
        let mut recording_error: Option<String> = None;
        let mut emitted_any = false;
        let mut attempt_allowed = allowed.clone();
        let approvals_map = Arc::clone(&state.approvals);
        let app_for_emit = app.clone();
        let app_for_approval = app.clone();

        let handle = tokio::spawn(async move {
            let res = run_agent_turn(
                params,
                &mut attempt_allowed,
                |ev| {
                    if matches!(
                        ev,
                        AgentEvent::ContentDelta { .. }
                            | AgentEvent::ThinkingDelta { .. }
                            | AgentEvent::ToolCallStart { .. }
                            | AgentEvent::StagedEditAdded { .. }
                    ) {
                        emitted_any = true;
                    }
                    if let Err(error) = recorder.record(&ev) {
                        recording_error = Some(error.to_string());
                    }
                    if !matches!(ev, AgentEvent::ApprovalRequired { .. }) {
                        let _ = app_for_emit.emit("keel://agent-event", ev);
                    }
                },
                |req_id, tool, summary, arguments| {
                    let app = app_for_approval.clone();
                    let map = Arc::clone(&approvals_map);
                    async move {
                        let (tx, rx) = oneshot::channel();
                        map.lock().await.insert(req_id.clone(), tx);
                        let _ = app.emit(
                            "keel://agent-event",
                            AgentEvent::ApprovalRequired {
                                id: req_id,
                                tool,
                                summary,
                                arguments,
                            },
                        );
                        rx.await.unwrap_or(ApprovalDecision::Deny)
                    }
                },
            )
            .await;
            (res, emitted_any, recording_error, attempt_allowed, recorder)
        });

        {
            let mut abort_slot = state.active_abort.lock().await;
            *abort_slot = Some(handle.abort_handle());
        }

        let join_res = handle.await;
        {
            let mut abort_slot = state.active_abort.lock().await;
            *abort_slot = None;
        }

        let (turn_res, emitted_any, recording_error, updated_allowed, mut recorder) = match join_res
        {
            Ok(result) => result,
            Err(error) => {
                let mut restored =
                    load_session(&state.data_dir, &input.session_id).map_err(|e| e.to_string())?;
                if let Some(message) = restored.messages.last_mut() {
                    message.status = "failed".into();
                    message.content.push_str(if error.is_cancelled() {
                        "\n\nReply stopped. Your partial answer and proposed edits have been kept."
                    } else {
                        "\n\nThis reply was interrupted. You can try again."
                    });
                }
                save_session(&state.data_dir, &restored).map_err(|e| e.to_string())?;
                state.approvals.lock().await.clear();
                return Ok(restored);
            }
        };

        let should_recover = attempt == 0
            && !emitted_any
            && cfg.external_base_url.is_none()
            && turn_res
                .as_ref()
                .err()
                .is_some_and(|e| e.downcast_ref::<EndpointUnreachable>().is_some());

        if should_recover {
            state.runtime.lock().await.stop().await;
            if let Ok(recovered) = auto_start_local_model(&app, &state).await {
                base_url = recovered.base_url;
                model_name = recovered
                    .active_model_id
                    .unwrap_or_else(|| cfg.selected_model_id.clone());
                allowed = updated_allowed;
                continue;
            }
        }

        if let Err(error) = &turn_res {
            let _ = recorder.fail(&error.to_string());
        }
        if let Some(error) = recording_error {
            let _ = recorder.fail(&format!("Could not save part of this reply: {error}"));
        }
        final_allowed = updated_allowed;
        final_session = recorder.session;
        break;
    }

    state
        .always_allowed
        .lock()
        .await
        .insert(allowance_key, final_allowed);
    state.approvals.lock().await.clear();
    session = final_session;
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;

    Ok(session)
}

async fn stream_keel_cli(
    app: &AppHandle,
    state: &AppState,
    session: ChatSession,
    prompt: &str,
    log: &Path,
    runtime: &RuntimeStatus,
) -> Result<ChatSession, String> {
    let root = session
        .active_workspace()
        .ok_or("Choose a Code folder first")?;
    let model = runtime
        .active_model_id
        .as_deref()
        .ok_or("Load a model for Code first")?;
    let mut recorder = keel_core::recording::TurnRecorder::new(&state.data_dir, session)
        .map_err(|error| error.to_string())?;
    let (stop, receiver) = watch::channel(false);
    *state.active_cli_stop.lock().await = Some(stop);
    let mut recording_error = None;
    let started = Instant::now();
    let result = run_keel_cli_turn(
        KeelCliTurn {
            root: &root,
            log,
            prompt,
            base_url: &runtime.base_url,
            model,
        },
        receiver,
        |event| {
            if let Err(error) = recorder.record(&event) {
                recording_error = Some(error);
            }
            let _ = app.emit("keel://agent-event", event);
        },
        |id, tool, summary, arguments| async move {
            let (sender, receiver) = oneshot::channel();
            state.approvals.lock().await.insert(id.clone(), sender);
            let _ = app.emit(
                "keel://agent-event",
                AgentEvent::ApprovalRequired {
                    id,
                    tool,
                    summary,
                    arguments,
                },
            );
            receiver.await.unwrap_or(ApprovalDecision::Deny)
        },
    )
    .await;
    *state.active_cli_stop.lock().await = None;
    state.approvals.lock().await.clear();
    if let Some(error) = recording_error {
        recorder.fail(&format!("Could not save part of this reply: {error}"))
    } else {
        match result {
            Ok(Some(content)) => {
                let event = AgentEvent::TurnFinished {
                    content,
                    thinking: recorder
                        .session
                        .messages
                        .last()
                        .and_then(|message| message.thinking.clone()),
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    tokens_per_sec: 0.0,
                    elapsed_secs: started.elapsed().as_secs_f64(),
                };
                let _ = app.emit("keel://agent-event", &event);
                recorder.record(&event)
            }
            Ok(None) => recorder.fail(
                "Code run stopped. Your partial answer and any approved changes have been kept.",
            ),
            Err(error) => recorder.fail(&format!("Could not finish this Code run: {error}")),
        }
    }
    .map_err(|error| error.to_string())?;
    Ok(recorder.session)
}

#[tauri::command]
pub async fn stop_generation(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(stop) = state.active_cli_stop.lock().await.as_ref() {
        let _ = stop.send(true);
    }
    if let Some(handle) = state.active_abort.lock().await.take() {
        handle.abort();
    }
    let mut map = state.approvals.lock().await;
    for (_, tx) in map.drain() {
        let _ = tx.send(ApprovalDecision::Deny);
    }
    Ok(())
}

#[tauri::command]
pub async fn respond_approval(
    state: State<'_, AppState>,
    id: String,
    decision: ApprovalDecision,
) -> Result<(), String> {
    if let Some(tx) = state.approvals.lock().await.remove(&id) {
        let _ = tx.send(decision);
    }
    Ok(())
}

#[tauri::command]
pub async fn apply_staged_edit_cmd(
    state: State<'_, AppState>,
    session_id: String,
    edit_id: String,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before changing this thread")?;
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let root = session
        .active_workspace()
        .ok_or_else(|| "No workspace folder selected".to_string())?;

    let edit = session
        .staged_edits
        .iter_mut()
        .find(|e| e.id == edit_id)
        .ok_or_else(|| "Staged edit not found".to_string())?;

    apply_staged_edit(&root, edit).map_err(|e| e.to_string())?;
    session.last_code_test = None;
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn discard_staged_edit_cmd(
    state: State<'_, AppState>,
    session_id: String,
    edit_id: String,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before changing this thread")?;
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let edit = session
        .staged_edits
        .iter_mut()
        .find(|e| e.id == edit_id)
        .ok_or_else(|| "Staged edit not found".to_string())?;
    edit.status = StagedEditStatus::Discarded;
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn apply_all_staged_edits(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before changing this thread")?;
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let root = session
        .active_workspace()
        .ok_or_else(|| "No workspace folder selected".to_string())?;

    let mut applied_any = false;
    let mut first_err: Option<String> = None;
    for edit in &mut session.staged_edits {
        if edit.status == StagedEditStatus::Pending {
            match apply_staged_edit(&root, edit) {
                Ok(()) => applied_any = true,
                Err(e) => {
                    first_err = Some(e.to_string());
                    break;
                }
            }
        }
    }
    if applied_any {
        session.last_code_test = None;
        session.updated_at = now_epoch();
        save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    }
    if let Some(err) = first_err {
        return Err(err);
    }
    Ok(session)
}

#[derive(Debug, Deserialize)]
pub struct CustomizeUpdate {
    pub custom_instructions: String,
    pub auto_apply_edits: bool,
    pub harness_mode: String,
    pub llama_server_path: Option<String>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub run_where: Option<String>,
    #[serde(default)]
    pub graphics_share: Option<String>,
}

#[tauri::command]
pub async fn save_customize_settings(
    state: State<'_, AppState>,
    update: CustomizeUpdate,
) -> Result<DesktopConfig, String> {
    let mut cfg = state.config.lock().await;
    cfg.custom_instructions = update.custom_instructions;
    cfg.auto_apply_edits = update.auto_apply_edits;
    cfg.harness_mode = update.harness_mode;
    cfg.llama_server_path = update.llama_server_path.filter(|s| !s.trim().is_empty());
    if let Some(ctx) = update.context_window {
        cfg.context_window = if ctx == 0 { 0 } else { ctx.clamp(2048, 32768) };
    }
    if let Some(run_where) = update.run_where {
        cfg.run_where = keel_core::hardware::canonical_run_where(&run_where);
    }
    if let Some(graphics_share) = update.graphics_share {
        cfg.graphics_share = keel_core::hardware::canonical_graphics_share(&graphics_share);
    }
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(cfg.clone())
}

#[derive(Debug, Deserialize)]
pub struct McpServerInput {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub scope: McpScope,
    #[serde(default)]
    pub workspace_path: Option<String>,
}

async fn inspect_current_mcp(
    state: &State<'_, AppState>,
    workspace: Option<&Path>,
) -> Vec<McpServerStatus> {
    let ws_trusted = {
        let cfg = state.config.lock().await;
        workspace
            .map(|p| cfg.is_mcp_workspace_trusted(p))
            .unwrap_or(false)
    };
    let loaded = load_mcp_configs_with_trust(&state.config_dir, workspace, ws_trusted);
    inspect_mcp_servers_with_trust(&loaded, workspace).await
}

#[tauri::command]
pub async fn save_mcp_server(
    state: State<'_, AppState>,
    input: McpServerInput,
    workspace_path: Option<String>,
) -> Result<Vec<McpServerStatus>, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() || input.command.trim().is_empty() {
        return Err("Server name and command are required".to_string());
    }
    let effective_ws = workspace_path
        .or(input.workspace_path)
        .filter(|s| !s.trim().is_empty());
    let server = McpServerConfig {
        command: input.command.trim().to_string(),
        args: input.args,
        env: HashMap::new(),
        enabled: true,
    };

    match input.scope {
        McpScope::Workspace => {
            let Some(ref ws_str) = effective_ws else {
                return Err("Pick a folder before saving a workspace MCP server.".to_string());
            };
            let ws_root = validate_workspace_root(Path::new(ws_str)).map_err(|e| e.to_string())?;
            let mut local = load_workspace_mcp_config(&ws_root);
            let had_other_entries = local.mcp_servers.keys().any(|k| k != &name);
            local.mcp_servers.insert(name, server);
            save_workspace_mcp_config(&ws_root, &local).map_err(|e| e.to_string())?;
            // Only auto-trust when this file had no other pre-existing unreviewed servers.
            if !had_other_entries {
                let mut cfg = state.config.lock().await;
                cfg.set_mcp_workspace_trusted(&ws_root, true);
                save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
            }
        }
        McpScope::Global => {
            let mut global = load_global_mcp_config(&state.config_dir);
            global.mcp_servers.insert(name, server);
            save_global_mcp_config(&state.config_dir, &global).map_err(|e| e.to_string())?;
        }
    }

    let ws = effective_ws.as_deref().map(Path::new);
    Ok(inspect_current_mcp(&state, ws).await)
}

#[tauri::command]
pub async fn remove_mcp_server(
    state: State<'_, AppState>,
    name: String,
    scope: Option<McpScope>,
    workspace_path: Option<String>,
) -> Result<Vec<McpServerStatus>, String> {
    let ws = workspace_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(Path::new);

    match scope {
        Some(McpScope::Workspace) => {
            let Some(ws_path) = ws else {
                return Err("Pick a folder to remove a workspace MCP server.".to_string());
            };
            let ws_root = validate_workspace_root(ws_path).map_err(|e| e.to_string())?;
            let mut local = load_workspace_mcp_config(&ws_root);
            local.mcp_servers.remove(&name);
            save_workspace_mcp_config(&ws_root, &local).map_err(|e| e.to_string())?;
        }
        Some(McpScope::Global) => {
            let mut global = load_global_mcp_config(&state.config_dir);
            global.mcp_servers.remove(&name);
            save_global_mcp_config(&state.config_dir, &global).map_err(|e| e.to_string())?;
        }
        None => {
            let mut removed = false;
            if let Some(ws_path) = ws {
                if let Ok(ws_root) = validate_workspace_root(ws_path) {
                    let mut local = load_workspace_mcp_config(&ws_root);
                    if local.mcp_servers.remove(&name).is_some() {
                        save_workspace_mcp_config(&ws_root, &local).map_err(|e| e.to_string())?;
                        removed = true;
                    }
                }
            }
            let mut global = load_global_mcp_config(&state.config_dir);
            if global.mcp_servers.remove(&name).is_some() || !removed {
                save_global_mcp_config(&state.config_dir, &global).map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(inspect_current_mcp(&state, ws).await)
}

#[tauri::command]
pub async fn set_workspace_mcp_trust(
    state: State<'_, AppState>,
    workspace_path: String,
    trusted: bool,
) -> Result<Vec<McpServerStatus>, String> {
    let ws_root = validate_workspace_root(Path::new(&workspace_path)).map_err(|e| e.to_string())?;
    {
        let mut cfg = state.config.lock().await;
        cfg.set_mcp_workspace_trusted(&ws_root, trusted);
        save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    }
    Ok(inspect_current_mcp(&state, Some(&ws_root)).await)
}

#[tauri::command]
pub async fn refresh_mcp_servers(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
) -> Result<Vec<McpServerStatus>, String> {
    let ws = workspace_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(Path::new);
    Ok(inspect_current_mcp(&state, ws).await)
}

#[tauri::command]
pub async fn connect_image_server(
    state: State<'_, AppState>,
    endpoint: String,
) -> Result<Vec<String>, String> {
    let endpoint = keel_core::images::local_endpoint(&endpoint).map_err(|e| e.to_string())?;
    let checkpoints = keel_core::images::probe_checkpoints(&endpoint)
        .await
        .map_err(|e| format!("Could not connect. Start AUTOMATIC1111 or Forge with --api. {e}"))?;
    let mut cfg = state.config.lock().await;
    cfg.image_endpoint = Some(endpoint);
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(checkpoints)
}

#[tauri::command]
pub async fn list_image_jobs(
    state: State<'_, AppState>,
) -> Result<Vec<keel_core::images::ImageJob>, String> {
    Ok(state.images.list().await)
}

#[tauri::command]
pub async fn start_image_job(
    app: AppHandle,
    state: State<'_, AppState>,
    workspace_path: String,
    mut request: keel_core::images::ImageRequest,
) -> Result<keel_core::images::ImageJob, String> {
    request.endpoint = state
        .config
        .lock()
        .await
        .image_endpoint
        .clone()
        .ok_or("Connect an image server first")?;
    state
        .images
        .start(Path::new(&workspace_path), request, move |job| {
            let _ = app.emit("keel://image-job", job);
        })
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_image_job(
    state: State<'_, AppState>,
    id: String,
) -> Result<keel_core::images::ImageJob, String> {
    state.images.cancel(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn read_workspace_png(
    workspace_path: String,
    rel_path: String,
) -> Result<String, String> {
    keel_core::images::read_workspace_png(Path::new(&workspace_path), &rel_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_runtime_status(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    Ok(state.runtime.lock().await.status())
}

#[tauri::command]
pub async fn create_code_worktree(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before creating an isolated checkout")?;
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    if session.app_preset != AppPreset::Code {
        return Err("Isolated checkouts are available in Code mode".to_string());
    }
    if session.code_checkout.is_some() {
        return Ok(session);
    }
    let folder = session
        .workspace_path
        .clone()
        .ok_or_else(|| "Choose a Git repository folder first".to_string())?;
    let destination = state.data_dir.join("worktrees").join(&session.id);
    let checkout = tokio::task::spawn_blocking(move || {
        keel_core::worktrees::CodeCheckout::create(Path::new(&folder), &destination)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    session.code_checkout = Some(checkout);
    session.applied_code_patch = None;
    session.last_code_test = None;
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn review_code_worktree(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<keel_core::worktrees::CodePatch, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before reviewing changes")?;
    let session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let checkout = session
        .code_checkout
        .ok_or_else(|| "Start an isolated checkout first".to_string())?;
    tokio::task::spawn_blocking(move || checkout.review())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_code_test(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before running tests")?;
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let checkout = session
        .code_checkout
        .clone()
        .ok_or_else(|| "Start an isolated checkout first".to_string())?;
    let result = tokio::task::spawn_blocking(move || checkout.run_test(&command))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    session.last_code_test = Some(result);
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn apply_code_worktree(
    state: State<'_, AppState>,
    session_id: String,
    patch: keel_core::worktrees::CodePatch,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before applying changes")?;
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let checkout = session
        .code_checkout
        .clone()
        .ok_or_else(|| "Start an isolated checkout first".to_string())?;
    let patch_clone = patch.clone();
    tokio::task::spawn_blocking(move || checkout.apply(&patch_clone))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    session.applied_code_patch = Some(patch);
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn undo_code_worktree(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before undoing changes")?;
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let checkout = session
        .code_checkout
        .clone()
        .ok_or_else(|| "No isolated checkout on this thread".to_string())?;
    let applied = session
        .applied_code_patch
        .clone()
        .ok_or_else(|| "No applied patch to undo".to_string())?;
    tokio::task::spawn_blocking(move || checkout.undo(&applied))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    session.applied_code_patch = None;
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn remove_code_worktree(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<ChatSession, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before removing checkout")?;
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    if let Some(checkout) = session.code_checkout.take() {
        tokio::task::spawn_blocking(move || checkout.remove())
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
    }
    session.applied_code_patch = None;
    session.last_code_test = None;
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[tauri::command]
pub async fn inspect_publish_status(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<keel_core::publish::PublishStatus, String> {
    let session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let folder = session
        .workspace_path
        .ok_or_else(|| "Choose a Git repository folder first".to_string())?;
    tokio::task::spawn_blocking(move || keel_core::publish::inspect_repo(Path::new(&folder)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn publish_code_changes(
    state: State<'_, AppState>,
    session_id: String,
    mut plan: keel_core::publish::PublishPlan,
    test_passed: Option<bool>,
) -> Result<keel_core::publish::PublishResult, String> {
    let _guard = state
        .turn_lock
        .try_lock()
        .map_err(|_| "Wait for the current reply to finish before publishing")?;
    let session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let folder = session
        .workspace_path
        .ok_or_else(|| "Choose a Git repository folder first".to_string())?;
    // Enforce server-recorded test status rather than trusting a stale client flag.
    let effective_test_passed = match (&session.last_code_test, &session.code_checkout) {
        (Some(result), _) => Some(result.passed),
        (None, Some(_)) => None,
        (None, None) => test_passed,
    };
    if plan.files.is_empty() {
        if let Some(ref applied) = session.applied_code_patch {
            plan.files = applied.paths.clone();
        }
    }
    tokio::task::spawn_blocking(move || {
        keel_core::publish::publish_changes(Path::new(&folder), &plan, effective_test_passed)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
