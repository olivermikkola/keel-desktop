use keel_core::agent::{run_agent_turn, AgentEvent, ApprovalDecision, TurnParams};
use keel_core::apps::{AppPreset, AppPresetInfo};
use keel_core::compaction::ChatTurnMessage;
use keel_core::harnesses::{detect_harnesses, run_external_harness, HarnessOption};
use keel_core::mcp::{
    inspect_mcp_servers, load_mcp_configs, save_global_mcp_config, McpConfigFile, McpServerConfig,
    McpServerStatus, McpToolInfo,
};
use keel_core::models::{
    catalog_download_url, download_gguf, list_models, models_dir, parse_huggingface_gguf_url,
    CatalogModel,
};
use keel_core::runtime::{probe_openai_models, RuntimeManager, RuntimeStatus};
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
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{oneshot, Mutex};
use tokio::task::AbortHandle;

pub struct AppState {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config: Arc<Mutex<DesktopConfig>>,
    pub runtime: Arc<Mutex<RuntimeManager>>,
    pub approvals: Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>,
    pub always_allowed: Arc<Mutex<HashSet<String>>>,
    pub active_abort: Arc<Mutex<Option<AbortHandle>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapState {
    pub version: String,
    pub config: DesktopConfig,
    pub models: Vec<CatalogModel>,
    pub runtime: RuntimeStatus,
    pub sessions: Vec<SessionSummary>,
    pub presets: Vec<AppPresetInfo>,
    pub harnesses: Vec<HarnessOption>,
    pub mcp_servers: Vec<McpServerStatus>,
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
    let mcp_cfg = load_mcp_configs(&state.config_dir, ws);
    let mcp_servers = inspect_mcp_servers(&mcp_cfg, ws).await;

    Ok(BootstrapState {
        version: keel_core::version().to_string(),
        config: cfg,
        models,
        runtime,
        sessions,
        presets,
        harnesses,
        mcp_servers,
    })
}

#[tauri::command]
pub async fn pick_workspace_folder() -> Result<Option<String>, String> {
    let output = tokio::task::spawn_blocking(pick_folder_native)
        .await
        .map_err(|e| e.to_string())?;
    Ok(output)
}

fn pick_folder_native() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(out) = std::process::Command::new("zenity")
            .args([
                "--file-selection",
                "--directory",
                "--title=Select Workspace Folder",
            ])
            .output()
        {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return Some(p);
                }
            }
        }
        if let Ok(out) = std::process::Command::new("kdialog")
            .args(["--getexistingdirectory", "."])
            .output()
        {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return Some(p);
                }
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("osascript")
            .args([
                "-e",
                "POSIX path of (choose folder with prompt \"Select Workspace Folder\")",
            ])
            .output()
        {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return Some(p.trim_end_matches('/').to_string());
                }
            }
        }
        None
    }
    #[cfg(target_os = "windows")]
    {
        let script = "Add-Type -AssemblyName System.Windows.Forms; $f = New-Object System.Windows.Forms.FolderBrowserDialog; if ($f.ShowDialog() -eq 'OK') { Write-Output $f.SelectedPath }";
        if let Ok(out) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", script])
            .output()
        {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !p.is_empty() {
                    return Some(p);
                }
            }
        }
        None
    }
}

#[tauri::command]
pub async fn set_workspace_folder(
    state: State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let canon = validate_workspace_root(Path::new(&path)).map_err(|e| e.to_string())?;
    let canon_str = canon.to_string_lossy().to_string();
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
    let bytes = std::fs::read(&target).map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("File is larger than 1 MB preview limit".to_string());
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[tauri::command]
pub async fn create_session(
    state: State<'_, AppState>,
    preset: AppPreset,
    workspace_path: Option<String>,
) -> Result<ChatSession, String> {
    let session = ChatSession {
        id: uuid::Uuid::new_v4().to_string(),
        title: format!("New {} chat", preset.title()),
        app_preset: preset,
        workspace_path,
        messages: Vec::new(),
        staged_edits: Vec::new(),
        updated_at: now_epoch(),
    };
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
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
    delete_session(&state.data_dir, &id).map_err(|e| e.to_string())?;
    Ok(list_sessions(&state.data_dir))
}

#[tauri::command]
pub async fn start_model(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<RuntimeStatus, String> {
    let mut cfg = state.config.lock().await;
    let models = list_models(&state.data_dir, &cfg.custom_model_paths);
    let model = models
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("Unknown model: {model_id}"))?;
    let path_str = model
        .local_path
        .ok_or_else(|| format!("Model `{}` is not installed yet", model.name))?;

    let mut rt = state.runtime.lock().await;
    let status = rt
        .start_model(
            &model.id,
            Path::new(&path_str),
            model.context_size,
            cfg.llama_server_path.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;

    cfg.selected_model_id = model.id;
    cfg.external_base_url = None;
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(status)
}

#[tauri::command]
pub async fn stop_runtime(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let mut rt = state.runtime.lock().await;
    rt.stop().await;
    Ok(rt.status())
}

#[tauri::command]
pub async fn set_external_endpoint(
    state: State<'_, AppState>,
    base_url: String,
    model_name: Option<String>,
) -> Result<RuntimeStatus, String> {
    let probed = probe_openai_models(&base_url).await.unwrap_or_default();
    let chosen = model_name
        .filter(|s| !s.trim().is_empty())
        .or_else(|| probed.into_iter().next())
        .unwrap_or_else(|| "default".to_string());

    let mut rt = state.runtime.lock().await;
    rt.set_external_url(&base_url, Some(chosen.clone()));
    let status = rt.status();

    let mut cfg = state.config.lock().await;
    cfg.external_base_url = Some(base_url.trim_end_matches('/').to_string());
    cfg.external_model_name = Some(chosen);
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(status)
}

#[tauri::command]
pub async fn download_catalog_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<Vec<CatalogModel>, String> {
    let cfg = state.config.lock().await.clone();
    let models = list_models(&state.data_dir, &cfg.custom_model_paths);
    let model = models
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("Unknown model: {model_id}"))?;
    let url = catalog_download_url(&model);
    let dest = models_dir(&state.data_dir).join(&model.file_name);

    download_gguf(&url, &dest, model.sha256.as_deref(), &model.id, |prog| {
        let _ = app.emit("keel://download-progress", prog);
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(list_models(&state.data_dir, &cfg.custom_model_paths))
}

#[tauri::command]
pub async fn download_huggingface_gguf(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Result<Vec<CatalogModel>, String> {
    let (_repo, _rev, file_name, resolve_url) =
        parse_huggingface_gguf_url(&url).map_err(|e| e.to_string())?;
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
        return Err(format!("File not found: {}", pb.display()));
    }
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
    pub workspace_path: Option<String>,
}

#[tauri::command]
pub async fn send_message(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SendMessageInput,
) -> Result<ChatSession, String> {
    let mut session =
        load_session(&state.data_dir, &input.session_id).map_err(|e| e.to_string())?;
    if input.workspace_path.is_some() {
        session.workspace_path = input.workspace_path.clone();
    }

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
    });
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;

    let cfg = state.config.lock().await.clone();
    let rt_status = state.runtime.lock().await.status();

    // External CLI harness path when selected.
    if cfg.harness_mode != "keel" {
        let ws = session
            .workspace_path
            .as_deref()
            .map(PathBuf::from)
            .ok_or_else(|| {
                "Select a workspace folder before using an external CLI harness".to_string()
            })?;
        let text = run_external_harness(
            &cfg.harness_mode,
            &ws,
            &input.prompt,
            Some(&rt_status.base_url),
            rt_status.active_model_id.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())?;

        session.messages.push(UiMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: "assistant".to_string(),
            content: text.clone(),
            thinking: None,
            tool_calls: Vec::new(),
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
            },
        );
        return Ok(session);
    }

    let base_url = cfg
        .external_base_url
        .clone()
        .unwrap_or(rt_status.base_url.clone());
    let model_name = cfg
        .external_model_name
        .clone()
        .or(rt_status.active_model_id.clone())
        .unwrap_or_else(|| cfg.selected_model_id.clone());

    let catalog = list_models(&state.data_dir, &cfg.custom_model_paths);
    let context_size = catalog
        .iter()
        .find(|m| m.id == cfg.selected_model_id)
        .map(|m| m.context_size as usize)
        .unwrap_or(8192);

    let ws_path = session.workspace_path.as_deref().map(PathBuf::from);
    let mcp_cfg: McpConfigFile = load_mcp_configs(&state.config_dir, ws_path.as_deref());
    let mcp_statuses = inspect_mcp_servers(&mcp_cfg, ws_path.as_deref()).await;
    let mcp_tools: Vec<McpToolInfo> = mcp_statuses.into_iter().flat_map(|s| s.tools).collect();

    let history: Vec<ChatTurnMessage> = session
        .messages
        .iter()
        .map(|m| ChatTurnMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            tool_call_id: None,
            tool_calls: None,
        })
        .collect();

    let mut allowed = state.always_allowed.lock().await.clone();
    let approvals_map = Arc::clone(&state.approvals);
    let app_for_emit = app.clone();

    let params = TurnParams {
        base_url,
        model_name,
        context_size,
        app_preset: session.app_preset,
        custom_instructions: cfg.custom_instructions.clone(),
        workspace_root: ws_path,
        auto_apply_edits: cfg.auto_apply_edits,
        enable_thinking: input.enable_thinking,
        history,
        mcp_config: mcp_cfg,
        mcp_tools,
        max_steps: 12,
    };

    let handle = tokio::spawn(async move {
        let res = run_agent_turn(
            params,
            &mut allowed,
            |ev| {
                let _ = app_for_emit.emit("keel://agent-event", ev);
            },
            |req_id, _tool, _summary, _args| {
                let map = Arc::clone(&approvals_map);
                async move {
                    let (tx, rx) = oneshot::channel();
                    map.lock().await.insert(req_id, tx);
                    rx.await.unwrap_or(ApprovalDecision::Deny)
                }
            },
        )
        .await;
        (res, allowed)
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

    let (turn_res, updated_allowed) = match join_res {
        Ok(pair) => pair,
        Err(e) if e.is_cancelled() => {
            return Err("Generation stopped.".to_string());
        }
        Err(e) => return Err(e.to_string()),
    };

    *state.always_allowed.lock().await = updated_allowed;

    let outcome = turn_res.map_err(|e| e.to_string())?;
    session.staged_edits.extend(outcome.staged_edits);
    session.messages.push(UiMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: "assistant".to_string(),
        content: outcome.content,
        thinking: outcome.thinking,
        tool_calls: outcome.tool_logs,
    });
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;

    Ok(session)
}

#[tauri::command]
pub async fn stop_generation(state: State<'_, AppState>) -> Result<(), String> {
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
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let ws = session
        .workspace_path
        .clone()
        .ok_or_else(|| "No workspace folder selected".to_string())?;
    let root = PathBuf::from(ws);

    let edit = session
        .staged_edits
        .iter_mut()
        .find(|e| e.id == edit_id)
        .ok_or_else(|| "Staged edit not found".to_string())?;

    apply_staged_edit(&root, edit).map_err(|e| e.to_string())?;
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
    let mut session = load_session(&state.data_dir, &session_id).map_err(|e| e.to_string())?;
    let ws = session
        .workspace_path
        .clone()
        .ok_or_else(|| "No workspace folder selected".to_string())?;
    let root = PathBuf::from(ws);

    for edit in &mut session.staged_edits {
        if edit.status == StagedEditStatus::Pending {
            apply_staged_edit(&root, edit).map_err(|e| e.to_string())?;
        }
    }
    session.updated_at = now_epoch();
    save_session(&state.data_dir, &session).map_err(|e| e.to_string())?;
    Ok(session)
}

#[derive(Debug, Deserialize)]
pub struct CustomizeUpdate {
    pub custom_instructions: String,
    pub auto_apply_edits: bool,
    pub harness_mode: String,
    pub llama_server_path: Option<String>,
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
    save_config(&state.config_dir, &cfg).map_err(|e| e.to_string())?;
    Ok(cfg.clone())
}

#[derive(Debug, Deserialize)]
pub struct McpServerInput {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
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
    let mut global = load_mcp_configs(&state.config_dir, None);
    global.mcp_servers.insert(
        name,
        McpServerConfig {
            command: input.command.trim().to_string(),
            args: input.args,
            env: HashMap::new(),
            enabled: true,
        },
    );
    save_global_mcp_config(&state.config_dir, &global).map_err(|e| e.to_string())?;
    let ws = workspace_path.as_deref().map(Path::new);
    let merged = load_mcp_configs(&state.config_dir, ws);
    Ok(inspect_mcp_servers(&merged, ws).await)
}

#[tauri::command]
pub async fn remove_mcp_server(
    state: State<'_, AppState>,
    name: String,
    workspace_path: Option<String>,
) -> Result<Vec<McpServerStatus>, String> {
    let mut global = load_mcp_configs(&state.config_dir, None);
    global.mcp_servers.remove(&name);
    save_global_mcp_config(&state.config_dir, &global).map_err(|e| e.to_string())?;
    let ws = workspace_path.as_deref().map(Path::new);
    let merged = load_mcp_configs(&state.config_dir, ws);
    Ok(inspect_mcp_servers(&merged, ws).await)
}

#[tauri::command]
pub async fn refresh_mcp_servers(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
) -> Result<Vec<McpServerStatus>, String> {
    let ws = workspace_path.as_deref().map(Path::new);
    let merged = load_mcp_configs(&state.config_dir, ws);
    Ok(inspect_mcp_servers(&merged, ws).await)
}
