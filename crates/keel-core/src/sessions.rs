use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::apps::AppPreset;
use crate::tools::StagedEdit;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopConfig {
    pub selected_model_id: String,
    pub external_base_url: Option<String>,
    pub external_model_name: Option<String>,
    pub llama_server_path: Option<String>,
    pub custom_instructions: String,
    pub auto_apply_edits: bool,
    pub harness_mode: String,
    pub custom_model_paths: HashMap<String, String>,
    pub recent_workspaces: Vec<String>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            selected_model_id: "bonsai-4b".to_string(),
            external_base_url: None,
            external_model_name: None,
            llama_server_path: None,
            custom_instructions: String::new(),
            auto_apply_edits: false,
            harness_mode: "keel".to_string(),
            custom_model_paths: HashMap::new(),
            recent_workspaces: Vec::new(),
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub app_preset: AppPreset,
    pub workspace_path: Option<String>,
    pub messages: Vec<UiMessage>,
    pub staged_edits: Vec<StagedEdit>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
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
    std::fs::write(path, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

pub fn sessions_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("sessions")
}

pub fn save_session(data_dir: &Path, session: &ChatSession) -> Result<()> {
    let dir = sessions_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", session.id));
    std::fs::write(path, serde_json::to_string_pretty(session)?)?;
    Ok(())
}

pub fn load_session(data_dir: &Path, id: &str) -> Result<ChatSession> {
    let path = sessions_dir(data_dir).join(format!("{id}.json"));
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn delete_session(data_dir: &Path, id: &str) -> Result<()> {
    let path = sessions_dir(data_dir).join(format!("{id}.json"));
    if path.is_file() {
        std::fs::remove_file(path)?;
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
                    app_preset: s.app_preset,
                    workspace_path: s.workspace_path,
                    message_count: s.messages.len(),
                    updated_at: s.updated_at,
                });
            }
        }
    }
    out.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
    out
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
            app_preset: AppPreset::Docs,
            workspace_path: None,
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
}
