use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::apps::AppPreset;
use crate::tools::StagedEdit;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    pub selected_model_id: String,
    #[serde(default)]
    pub image_endpoint: Option<String>,
    #[serde(default)]
    pub personal_context: String,
    pub external_base_url: Option<String>,
    pub external_model_name: Option<String>,
    pub llama_server_path: Option<String>,
    pub custom_instructions: String,
    pub auto_apply_edits: bool,
    pub harness_mode: String,
    pub custom_model_paths: HashMap<String, String>,
    pub recent_workspaces: Vec<String>,
    #[serde(default)]
    pub trusted_mcp_workspaces: Vec<String>,
    /// Preferred context window in tokens (`0` = automatic RAM-aware default).
    #[serde(default)]
    pub context_window: u32,
    /// Where the model runs: `auto`, `graphics`, or `processor`.
    #[serde(default = "default_auto_choice")]
    pub run_where: String,
    /// How much of the model sits on the graphics chip: `auto`, `full`, or `partial`.
    #[serde(default = "default_auto_choice")]
    pub graphics_share: String,
}

fn default_auto_choice() -> String {
    "auto".to_string()
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            image_endpoint: None,
            personal_context: String::new(),
            selected_model_id: "bonsai-4b".to_string(),
            external_base_url: None,
            external_model_name: None,
            llama_server_path: None,
            custom_instructions: String::new(),
            auto_apply_edits: false,
            harness_mode: "keel".to_string(),
            custom_model_paths: HashMap::new(),
            recent_workspaces: Vec::new(),
            trusted_mcp_workspaces: Vec::new(),
            context_window: 0,
            run_where: default_auto_choice(),
            graphics_share: default_auto_choice(),
        }
    }
}

impl DesktopConfig {
    pub fn normalize_workspace_key(workspace: &Path) -> String {
        std::fs::canonicalize(workspace)
            .unwrap_or_else(|_| workspace.to_path_buf())
            .to_string_lossy()
            .into_owned()
    }

    pub fn is_mcp_workspace_trusted(&self, workspace: &Path) -> bool {
        let key = Self::normalize_workspace_key(workspace);
        let raw = workspace.to_string_lossy();
        self.trusted_mcp_workspaces
            .iter()
            .any(|item| item == &key || item == raw.as_ref())
    }

    pub fn set_mcp_workspace_trusted(&mut self, workspace: &Path, trusted: bool) {
        let key = Self::normalize_workspace_key(workspace);
        let raw = workspace.to_string_lossy().into_owned();
        self.trusted_mcp_workspaces
            .retain(|item| item != &key && item != &raw);
        if trusted {
            self.trusted_mcp_workspaces.push(key);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiToolLog {
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: Option<String>,
    pub is_error: bool,
    #[serde(default)]
    pub edit_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub thinking: Option<String>,
    pub tool_calls: Vec<UiToolLog>,
    #[serde(default)]
    pub attachments: Vec<crate::documents::Document>,
    #[serde(default = "default_status")]
    pub status: String,
}

fn default_status() -> String {
    "complete".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    pub app_preset: AppPreset,
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub keel_session_log: Option<PathBuf>,
    #[serde(default)]
    pub code_checkout: Option<crate::worktrees::CodeCheckout>,
    #[serde(default)]
    pub applied_code_patch: Option<crate::worktrees::CodePatch>,
    #[serde(default)]
    pub last_code_test: Option<crate::worktrees::CodeTestResult>,
    pub messages: Vec<UiMessage>,
    pub staged_edits: Vec<StagedEdit>,
    pub updated_at: u64,
}

impl ChatSession {
    /// Active working directory for tools: the detached Code worktree when present.
    pub fn active_workspace(&self) -> Option<PathBuf> {
        if self.app_preset == AppPreset::Code {
            if let Some(ref checkout) = self.code_checkout {
                return Some(checkout.isolated.clone());
            }
        }
        self.workspace_path.as_deref().map(PathBuf::from)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    pub app_preset: AppPreset,
    pub workspace_path: Option<String>,
    pub message_count: usize,
    pub updated_at: u64,
}

pub fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("keel-desktop")
}

pub fn default_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("keel-desktop")
}

pub fn load_config(config_dir: &Path) -> DesktopConfig {
    let path = config_dir.join("config.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(config_dir: &Path, cfg: &DesktopConfig) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let path = config_dir.join("config.json");
    crate::projects::write_json(&path, cfg)?;
    Ok(())
}

pub fn sessions_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("sessions")
}

pub fn save_session(data_dir: &Path, session: &ChatSession) -> Result<()> {
    let dir = sessions_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", session.id));
    crate::projects::valid_id(&session.id)?;
    crate::projects::write_json(&path, session)?;
    Ok(())
}

pub fn load_session(data_dir: &Path, id: &str) -> Result<ChatSession> {
    crate::projects::valid_id(id)?;
    let path = sessions_dir(data_dir).join(format!("{id}.json"));
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn delete_session(data_dir: &Path, id: &str) -> Result<()> {
    crate::projects::valid_id(id)?;
    let path = sessions_dir(data_dir).join(format!("{id}.json"));
    if path.is_file() {
        let trash = data_dir.join("trash");
        std::fs::create_dir_all(&trash)?;
        std::fs::rename(path, trash.join(format!("{id}.json")))?;
    }
    Ok(())
}

pub fn list_sessions(data_dir: &Path) -> Vec<SessionSummary> {
    let dir = sessions_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(&p) {
            if let Ok(s) = serde_json::from_str::<ChatSession>(&raw) {
                out.push(SessionSummary {
                    id: s.id,
                    title: s.title,
                    project_id: s.project_id,
                    pinned: s.pinned,
                    app_preset: s.app_preset,
                    workspace_path: s.workspace_path,
                    message_count: s.messages.len(),
                    updated_at: s.updated_at,
                });
            }
        }
    }
    out.sort_by_key(|b| (std::cmp::Reverse(b.pinned), std::cmp::Reverse(b.updated_at)));
    out
}

pub fn recover_interrupted(data: &Path) -> anyhow::Result<()> {
    for summary in list_sessions(data) {
        let mut session = load_session(data, &summary.id)?;
        let mut changed = false;
        for message in &mut session.messages {
            if message.status == "streaming" {
                message.status = "failed".into();
                message.content.push_str("\n\nKeel closed before this reply finished. The partial answer and proposed edits have been kept.");
                changed = true;
            }
        }
        if changed {
            save_session(data, &session)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_config_and_sessions() {
        let dir = std::env::temp_dir().join(format!("keel_sess_test_{}", uuid::Uuid::new_v4()));
        let cfg = DesktopConfig {
            custom_instructions: "Be concise.".to_string(),
            ..Default::default()
        };
        save_config(&dir, &cfg).unwrap();
        let loaded = load_config(&dir);
        assert_eq!(loaded.custom_instructions, "Be concise.");

        let sess = ChatSession {
            id: "s1".to_string(),
            title: "Test".to_string(),
            project_id: None,
            pinned: false,
            app_preset: AppPreset::Docs,
            workspace_path: None,
            keel_session_log: None,
            code_checkout: None,
            applied_code_patch: None,
            last_code_test: None,
            messages: vec![],
            staged_edits: vec![],
            updated_at: 42,
        };
        save_session(&dir, &sess).unwrap();
        let list = list_sessions(&dir);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "Test");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn older_config_keeps_the_selected_model_and_defaults_speed_choices() {
        let dir = std::env::temp_dir().join(format!("keel_cfg_old_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("config.json"),
            r#"{"selected_model_id":"lfm3-1.7b","external_base_url":null,"external_model_name":null,"llama_server_path":"/kept/llama-server","custom_instructions":"","auto_apply_edits":false,"harness_mode":"keel","custom_model_paths":{},"recent_workspaces":[]}"#,
        )
        .unwrap();
        let loaded = load_config(&dir);
        assert_eq!(loaded.selected_model_id, "lfm3-1.7b");
        assert_eq!(loaded.run_where, "auto");
        assert_eq!(loaded.graphics_share, "auto");
        assert_eq!(
            loaded.llama_server_path.as_deref(),
            Some("/kept/llama-server")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
