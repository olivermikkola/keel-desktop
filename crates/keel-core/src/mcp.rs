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
    pub tools: Vec<McpToolInfo>,
    pub error: Option<String>,
}

pub fn global_mcp_config_path(config_dir: &Path) -> PathBuf {
    config_dir.join("mcp.json")
}

pub fn load_mcp_configs(config_dir: &Path, workspace: Option<&Path>) -> McpConfigFile {
    let mut merged = McpConfigFile::default();
    let global = global_mcp_config_path(config_dir);
    if let Ok(raw) = std::fs::read_to_string(global) {
        if let Ok(parsed) = serde_json::from_str::<McpConfigFile>(&raw) {
            merged.mcp_servers.extend(parsed.mcp_servers);
        }
    }
    if let Some(ws) = workspace {
        let local = ws.join(".keel/mcp.json");
        if let Ok(raw) = std::fs::read_to_string(local) {
            if let Ok(parsed) = serde_json::from_str::<McpConfigFile>(&raw) {
                merged.mcp_servers.extend(parsed.mcp_servers);
            }
        }
    }
    merged
}

pub fn save_global_mcp_config(config_dir: &Path, cfg: &McpConfigFile) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let path = global_mcp_config_path(config_dir);
    let body = serde_json::to_string_pretty(cfg)?;
    std::fs::write(path, body)?;
    Ok(())
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
            .stderr(Stdio::null());
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
        session.initialize().await?;
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
            let mut buf = String::new();
            loop {
                buf.clear();
                let read = self.stdout.read_line(&mut buf).await?;
                if read == 0 {
                    bail!("MCP server closed stdout unexpectedly");
                }
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
        if let Some(arr) = res.get("content").and_then(|c| c.as_array()) {
            let texts: Vec<String> = arr
                .iter()
                .filter_map(|item| item.get("text").and_then(|t| t.as_str()).map(String::from))
                .collect();
            if !texts.is_empty() {
                return Ok(texts.join("\n"));
            }
        }
        Ok(serde_json::to_string_pretty(&res)?)
    }

    async fn close(mut self) {
        let _ = self.child.kill().await;
    }
}

/// Discovers tools from all configured MCP servers.
pub async fn inspect_mcp_servers(
    cfg: &McpConfigFile,
    workspace: Option<&Path>,
) -> Vec<McpServerStatus> {
    let mut statuses = Vec::new();
    for (name, server) in &cfg.mcp_servers {
        if !server.enabled {
            statuses.push(McpServerStatus {
                name: name.clone(),
                command: server.command.clone(),
                args: server.args.clone(),
                enabled: false,
                tools: Vec::new(),
                error: None,
            });
            continue;
        }
        match McpSession::spawn(server, workspace).await {
            Ok(mut session) => {
                let res = session.list_tools(name).await;
                session.close().await;
                match res {
                    Ok(tools) => statuses.push(McpServerStatus {
                        name: name.clone(),
                        command: server.command.clone(),
                        args: server.args.clone(),
                        enabled: true,
                        tools,
                        error: None,
                    }),
                    Err(e) => statuses.push(McpServerStatus {
                        name: name.clone(),
                        command: server.command.clone(),
                        args: server.args.clone(),
                        enabled: true,
                        tools: Vec::new(),
                        error: Some(e.to_string()),
                    }),
                }
            }
            Err(e) => statuses.push(McpServerStatus {
                name: name.clone(),
                command: server.command.clone(),
                args: server.args.clone(),
                enabled: true,
                tools: Vec::new(),
                error: Some(e.to_string()),
            }),
        }
    }
    statuses.sort_by(|a, b| a.name.cmp(&b.name));
    statuses
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
}
