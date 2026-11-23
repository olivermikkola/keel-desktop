use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const MCP_TIMEOUT_SECS: u64 = 30;
const MAX_MCP_LINE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MCP_TOOL_OUTPUT_CHARS: usize = 24_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpScope {
    #[default]
    Global,
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfigFile {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub server: String,
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub input_schema: Value,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerStatus {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub enabled: bool,
    #[serde(default)]
    pub scope: McpScope,
    #[serde(default = "default_true")]
    pub trusted: bool,
    pub tools: Vec<McpToolInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ScopedMcpEntry {
    pub name: String,
    pub config: McpServerConfig,
    pub scope: McpScope,
    pub trusted: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LoadedMcpConfigs {
    /// Servers allowed to run (global servers + trusted workspace servers).
    pub active: McpConfigFile,
    /// All declared servers for display, including untrusted workspace definitions.
    pub entries: Vec<ScopedMcpEntry>,
}

pub fn global_mcp_config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("mcp.json")
}

pub fn workspace_mcp_config_path(workspace: &Path) -> PathBuf {
    workspace.join(".keel").join("mcp.json")
}

pub fn load_global_mcp_config(config_dir: &Path) -> McpConfigFile {
    let global = global_mcp_config_path(config_dir);
    std::fs::read_to_string(global)
        .ok()
        .and_then(|raw| serde_json::from_str::<McpConfigFile>(&raw).ok())
        .unwrap_or_default()
}

pub fn load_workspace_mcp_config(workspace: &Path) -> McpConfigFile {
    let local = workspace_mcp_config_path(workspace);
    std::fs::read_to_string(local)
        .ok()
        .and_then(|raw| serde_json::from_str::<McpConfigFile>(&raw).ok())
        .unwrap_or_default()
}

/// Loads global and (when trusted) workspace MCP server configs.
/// Untrusted workspace servers are returned in `entries` with `trusted = false`
/// and are never merged into `active` or permitted to shadow global servers.
pub fn load_mcp_configs_with_trust(
    config_dir: &Path,
    workspace: Option<&Path>,
    workspace_trusted: bool,
) -> LoadedMcpConfigs {
    let global = load_global_mcp_config(config_dir);
    let mut active = McpConfigFile::default();
    let mut entries = Vec::new();

    for (name, server) in &global.mcp_servers {
        active.mcp_servers.insert(name.clone(), server.clone());
        entries.push(ScopedMcpEntry {
            name: name.clone(),
            config: server.clone(),
            scope: McpScope::Global,
            trusted: true,
        });
    }

    if let Some(ws) = workspace {
        let local = load_workspace_mcp_config(ws);
        for (name, server) in local.mcp_servers {
            if workspace_trusted {
                active.mcp_servers.insert(name.clone(), server.clone());
                entries.retain(|e| !(e.name == name && e.scope == McpScope::Global));
                entries.push(ScopedMcpEntry {
                    name,
                    config: server,
                    scope: McpScope::Workspace,
                    trusted: true,
                });
            } else {
                entries.push(ScopedMcpEntry {
                    name,
                    config: server,
                    scope: McpScope::Workspace,
                    trusted: false,
                });
            }
        }
    }

    entries.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.scope.cmp_order().cmp(&b.scope.cmp_order()))
    });
    LoadedMcpConfigs { active, entries }
}

impl McpScope {
    fn cmp_order(self) -> u8 {
        match self {
            McpScope::Global => 0,
            McpScope::Workspace => 1,
        }
    }
}

pub fn load_mcp_configs(config_dir: &Path, workspace: Option<&Path>) -> McpConfigFile {
    load_mcp_configs_with_trust(config_dir, workspace, true).active
}

pub fn save_global_mcp_config(config_dir: &Path, cfg: &McpConfigFile) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let path = global_mcp_config_path(config_dir);
    write_mcp_file_atomic(&path, cfg)
}

pub fn save_workspace_mcp_config(workspace: &Path, cfg: &McpConfigFile) -> Result<()> {
    let keel_dir = workspace.join(".keel");
    std::fs::create_dir_all(&keel_dir)?;
    let path = workspace_mcp_config_path(workspace);
    write_mcp_file_atomic(&path, cfg)
}

fn write_mcp_file_atomic(path: &Path, cfg: &McpConfigFile) -> Result<()> {
    let body = serde_json::to_string_pretty(cfg)?;
    let tmp = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

async fn read_bounded_line(
    reader: &mut BufReader<ChildStdout>,
    limit: usize,
) -> Result<Option<String>> {
    let mut raw = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if raw.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            let take = pos + 1;
            if raw.len() + take > limit {
                bail!("MCP server response line exceeded {limit} bytes");
            }
            raw.extend_from_slice(&available[..take]);
            reader.consume(take);
            break;
        }
        let len = available.len();
        if raw.len() + len > limit {
            bail!("MCP server response line exceeded {limit} bytes");
        }
        raw.extend_from_slice(available);
        reader.consume(len);
    }
    Ok(Some(String::from_utf8_lossy(&raw).into_owned()))
}

fn truncate_mcp_output(text: String) -> String {
    if text.chars().count() <= MAX_MCP_TOOL_OUTPUT_CHARS {
        return text;
    }
    let truncated: String = text.chars().take(MAX_MCP_TOOL_OUTPUT_CHARS).collect();
    format!(
        "{truncated}\n\n[MCP tool output truncated after {MAX_MCP_TOOL_OUTPUT_CHARS} characters]"
    )
}

fn format_tool_call_result(tool_name: &str, res: &Value) -> Result<String> {
    let body = if let Some(arr) = res.get("content").and_then(|c| c.as_array()) {
        let texts: Vec<String> = arr
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()).map(String::from))
            .collect();
        if !texts.is_empty() {
            texts.join("\n")
        } else {
            serde_json::to_string_pretty(res)?
        }
    } else {
        serde_json::to_string_pretty(res)?
    };
    let bounded = truncate_mcp_output(body);
    if res.get("isError").and_then(|v| v.as_bool()) == Some(true) {
        bail!("MCP tool `{tool_name}` reported an error: {bounded}");
    }
    Ok(bounded)
}

struct McpSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpSession {
    async fn spawn(cfg: &McpServerConfig, workspace: Option<&Path>) -> Result<Self> {
        let mut cmd = Command::new(&cfg.command);
        cmd.args(&cfg.args)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(ws) = workspace {
            cmd.current_dir(ws);
        }
        let mut child = cmd
            .spawn()
            .with_context(|| format!("Failed to launch MCP server `{}`", cfg.command))?;
        let stdin = child.stdin.take().context("Missing MCP stdin")?;
        let stdout = BufReader::new(child.stdout.take().context("Missing MCP stdout")?);
        let mut session = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };
        if let Err(err) = session.initialize().await {
            session.close().await;
            return Err(err);
        }
        Ok(session)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        let deadline = Duration::from_secs(MCP_TIMEOUT_SECS);
        tokio::time::timeout(deadline, async {
            loop {
                let Some(buf) = read_bounded_line(&mut self.stdout, MAX_MCP_LINE_BYTES).await?
                else {
                    bail!("MCP server closed stdout unexpectedly");
                };
                let trimmed = buf.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };
                if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                    if let Some(err) = val.get("error") {
                        bail!("MCP error: {err}");
                    }
                    return Ok(val.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        })
        .await
        .context("MCP request timed out")?
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let mut line = serde_json::to_string(&payload)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn initialize(&mut self) -> Result<()> {
        let _ = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "keel-desktop",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    async fn list_tools(&mut self, server_name: &str) -> Result<Vec<McpToolInfo>> {
        let res = self.request("tools/list", json!({})).await?;
        let tools = res
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                let description = t
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                let input_schema = t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                let read_only = t
                    .get("annotations")
                    .and_then(|a| a.get("readOnlyHint"))
                    .and_then(|b| b.as_bool())
                    .unwrap_or(false);
                Some(McpToolInfo {
                    server: server_name.to_string(),
                    full_name: format!("mcp__{server_name}__{name}"),
                    name,
                    description,
                    input_schema,
                    read_only,
                })
            })
            .collect())
    }

    async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<String> {
        let res = self
            .request(
                "tools/call",
                json!({
                    "name": name,
                    "arguments": arguments
                }),
            )
            .await?;
        format_tool_call_result(name, &res)
    }

    async fn close(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

/// Discovers tools from trusted/enabled MCP servers while reporting untrusted workspace servers
/// without spawning them.
pub async fn inspect_mcp_servers_with_trust(
    loaded: &LoadedMcpConfigs,
    workspace: Option<&Path>,
) -> Vec<McpServerStatus> {
    let mut statuses = Vec::new();
    for entry in &loaded.entries {
        if !entry.trusted {
            statuses.push(McpServerStatus {
                name: entry.name.clone(),
                command: entry.config.command.clone(),
                args: entry.config.args.clone(),
                enabled: entry.config.enabled,
                scope: entry.scope,
                trusted: false,
                tools: Vec::new(),
                error: Some(
                    "Untrusted workspace MCP server. Review and trust this folder to run it."
                        .to_string(),
                ),
            });
            continue;
        }
        if !entry.config.enabled {
            statuses.push(McpServerStatus {
                name: entry.name.clone(),
                command: entry.config.command.clone(),
                args: entry.config.args.clone(),
                enabled: false,
                scope: entry.scope,
                trusted: true,
                tools: Vec::new(),
                error: None,
            });
            continue;
        }
        match McpSession::spawn(&entry.config, workspace).await {
            Ok(mut session) => {
                let res = session.list_tools(&entry.name).await;
                session.close().await;
                match res {
                    Ok(tools) => statuses.push(McpServerStatus {
                        name: entry.name.clone(),
                        command: entry.config.command.clone(),
                        args: entry.config.args.clone(),
                        enabled: true,
                        scope: entry.scope,
                        trusted: true,
                        tools,
                        error: None,
                    }),
                    Err(e) => statuses.push(McpServerStatus {
                        name: entry.name.clone(),
                        command: entry.config.command.clone(),
                        args: entry.config.args.clone(),
                        enabled: true,
                        scope: entry.scope,
                        trusted: true,
                        tools: Vec::new(),
                        error: Some(e.to_string()),
                    }),
                }
            }
            Err(e) => statuses.push(McpServerStatus {
                name: entry.name.clone(),
                command: entry.config.command.clone(),
                args: entry.config.args.clone(),
                enabled: true,
                scope: entry.scope,
                trusted: true,
                tools: Vec::new(),
                error: Some(e.to_string()),
            }),
        }
    }
    statuses.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then(a.scope.cmp_order().cmp(&b.scope.cmp_order()))
    });
    statuses
}

/// Discovers tools from all configured MCP servers.
pub async fn inspect_mcp_servers(
    cfg: &McpConfigFile,
    workspace: Option<&Path>,
) -> Vec<McpServerStatus> {
    let entries = cfg
        .mcp_servers
        .iter()
        .map(|(name, config)| ScopedMcpEntry {
            name: name.clone(),
            config: config.clone(),
            scope: McpScope::Global,
            trusted: true,
        })
        .collect();
    let loaded = LoadedMcpConfigs {
        active: cfg.clone(),
        entries,
    };
    inspect_mcp_servers_with_trust(&loaded, workspace).await
}

pub async fn invoke_mcp_tool(
    cfg: &McpConfigFile,
    workspace: Option<&Path>,
    server_name: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<String> {
    let server = cfg
        .mcp_servers
        .get(server_name)
        .with_context(|| format!("Unknown MCP server: {server_name}"))?;
    let mut session = McpSession::spawn(server, workspace).await?;
    let result = session.call_tool(tool_name, arguments).await;
    session.close().await;
    result
}

pub fn mcp_tools_to_openai_schemas(tools: &[McpToolInfo]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.full_name,
                    "description": format!("[MCP:{}] {}", t.server, t.description),
                    "parameters": t.input_schema
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_global_and_workspace_mcp_configs() {
        let dir = std::env::temp_dir().join(format!("keel_mcp_test_{}", uuid::Uuid::new_v4()));
        let cfg_dir = dir.join("cfg");
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join(".keel")).unwrap();

        let mut global = McpConfigFile::default();
        global.mcp_servers.insert(
            "fs".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "server-fs".to_string()],
                env: HashMap::new(),
                enabled: true,
            },
        );
        save_global_mcp_config(&cfg_dir, &global).unwrap();

        std::fs::write(
            ws.join(".keel/mcp.json"),
            r#"{"mcpServers":{"local":{"command":"python3","args":["srv.py"]}}}"#,
        )
        .unwrap();

        let merged = load_mcp_configs(&cfg_dir, Some(&ws));
        assert!(merged.mcp_servers.contains_key("fs"));
        assert!(merged.mcp_servers.contains_key("local"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn untrusted_workspace_mcp_is_not_spawned_and_cannot_shadow_global() {
        let dir = std::env::temp_dir().join(format!("keel_mcp_trust_{}", uuid::Uuid::new_v4()));
        let cfg_dir = dir.join("cfg");
        let ws = dir.join("ws");
        std::fs::create_dir_all(ws.join(".keel")).unwrap();
        let marker = dir.join("should-not-exist");

        let mut global = McpConfigFile::default();
        global.mcp_servers.insert(
            "shared".to_string(),
            McpServerConfig {
                command: "global-tool".to_string(),
                args: vec![],
                env: HashMap::new(),
                enabled: false,
            },
        );
        save_global_mcp_config(&cfg_dir, &global).unwrap();

        let mut local = McpConfigFile::default();
        local.mcp_servers.insert(
            "shared".to_string(),
            McpServerConfig {
                command: "sh".to_string(),
                args: vec!["-c".to_string(), format!("touch {}", marker.display())],
                env: HashMap::new(),
                enabled: true,
            },
        );
        save_workspace_mcp_config(&ws, &local).unwrap();

        let loaded = load_mcp_configs_with_trust(&cfg_dir, Some(&ws), false);
        assert_eq!(loaded.active.mcp_servers["shared"].command, "global-tool");
        assert!(loaded
            .entries
            .iter()
            .any(|e| e.scope == McpScope::Workspace && !e.trusted));

        let statuses = inspect_mcp_servers_with_trust(&loaded, Some(&ws)).await;
        assert!(
            !marker.exists(),
            "untrusted workspace MCP must never execute"
        );
        let ws_status = statuses
            .iter()
            .find(|s| s.scope == McpScope::Workspace)
            .expect("workspace status entry");
        assert!(!ws_status.trusted);
        assert!(ws_status.tools.is_empty());
        assert!(ws_status
            .error
            .as_deref()
            .unwrap_or("")
            .contains("Untrusted"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_tool_result_handles_is_error_and_truncates_large_output() {
        let err_val = json!({
            "isError": true,
            "content": [{"type": "text", "text": "permission denied"}]
        });
        let err = format_tool_call_result("write_note", &err_val).unwrap_err();
        assert!(err.to_string().contains("permission denied"));

        let big = "a".repeat(MAX_MCP_TOOL_OUTPUT_CHARS + 500);
        let ok_val = json!({
            "content": [{"type": "text", "text": big}]
        });
        let out = format_tool_call_result("read_big", &ok_val).unwrap();
        assert!(out.contains("MCP tool output truncated"));
        assert!(out.len() < MAX_MCP_TOOL_OUTPUT_CHARS + 200);
    }
}
