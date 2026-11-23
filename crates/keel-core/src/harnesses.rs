use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::watch;

use crate::agent::{AgentEvent, ApprovalDecision};
use crate::sessions::UiMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessOption {
    pub id: String,
    pub name: String,
    pub description: String,
    pub available: bool,
    pub binary_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeExecutionBoundary {
    pub can_run_commands: bool,
    pub sandbox: String,
    pub notice: Option<String>,
}

/// Evaluate whether Code mode may run shell commands on this platform.
/// On macOS, command execution requires both Seatbelt (`/usr/bin/sandbox-exec`)
/// and an installed Keel CLI; otherwise Code mode stays in inspect-and-propose mode.
pub fn code_execution_boundary() -> CodeExecutionBoundary {
    let has_sandbox_exec = Path::new("/usr/bin/sandbox-exec").is_file();
    let has_keel_cli = find_on_path("keel").is_some();
    code_execution_boundary_for(std::env::consts::OS, has_sandbox_exec, has_keel_cli)
}

pub fn code_execution_boundary_for(
    os: &str,
    has_sandbox_exec: bool,
    has_keel_cli: bool,
) -> CodeExecutionBoundary {
    match os {
        "linux" => CodeExecutionBoundary {
            can_run_commands: true,
            sandbox: "landlock".into(),
            notice: None,
        },
        "macos" if has_sandbox_exec && has_keel_cli => CodeExecutionBoundary {
            can_run_commands: true,
            sandbox: "seatbelt".into(),
            notice: None,
        },
        "macos" => CodeExecutionBoundary {
            can_run_commands: false,
            sandbox: "none".into(),
            notice: Some(
                "Shell commands are off on this Mac until a confined Keel CLI is installed. You can still inspect files and propose edits."
                    .into(),
            ),
        },
        _ => CodeExecutionBoundary {
            can_run_commands: true,
            sandbox: "approval-only".into(),
            notice: Some(
                "Shell commands require explicit per-command approval on this platform.".into(),
            ),
        },
    }
}

/// Lists built-in Keel Harness plus any installed external CLI coding agents.
pub fn detect_harnesses() -> Vec<HarnessOption> {
    let mut list = vec![HarnessOption {
        id: "keel".to_string(),
        name: "Keel Harness".to_string(),
        description:
            "Built-in small-model engine with staged diffs, SQLite/CSV sandbox, and stdio MCP"
                .to_string(),
        available: true,
        binary_path: None,
    }];

    let externals = [
        (
            "opencode",
            "OpenCode",
            "Route workspace prompts through the installed `opencode` CLI",
        ),
        (
            "keel-cli",
            "Keel CLI",
            "Route workspace prompts through the standalone `keel` CLI binary",
        ),
        (
            "claude",
            "Claude Code",
            "Route workspace prompts through the installed `claude` CLI",
        ),
    ];

    for (id, name, desc) in externals {
        let bin = match id {
            "keel-cli" => find_on_path("keel"),
            other => find_on_path(other),
        };
        list.push(HarnessOption {
            id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            available: bin.is_some(),
            binary_path: bin.map(|p| p.to_string_lossy().to_string()),
        });
    }

    list
}

fn find_on_path(bin: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        format!("{bin}.exe")
    } else {
        bin.to_string()
    };
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(&exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        for extra in [".local/bin", ".cargo/bin", ".bun/bin"] {
            let cand = home.join(extra).join(&exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

pub struct KeelCliTurn<'a> {
    pub root: &'a Path,
    pub log: &'a Path,
    pub prompt: &'a str,
    pub base_url: &'a str,
    pub model: &'a str,
}

const MAX_EVENT_BYTES: u64 = 1024 * 1024;
const TURN_TIMEOUT: Duration = Duration::from_secs(300);
const EXIT_GRACE: Duration = Duration::from_secs(5);

/// Project completed text messages from a user-selected Keel journal for display.
/// The CLI continues the original journal, including tool observations.
pub fn read_keel_session(path: &Path) -> Result<Vec<UiMessage>> {
    use std::io::{BufRead, Read};

    const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
    let file = std::fs::File::open(path).context("Cannot open Keel session log")?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_JOURNAL_BYTES {
        bail!("Select a regular Keel JSONL file smaller than 64 MB");
    }
    let mut reader = std::io::BufReader::new(file.take(MAX_JOURNAL_BYTES + 1));
    let mut messages = Vec::new();
    let mut line = Vec::new();
    let mut read = 0u64;
    loop {
        line.clear();
        let n = (&mut reader)
            .take(MAX_EVENT_BYTES + 1)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if n as u64 > MAX_EVENT_BYTES || read > MAX_JOURNAL_BYTES {
            bail!("Keel session log exceeded the read limit");
        }
        let entry: Value =
            serde_json::from_slice(&line).context("Invalid entry in Keel session log")?;
        let kind = entry["kind"]
            .as_str()
            .context("File is not a Keel session log")?;
        if !matches!(kind, "user" | "assistant") {
            continue;
        }
        let text = entry["data"]["text"]
            .as_str()
            .context("Invalid conversation message in Keel session log")?;
        if text.trim().is_empty() {
            continue;
        }
        messages.push(UiMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: kind.into(),
            content: text.into(),
            thinking: None,
            tool_calls: vec![],
            attachments: vec![],
            status: "complete".into(),
        });
    }
    if !messages.iter().any(|message| message.role == "user") {
        bail!("Keel session log has no conversation to open");
    }
    Ok(messages)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KeelEvent {
    Capabilities {
        approval_protocol: String,
        stop_protocol: Option<String>,
        #[serde(default)]
        sandbox: Option<String>,
    },
    Text {
        content: String,
    },
    Reasoning {
        content: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: Value,
    },
    ToolResult {
        id: String,
        output: String,
    },
    ApprovalRequest {
        protocol: String,
        id: String,
        tool_call_id: String,
        tool: String,
        arguments: Value,
    },
    Done {
        status: String,
        answer: Option<String>,
        error: Option<String>,
    },
    #[serde(other)]
    Other,
}

/// Run the canonical CLI once against its original session log. Closing stdin
/// denies an outstanding approval, and SIGINT lets Keel reap an active shell job.
pub async fn run_keel_cli_turn<E, A, F>(
    turn: KeelCliTurn<'_>,
    mut stop: watch::Receiver<bool>,
    emit: E,
    approve: A,
) -> Result<Option<String>>
where
    E: FnMut(AgentEvent),
    A: FnMut(String, String, String, Value) -> F,
    F: std::future::Future<Output = ApprovalDecision>,
{
    if !cfg!(unix) {
        bail!("Keel CLI integration is unavailable on this platform");
    }
    let binary = find_on_path("keel")
        .context("Install the current Keel CLI and put it on PATH to use Code")?;
    let root = crate::tools::validate_workspace_root(turn.root)?;
    let mut command = Command::new(binary);
    command
        .current_dir(&root)
        .args(["run", "--json", "--safety-mode", "safe", "--root"])
        .arg(&root)
        .arg("--log")
        .arg(turn.log)
        .args(["--base-url", turn.base_url, "--model", turn.model])
        .env_remove("KEEL_API_KEY")
        .env_remove("DEEPSEEK_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    match std::fs::metadata(turn.log) {
        Ok(meta) if !meta.is_file() || meta.len() == 0 => {
            bail!("Keel's session log is empty or invalid")
        }
        Ok(_) => {
            command.arg("--resume").arg(turn.log);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("Reading Keel's session log"),
    }
    command.arg("--").arg(turn.prompt);
    let mut child = command.spawn().context("Starting the Keel CLI")?;
    let mut stdin = child
        .stdin
        .take()
        .context("Keel CLI stdin is unavailable")?;
    let stdout = child
        .stdout
        .take()
        .context("Keel CLI stdout is unavailable")?;
    let mut stderr = child
        .stderr
        .take()
        .context("Keel CLI stderr is unavailable")?;
    let drain =
        tokio::spawn(async move { tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await });
    let result = tokio::select! {
        biased;
        _ = stop.wait_for(|stopped| *stopped) => Ok(None),
        response = tokio::time::timeout(TURN_TIMEOUT, read_keel_turn(stdout, &mut stdin, emit, approve)) => {
            match response {
                Ok(result) => result.map(Some),
                Err(_) => Err(anyhow::anyhow!("The Keel CLI turn timed out")),
            }
        }
    };

    #[cfg(unix)]
    if !matches!(result, Ok(Some(_))) {
        if let Some(pid) = child.id() {
            // The Child still owns this PID until wait or kill completes.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGINT) };
        }
    }
    drop(stdin);
    let status = match tokio::time::timeout(EXIT_GRACE, child.wait()).await {
        Ok(status) => status?,
        Err(_) => {
            child.kill().await.context("Stopping the Keel CLI")?;
            drain.abort();
            return result.and_then(|answer| {
                if answer.is_some() {
                    bail!("Keel CLI did not exit after reporting a result");
                }
                Ok(None)
            });
        }
    };
    let _ = tokio::time::timeout(EXIT_GRACE, drain).await;
    if matches!(result, Ok(Some(_))) && !status.success() {
        bail!("Keel CLI exited unsuccessfully ({status})");
    }
    result
}

async fn read_keel_turn<E, A, F>(
    stdout: ChildStdout,
    stdin: &mut ChildStdin,
    mut emit: E,
    mut approve: A,
) -> Result<String>
where
    E: FnMut(AgentEvent),
    A: FnMut(String, String, String, Value) -> F,
    F: std::future::Future<Output = ApprovalDecision>,
{
    use std::collections::HashSet;
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    let mut ready = false;
    let mut tools = HashMap::<String, String>::new();
    let mut approved_requests = HashSet::new();
    loop {
        line.clear();
        let size = (&mut reader)
            .take(MAX_EVENT_BYTES + 1)
            .read_until(b'\n', &mut line)
            .await?;
        if size == 0 {
            bail!("Keel CLI closed its event stream without finishing the turn");
        }
        if size as u64 > MAX_EVENT_BYTES {
            bail!("Keel CLI event exceeded the size limit");
        }
        let event: KeelEvent =
            serde_json::from_slice(&line).context("Invalid event from the Keel CLI")?;
        if !ready {
            match event {
                KeelEvent::Capabilities {
                    approval_protocol,
                    stop_protocol,
                    sandbox,
                } if approval_protocol == "stdio-line-v1"
                    && stop_protocol.as_deref() == Some("sigint-shell-v1") =>
                {
                    if cfg!(target_os = "macos") && sandbox.as_deref() != Some("seatbelt") {
                        bail!("Update Keel CLI. This Mac build requires Seatbelt command confinement before running Code commands.");
                    }
                    ready = true;
                    continue;
                }
                KeelEvent::Done { error: Some(error), .. } => bail!("{error}"),
                _ => bail!("Update Keel CLI. This version does not advertise interactive approvals and command cancellation"),
            }
        }
        match event {
            KeelEvent::Text { content } => emit(AgentEvent::ContentDelta { text: content }),
            KeelEvent::Reasoning { content } => emit(AgentEvent::ThinkingDelta { text: content }),
            KeelEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                tools.insert(id, name.clone());
                emit(AgentEvent::ToolCallStart { name, arguments });
            }
            KeelEvent::ToolResult { id, output } => {
                if let Some(name) = tools.remove(&id) {
                    let failed = output.starts_with("error:") || output.starts_with("[Denied");
                    emit(AgentEvent::ToolCallResult {
                        name,
                        result: output,
                        is_error: failed,
                        edit_id: None,
                    });
                }
            }
            KeelEvent::ApprovalRequest {
                protocol,
                id,
                tool_call_id,
                tool,
                arguments,
            } => {
                if protocol != "stdio-line-v1"
                    || tools.get(&tool_call_id) != Some(&tool)
                    || !approved_requests.insert(id.clone())
                {
                    bail!("Unexpected approval request from the Keel CLI");
                }
                let detail = arguments
                    .get("command")
                    .or_else(|| arguments.get("path"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let summary = format!("{tool} {detail}");
                let decision = approve(id, tool, summary, arguments).await;
                // Keep CLI approvals per call. Desktop never grants an entire tool class.
                let line = if decision == ApprovalDecision::Deny {
                    b"deny\n".as_slice()
                } else {
                    b"allow\n".as_slice()
                };
                stdin.write_all(line).await?;
                stdin.flush().await?;
                if decision == ApprovalDecision::Deny {
                    if let Some(name) = tools.remove(&tool_call_id) {
                        emit(AgentEvent::ToolCallResult {
                            name,
                            result: "Denied by you. The tool did not run.".into(),
                            is_error: true,
                            edit_id: None,
                        });
                    }
                }
            }
            KeelEvent::Done {
                status,
                answer,
                error,
            } => {
                if status == "ok" {
                    return answer.context("Keel CLI finished without a result");
                }
                bail!(
                    "{}",
                    error
                        .as_deref()
                        .unwrap_or("Keel CLI reported a failed turn")
                );
            }
            _ => {}
        }
    }
}

async fn read_capped_and_drain<R: tokio::io::AsyncRead + Unpin>(
    mut reader: Option<R>,
    limit: usize,
) -> Vec<u8> {
    let Some(ref mut r) = reader else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() < limit {
                    let take = (limit - out.len()).min(n);
                    out.extend_from_slice(&chunk[..take]);
                }
            }
        }
    }
    out
}

/// Runs a single prompt through a supported external CLI harness in `workspace`.
pub async fn run_external_harness(
    harness_id: &str,
    workspace: &Path,
    prompt: &str,
    base_url: Option<&str>,
    model: Option<&str>,
) -> Result<String> {
    let workspace = crate::tools::validate_workspace_root(workspace)?;
    let mut cmd = match harness_id {
        "opencode" => {
            let bin = find_on_path("opencode").context("`opencode` is not installed on PATH")?;
            let mut c = Command::new(bin);
            c.arg("run").arg("--").arg(prompt);
            c
        }
        "keel-cli" => {
            let bin = find_on_path("keel").context("`keel` CLI is not installed on PATH")?;
            let mut c = Command::new(bin);
            if let Some(url) = base_url {
                c.arg("--base-url").arg(url);
            }
            if let Some(m) = model {
                c.arg("--model").arg(m);
            }
            c.arg("--").arg(prompt);
            c
        }
        "claude" => {
            let bin = find_on_path("claude").context("`claude` CLI is not installed on PATH")?;
            let mut c = Command::new(bin);
            c.arg("-p").arg("--").arg(prompt);
            c
        }
        other => bail!("Unsupported external harness: {other}"),
    };

    cmd.current_dir(&workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().context("Failed to launch external harness")?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let res = tokio::time::timeout(Duration::from_secs(120), async {
        let (out, err, status) = tokio::join!(
            read_capped_and_drain(stdout, 128 * 1024),
            read_capped_and_drain(stderr, 32 * 1024),
            child.wait()
        );
        let status = status?;
        Ok::<_, anyhow::Error>((status, out, err))
    })
    .await;

    match res {
        Ok(inner) => {
            let (status, out, err) = inner?;
            let out_str = String::from_utf8_lossy(&out).trim().to_string();
            let err_str = String::from_utf8_lossy(&err).trim().to_string();
            if !status.success() && out_str.is_empty() {
                bail!("External harness failed ({status}): {err_str}");
            }
            if out_str.is_empty() {
                Ok(err_str)
            } else {
                Ok(out_str)
            }
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            bail!("External harness timed out after 120s")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_builtin_keel_harness() {
        let list = detect_harnesses();
        assert!(!list.is_empty());
        assert_eq!(list[0].id, "keel");
        assert!(list[0].available);
    }

    #[test]
    fn macos_without_confined_cli_disables_commands_and_explains_limit() {
        let mac_unverified = code_execution_boundary_for("macos", true, false);
        assert!(!mac_unverified.can_run_commands);
        assert_eq!(mac_unverified.sandbox, "none");
        assert!(mac_unverified
            .notice
            .as_deref()
            .is_some_and(|n| n.contains("inspect files and propose edits")));

        let mac_no_seatbelt = code_execution_boundary_for("macos", false, true);
        assert!(!mac_no_seatbelt.can_run_commands);

        let mac_verified = code_execution_boundary_for("macos", true, true);
        assert!(mac_verified.can_run_commands);
        assert_eq!(mac_verified.sandbox, "seatbelt");
        assert!(mac_verified.notice.is_none());

        let linux = code_execution_boundary_for("linux", false, false);
        assert!(linux.can_run_commands);
        assert_eq!(linux.sandbox, "landlock");
    }
}
