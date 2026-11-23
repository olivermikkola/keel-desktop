use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Command;
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
/// On Windows there is no landlock/seatbelt; commands may run with approvals only
/// and Desktop prefers the Keel CLI when it is installed.
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
        "windows" => {
            // Honest: Windows has no Desktop-enforced OS sandbox equivalent to
            // landlock/seatbelt. Prefer Keel CLI (stdio approvals) when present.
            let notice = if has_keel_cli {
                "Windows has no landlock or Seatbelt confinement. Prefer Keel CLI for Code (stdio approvals); shell commands still need your approval and are not OS-sandboxed."
            } else {
                "Windows has no landlock or Seatbelt confinement. Install the Keel CLI for Code when possible; shell commands need your approval and are not OS-sandboxed."
            };
            CodeExecutionBoundary {
                can_run_commands: true,
                sandbox: "none".into(),
                notice: Some(notice.into()),
            }
        }
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
///
/// When the standalone `keel` binary is on PATH, **Keel CLI** is listed
/// immediately after the built-in harness and marked as the recommended Code
/// engine (canonical vision: one coding engine).
pub fn detect_harnesses() -> Vec<HarnessOption> {
    let mut list = vec![HarnessOption {
        id: "keel".to_string(),
        name: "Keel Harness".to_string(),
        description:
            "Built-in small-model engine with staged diffs, SQLite/CSV sandbox, and stdio MCP. Default until the Keel CLI is installed."
                .to_string(),
        available: true,
        binary_path: None,
    }];

    let keel_bin = find_on_path("keel");
    list.push(HarnessOption {
        id: "keel-cli".to_string(),
        name: "Keel CLI".to_string(),
        description: if keel_bin.is_some() {
            "Recommended for Code. Streams tools, requests approvals in the app, and resumes the same JSONL journal in the terminal."
                .to_string()
        } else {
            "Canonical coding engine. Install the `keel` CLI on PATH to use it from Code."
                .to_string()
        },
        available: keel_bin.is_some(),
        binary_path: keel_bin.map(|p| p.to_string_lossy().to_string()),
    });

    let others = [
        (
            "opencode",
            "OpenCode",
            "Route workspace prompts through the installed `opencode` CLI",
        ),
        (
            "claude",
            "Claude Code",
            "Route workspace prompts through the installed `claude` CLI",
        ),
    ];

    for (id, name, desc) in others {
        let bin = find_on_path(id);
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

/// Prefer the Keel CLI for Code when it is installed; otherwise the built-in harness.
pub fn recommended_code_harness(harnesses: &[HarnessOption]) -> &'static str {
    if harnesses.iter().any(|h| h.id == "keel-cli" && h.available) {
        "keel-cli"
    } else {
        "keel"
    }
}

fn windows_exe_names(bin: &str) -> Vec<String> {
    vec![
        format!("{bin}.exe"),
        format!("{bin}.cmd"),
        format!("{bin}.bat"),
        bin.to_string(),
    ]
}

fn first_existing_in_dir(dir: &Path, names: &[String]) -> Option<PathBuf> {
    for name in names {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn find_on_path(bin: &str) -> Option<PathBuf> {
    let names = if cfg!(target_os = "windows") {
        windows_exe_names(bin)
    } else {
        vec![bin.to_string()]
    };
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if let Some(found) = first_existing_in_dir(&dir, &names) {
                return Some(found);
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        for extra in [".local/bin", ".cargo/bin", ".bun/bin"] {
            if let Some(found) = first_existing_in_dir(&home.join(extra), &names) {
                return Some(found);
            }
        }
        // Typical Windows install / package-manager shim locations outside PATH.
        if cfg!(target_os = "windows") {
            for extra in [
                PathBuf::from(r"scoop\shims"),
                PathBuf::from(r".keel\bin"),
                PathBuf::from(r"AppData\Local\Microsoft\WinGet\Links"),
                PathBuf::from(r"AppData\Local\Programs\keel"),
                PathBuf::from(r"AppData\Local\keel\bin"),
            ] {
                if let Some(found) = first_existing_in_dir(&home.join(extra), &names) {
                    return Some(found);
                }
            }
        }
    }
    if cfg!(target_os = "windows") {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            for extra in [
                PathBuf::from(r"Microsoft\WinGet\Links"),
                PathBuf::from(r"Programs\keel"),
                PathBuf::from(r"keel\bin"),
            ] {
                if let Some(found) = first_existing_in_dir(&local.join(extra), &names) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Default Keel CLI journal folder (`~/.keel/sessions`) when it exists.
pub fn default_keel_sessions_dir() -> Option<std::path::PathBuf> {
    let sessions = keel_sessions_dir()?;
    sessions.is_dir().then_some(sessions)
}

/// Path of the Keel CLI journal folder (`~/.keel/sessions`), whether or not it exists yet.
pub fn keel_sessions_dir() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join(".keel").join("sessions"))
}

/// Create `~/.keel/sessions` if needed so Desktop can export a journal for the CLI.
pub fn ensure_keel_sessions_dir() -> Result<PathBuf> {
    let sessions = keel_sessions_dir().context("Home directory is unavailable")?;
    std::fs::create_dir_all(&sessions).context("Creating ~/.keel/sessions")?;
    Ok(sessions)
}

const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;

/// Confirm a path is a regular Keel journal file before revealing or exporting it.
pub fn validate_keel_session_log(path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("Cannot find Keel session journal at {}", path.display()))?;
    let meta = std::fs::metadata(&path).context("Cannot read Keel session journal")?;
    if !meta.is_file() {
        bail!("Keel session journal is not a regular file");
    }
    if meta.len() > MAX_JOURNAL_BYTES {
        bail!("Keel session journal is larger than 64 MB");
    }
    Ok(path)
}

/// Build a filesystem-safe basename for an exported Desktop journal copy.
pub fn safe_journal_export_name(session_id: &str, title: &str) -> String {
    let id: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(36)
        .collect();
    let id = if id.is_empty() {
        "session".to_string()
    } else {
        id
    };
    let title_part: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .take(40)
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if title_part.is_empty() {
        format!("keel-desktop-{id}.jsonl")
    } else {
        format!("keel-desktop-{id}-{title_part}.jsonl")
    }
}

/// Copy a validated journal into `~/.keel/sessions` under a safe name.
pub fn export_keel_session_log(source: &Path, session_id: &str, title: &str) -> Result<PathBuf> {
    let source = validate_keel_session_log(source)?;
    let dest_dir = ensure_keel_sessions_dir()?;
    let dest = dest_dir.join(safe_journal_export_name(session_id, title));
    if source == dest {
        return Ok(dest);
    }
    std::fs::copy(&source, &dest).context("Copying Keel session journal")?;
    Ok(dest)
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
        match kind {
            "user" | "assistant" => {
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
            "tool" => {
                let Some(log) = tool_log_from_journal_data(&entry["data"]) else {
                    continue;
                };
                if let Some(message) = messages.iter_mut().rev().find(|m| m.role == "assistant") {
                    message.tool_calls.push(log);
                } else {
                    messages.push(UiMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        role: "assistant".into(),
                        content: String::new(),
                        thinking: None,
                        tool_calls: vec![log],
                        attachments: vec![],
                        status: "complete".into(),
                    });
                }
            }
            _ => continue,
        }
    }
    if !messages.iter().any(|message| message.role == "user") {
        bail!("Keel session log has no conversation to open");
    }
    Ok(messages)
}

fn tool_log_from_journal_data(data: &Value) -> Option<crate::sessions::UiToolLog> {
    let name = data
        .get("name")
        .or_else(|| data.get("tool"))
        .and_then(Value::as_str)?
        .to_string();
    if name.trim().is_empty() {
        return None;
    }
    let arguments = data
        .get("arguments")
        .or_else(|| data.get("args"))
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let result = data
        .get("output")
        .or_else(|| data.get("result"))
        .and_then(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| Some(v.to_string()))
        });
    let is_error = result
        .as_deref()
        .is_some_and(|r| r.starts_with("error:") || r.starts_with("[Denied"));
    Some(crate::sessions::UiToolLog {
        name,
        arguments,
        result,
        is_error,
        edit_id: None,
    })
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KeelEvent {
    Capabilities {
        approval_protocol: String,
        stop_protocol: Option<String>,
        #[serde(default)]
        sandbox: Option<String>,
        /// Present on current Keel CLI builds; ignored for handshake.
        #[serde(default)]
        context_window: Option<u64>,
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
    SentinelVeto {
        tool_name: String,
        reason: String,
    },
    TurnSummary {
        summary: Value,
    },
    Done {
        status: String,
        answer: Option<String>,
        error: Option<String>,
    },
    #[serde(other)]
    Other,
}

fn turn_summary_note(summary: &Value) -> String {
    let files = summary
        .get("files")
        .and_then(Value::as_u64)
        .or_else(|| summary.get("files_touched").and_then(Value::as_u64));
    let tools = summary.get("tools").and_then(Value::as_u64);
    let tokens = summary
        .get("tokens")
        .and_then(Value::as_u64)
        .or_else(|| summary.get("total_tokens").and_then(Value::as_u64));
    let mut parts = Vec::new();
    if let Some(n) = tools {
        parts.push(format!("{n} tool call{}", if n == 1 { "" } else { "s" }));
    }
    if let Some(n) = files {
        parts.push(format!("{n} file{}", if n == 1 { "" } else { "s" }));
    }
    if let Some(n) = tokens {
        parts.push(format!("{n} tokens"));
    }
    if parts.is_empty() {
        "Keel CLI turn finished.".into()
    } else {
        format!("Keel CLI turn: {}.", parts.join(", "))
    }
}

fn parse_keel_event_line(line: &[u8]) -> Result<KeelEvent> {
    serde_json::from_slice(line).context("Invalid event from the Keel CLI")
}

fn keel_capabilities_ready(event: &KeelEvent) -> Result<bool> {
    match event {
        KeelEvent::Capabilities {
            approval_protocol,
            stop_protocol,
            sandbox,
            context_window: _,
        } if approval_protocol == "stdio-line-v1"
            && stop_protocol.as_deref() == Some("sigint-shell-v1") =>
        {
            if cfg!(target_os = "macos") && sandbox.as_deref() != Some("seatbelt") {
                bail!("Update Keel CLI. This Mac build requires Seatbelt command confinement before running Code commands.");
            }
            Ok(true)
        }
        KeelEvent::Done {
            error: Some(error), ..
        } => bail!("{error}"),
        _ => bail!(
            "Update Keel CLI. This version does not advertise interactive approvals and command cancellation"
        ),
    }
}

fn approval_stdin_line(decision: ApprovalDecision) -> &'static [u8] {
    // Keep CLI approvals per call. Desktop never grants an entire tool class
    // through the stdio-line-v1 protocol (AllowAlways still maps to one allow).
    if decision == ApprovalDecision::Deny {
        b"deny\n"
    } else {
        b"allow\n"
    }
}

/// Run the canonical CLI once against its original session log. Closing stdin
/// denies an outstanding approval. On Unix, SIGINT lets Keel reap an active
/// shell job (`sigint-shell-v1`); on Windows, stop falls through to a grace
/// wait then process kill (no invented OS confinement).
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
    // Avoid flashing a console window when Desktop (GUI) spawns the CLI.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
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

/// Drive a Keel-CLI-shaped NDJSON event stream through Desktop's projection
/// (capabilities handshake → text/tool/approval → done). Production wires this
/// to a spawned `keel` process; tests use a fake CLI over duplex pipes.
async fn read_keel_turn<R, W, E, A, F>(
    stdout: R,
    stdin: &mut W,
    mut emit: E,
    mut approve: A,
) -> Result<String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
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
        let event = parse_keel_event_line(&line)?;
        if !ready {
            if keel_capabilities_ready(&event)? {
                ready = true;
            }
            continue;
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
            KeelEvent::SentinelVeto { tool_name, reason } => {
                tools.retain(|_, name| name != &tool_name);
                emit(AgentEvent::ToolCallResult {
                    name: tool_name,
                    result: format!("Blocked by Keel security: {reason}"),
                    is_error: true,
                    edit_id: None,
                });
            }
            KeelEvent::TurnSummary { summary } => {
                emit(AgentEvent::Compacted {
                    note: turn_summary_note(&summary),
                });
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
                stdin.write_all(approval_stdin_line(decision)).await?;
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
    // Keel CLI must use run_keel_cli_turn (streaming JSON + approvals), never this one-shot path.
    let _ = (base_url, model);
    let mut cmd = match harness_id {
        "opencode" => {
            let bin = find_on_path("opencode").context("`opencode` is not installed on PATH")?;
            let mut c = Command::new(bin);
            c.arg("run").arg("--").arg(prompt);
            c
        }
        "keel-cli" => bail!(
            "Use Code with Engine for Code mode set to Keel CLI. The one-shot CLI path was removed."
        ),
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
    fn always_includes_builtin_then_keel_cli_option() {
        let list = detect_harnesses();
        assert!(list.len() >= 2);
        assert_eq!(list[0].id, "keel");
        assert!(list[0].available);
        assert_eq!(list[1].id, "keel-cli");
    }

    #[tokio::test]
    async fn keel_cli_one_shot_path_is_removed() {
        let dir = std::env::temp_dir().join(format!("keel_cli_oneshot_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let err = run_external_harness("keel-cli", &dir, "hi", None, None)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("Keel CLI") || err.contains("one-shot"),
            "unexpected error: {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recommends_keel_cli_only_when_installed() {
        let without = vec![HarnessOption {
            id: "keel".into(),
            name: "Keel Harness".into(),
            description: String::new(),
            available: true,
            binary_path: None,
        }];
        assert_eq!(recommended_code_harness(&without), "keel");

        let with_cli = vec![
            without[0].clone(),
            HarnessOption {
                id: "keel-cli".into(),
                name: "Keel CLI".into(),
                description: String::new(),
                available: true,
                binary_path: Some("/usr/bin/keel".into()),
            },
        ];
        assert_eq!(recommended_code_harness(&with_cli), "keel-cli");
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

    #[test]
    fn windows_boundary_is_honest_about_missing_os_sandbox() {
        let without_cli = code_execution_boundary_for("windows", false, false);
        assert!(without_cli.can_run_commands);
        assert_eq!(without_cli.sandbox, "none");
        let notice = without_cli.notice.as_deref().expect("windows notice");
        assert!(
            notice.contains("no landlock or Seatbelt"),
            "notice should state missing OS sandboxes: {notice}"
        );
        assert!(
            notice.contains("Keel CLI"),
            "notice should recommend Keel CLI: {notice}"
        );
        assert!(
            notice.contains("not OS-sandboxed"),
            "notice must not claim a sandbox: {notice}"
        );

        let with_cli = code_execution_boundary_for("windows", false, true);
        assert!(with_cli.can_run_commands);
        assert_eq!(with_cli.sandbox, "none");
        let notice = with_cli.notice.as_deref().expect("windows notice with CLI");
        assert!(
            notice.contains("Prefer Keel CLI"),
            "when CLI is present, prefer it: {notice}"
        );
        assert!(notice.contains("not OS-sandboxed"));
        assert_eq!(with_cli.sandbox, "none");
    }

    #[test]
    fn windows_exe_name_candidates_include_common_suffixes() {
        let names = windows_exe_names("keel");
        assert_eq!(
            names,
            vec![
                "keel.exe".to_string(),
                "keel.cmd".to_string(),
                "keel.bat".to_string(),
                "keel".to_string(),
            ]
        );
    }

    #[test]
    fn parses_streaming_cli_events_and_capabilities() {
        let sandbox = if cfg!(target_os = "macos") {
            "seatbelt"
        } else {
            "landlock"
        };
        let caps_line = format!(
            r#"{{"type":"capabilities","approval_protocol":"stdio-line-v1","stop_protocol":"sigint-shell-v1","sandbox":"{sandbox}"}}"#
        );
        let caps = parse_keel_event_line(caps_line.as_bytes()).unwrap();
        assert!(keel_capabilities_ready(&caps).unwrap());

        let text = parse_keel_event_line(br#"{"type":"text","content":"hi"}"#).unwrap();
        assert_eq!(
            text,
            KeelEvent::Text {
                content: "hi".into()
            }
        );

        let tool = parse_keel_event_line(
            br#"{"type":"tool_call","id":"1","name":"run_shell","arguments":{"command":"ls"}}"#,
        )
        .unwrap();
        assert!(matches!(tool, KeelEvent::ToolCall { name, .. } if name == "run_shell"));

        let done =
            parse_keel_event_line(br#"{"type":"done","status":"ok","answer":"done","error":null}"#)
                .unwrap();
        assert!(matches!(done, KeelEvent::Done { status, .. } if status == "ok"));
    }

    #[test]
    fn rejects_cli_without_interactive_approval_protocol() {
        let bad = parse_keel_event_line(br#"{"type":"text","content":"too soon"}"#).unwrap();
        let err = keel_capabilities_ready(&bad).unwrap_err().to_string();
        assert!(err.contains("Update Keel CLI"));
    }

    #[test]
    fn maps_approval_decisions_to_stdio_lines() {
        assert_eq!(approval_stdin_line(ApprovalDecision::AllowOnce), b"allow\n");
        assert_eq!(
            approval_stdin_line(ApprovalDecision::AllowAlways),
            b"allow\n"
        );
        assert_eq!(approval_stdin_line(ApprovalDecision::Deny), b"deny\n");
    }

    #[test]
    fn read_keel_session_projects_tool_observations_onto_assistant() {
        let dir = std::env::temp_dir().join(format!("keel_journal_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.jsonl");
        std::fs::write(
            &path,
            r#"{"kind":"user","data":{"text":"list files"}}
{"kind":"assistant","data":{"text":"Looking around."}}
{"kind":"tool","data":{"name":"list_files","arguments":{"path":"."},"output":"src\n"}}
{"kind":"assistant","data":{"text":"There is a src folder."}}
"#,
        )
        .unwrap();
        let messages = read_keel_session(&path).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[1].tool_calls.len(), 1);
        assert_eq!(messages[1].tool_calls[0].name, "list_files");
        assert_eq!(messages[1].tool_calls[0].result.as_deref(), Some("src\n"));
        assert_eq!(messages[2].content, "There is a src folder.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_keel_session_requires_a_user_turn() {
        let dir = std::env::temp_dir().join(format!("keel_journal_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.jsonl");
        std::fs::write(&path, r#"{"kind":"assistant","data":{"text":"hi"}}"#).unwrap();
        let err = read_keel_session(&path).unwrap_err().to_string();
        assert!(err.contains("no conversation"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn turn_summary_and_sentinel_events_parse() {
        let summary = parse_keel_event_line(
            br#"{"type":"turn_summary","summary":{"tools":2,"files":1,"tokens":40}}"#,
        )
        .unwrap();
        match summary {
            KeelEvent::TurnSummary { summary } => {
                assert_eq!(
                    turn_summary_note(&summary),
                    "Keel CLI turn: 2 tool calls, 1 file, 40 tokens."
                );
            }
            other => panic!("unexpected {other:?}"),
        }
        let veto = parse_keel_event_line(
            br#"{"type":"sentinel_veto","tool_name":"shell","reason":"blocked"}"#,
        )
        .unwrap();
        assert!(matches!(
            veto,
            KeelEvent::SentinelVeto { tool_name, reason }
                if tool_name == "shell" && reason == "blocked"
        ));
    }

    #[test]
    fn streaming_protocol_has_no_file_edit_event_for_diff_projection() {
        // Checklist: do not invent Diff projection until Keel CLI emits a dedicated
        // file-edit / patch event type. Unknown types stay KeelEvent::Other.
        // Diff/--shadow parity cannot be asserted until that protocol lands.
        let other = parse_keel_event_line(
            br#"{"type":"file_edit","path":"src/main.rs","diff":"--- a\n+++ b\n"}"#,
        )
        .unwrap();
        assert_eq!(other, KeelEvent::Other);
        let patch = parse_keel_event_line(br#"{"type":"patch","files":[]}"#).unwrap();
        assert_eq!(patch, KeelEvent::Other);
    }

    fn fake_cli_sandbox() -> &'static str {
        if cfg!(target_os = "macos") {
            "seatbelt"
        } else {
            "landlock"
        }
    }

    async fn write_ndjson_line(w: &mut (impl AsyncWriteExt + Unpin), line: &str) {
        w.write_all(line.as_bytes()).await.unwrap();
        w.write_all(b"\n").await.unwrap();
        w.flush().await.unwrap();
    }

    /// Fake-CLI e2e: duplex NDJSON peer speaks capabilities → text → tool →
    /// approval (allow) → tool_result → done. Proves Desktop drives the stream
    /// and writes stdio-line-v1 decisions without a real `keel` binary.
    #[tokio::test]
    async fn fake_cli_e2e_stream_text_tool_approval_allow_and_done() {
        let (mut cli_out, desktop_in) = tokio::io::duplex(64 * 1024);
        let (mut desktop_out, cli_in) = tokio::io::duplex(64 * 1024);
        let sandbox = fake_cli_sandbox();

        let fake = tokio::spawn(async move {
            let mut cli_in = BufReader::new(cli_in);
            write_ndjson_line(
                &mut cli_out,
                &format!(
                    r#"{{"type":"capabilities","approval_protocol":"stdio-line-v1","stop_protocol":"sigint-shell-v1","sandbox":"{sandbox}"}}"#
                ),
            )
            .await;
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"text","content":"I'll list files."}"#,
            )
            .await;
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"tool_call","id":"tc1","name":"run_shell","arguments":{"command":"ls"}}"#,
            )
            .await;
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"approval_request","protocol":"stdio-line-v1","id":"ap1","tool_call_id":"tc1","tool":"run_shell","arguments":{"command":"ls"}}"#,
            )
            .await;

            let mut decision = Vec::new();
            cli_in.read_until(b'\n', &mut decision).await.unwrap();
            assert_eq!(
                decision, b"allow\n",
                "Desktop must write allow for AllowOnce"
            );

            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"tool_result","id":"tc1","output":"src\n"}"#,
            )
            .await;
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"turn_summary","summary":{"tools":1,"files":0,"tokens":12}}"#,
            )
            .await;
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"done","status":"ok","answer":"Listed the workspace.","error":null}"#,
            )
            .await;
        });

        let mut events = Vec::new();
        let answer = read_keel_turn(
            desktop_in,
            &mut desktop_out,
            |event| events.push(event),
            |_id, tool, summary, args| {
                assert_eq!(tool, "run_shell");
                assert!(summary.contains("run_shell"));
                assert_eq!(args.get("command").and_then(Value::as_str), Some("ls"));
                async { ApprovalDecision::AllowOnce }
            },
        )
        .await
        .expect("fake CLI turn should succeed");

        fake.await.expect("fake CLI task");
        assert_eq!(answer, "Listed the workspace.");
        assert!(
            events.iter().any(
                |e| matches!(e, AgentEvent::ContentDelta { text } if text == "I'll list files.")
            ),
            "expected text delta: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::ToolCallStart { name, .. } if name == "run_shell"
            )),
            "expected tool start: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::ToolCallResult {
                    name,
                    result,
                    is_error: false,
                    edit_id: None,
                } if name == "run_shell" && result == "src\n"
            )),
            "expected tool result: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::Compacted { note } if note.contains("1 tool call")
            )),
            "expected turn summary: {events:?}"
        );
        // Diff/--shadow: no staged edits until CLI advertises a file-edit event.
        assert!(
            events
                .iter()
                .all(|e| !matches!(e, AgentEvent::StagedEditAdded { .. })),
            "must not invent staged edits from the stream: {events:?}"
        );
    }

    /// Fake-CLI e2e deny path: Desktop writes `deny\n` and projects a denied tool result.
    #[tokio::test]
    async fn fake_cli_e2e_approval_deny_projects_error_without_tool_result() {
        let (mut cli_out, desktop_in) = tokio::io::duplex(64 * 1024);
        let (mut desktop_out, cli_in) = tokio::io::duplex(64 * 1024);
        let sandbox = fake_cli_sandbox();

        let fake = tokio::spawn(async move {
            let mut cli_in = BufReader::new(cli_in);
            write_ndjson_line(
                &mut cli_out,
                &format!(
                    r#"{{"type":"capabilities","approval_protocol":"stdio-line-v1","stop_protocol":"sigint-shell-v1","sandbox":"{sandbox}"}}"#
                ),
            )
            .await;
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"tool_call","id":"tc2","name":"run_shell","arguments":{"command":"rm -rf /"}}"#,
            )
            .await;
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"approval_request","protocol":"stdio-line-v1","id":"ap2","tool_call_id":"tc2","tool":"run_shell","arguments":{"command":"rm -rf /"}}"#,
            )
            .await;

            let mut decision = Vec::new();
            cli_in.read_until(b'\n', &mut decision).await.unwrap();
            assert_eq!(decision, b"deny\n");

            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"done","status":"ok","answer":"Stopped.","error":null}"#,
            )
            .await;
        });

        let mut events = Vec::new();
        let answer = read_keel_turn(
            desktop_in,
            &mut desktop_out,
            |event| events.push(event),
            |_id, _tool, _summary, _args| async { ApprovalDecision::Deny },
        )
        .await
        .expect("denied turn should still finish");

        fake.await.expect("fake CLI task");
        assert_eq!(answer, "Stopped.");
        assert!(
            events.iter().any(|e| matches!(
                e,
                AgentEvent::ToolCallResult {
                    name,
                    is_error: true,
                    result,
                    ..
                } if name == "run_shell" && result.contains("Denied by you")
            )),
            "expected denied tool projection: {events:?}"
        );
    }

    /// Diff/--shadow parity is intentionally not claimed yet: unknown file-edit
    /// lines stay `Other` and never become `StagedEditAdded`. When the CLI
    /// advertises a dedicated event, replace this with a real shadow assertion.
    #[tokio::test]
    async fn fake_cli_diff_shadow_parity_blocked_until_file_edit_protocol() {
        let (mut cli_out, desktop_in) = tokio::io::duplex(64 * 1024);
        let (mut desktop_out, _cli_in) = tokio::io::duplex(64 * 1024);
        let sandbox = fake_cli_sandbox();

        let fake = tokio::spawn(async move {
            write_ndjson_line(
                &mut cli_out,
                &format!(
                    r#"{{"type":"capabilities","approval_protocol":"stdio-line-v1","stop_protocol":"sigint-shell-v1","sandbox":"{sandbox}"}}"#
                ),
            )
            .await;
            // Hypothetical future event — must not be treated as production Diff input.
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"file_edit","path":"src/main.rs","diff":"--- a\n+++ b\n"}"#,
            )
            .await;
            write_ndjson_line(
                &mut cli_out,
                r#"{"type":"done","status":"ok","answer":"no diff yet","error":null}"#,
            )
            .await;
        });

        let mut events = Vec::new();
        let answer = read_keel_turn(
            desktop_in,
            &mut desktop_out,
            |event| events.push(event),
            |_id, _tool, _summary, _args| async { ApprovalDecision::Deny },
        )
        .await
        .expect("unknown event types are skipped, turn still completes");

        fake.await.expect("fake CLI task");
        assert_eq!(answer, "no diff yet");
        assert!(
            events.is_empty()
                || events
                    .iter()
                    .all(|e| !matches!(e, AgentEvent::StagedEditAdded { .. })),
            "Diff/--shadow must stay off until a real file-edit protocol event exists: {events:?}"
        );
    }

    #[test]
    fn safe_journal_export_name_strips_path_separators() {
        let name = safe_journal_export_name("abc/../evil", "Hello World!!/../x");
        // "!!/../" becomes six '-' chars between world and x.
        assert_eq!(name, "keel-desktop-abcevil-hello-world------x.jsonl");
        assert!(!name.contains('/'));
        assert!(!name.contains('\\'));
    }

    #[test]
    fn export_keel_session_log_copies_into_sessions_dir() {
        let dir = std::env::temp_dir().join(format!("keel_export_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.jsonl");
        std::fs::write(
            &source,
            r#"{"kind":"user","data":{"text":"hi"}}
"#,
        )
        .unwrap();
        let validated = validate_keel_session_log(&source).unwrap();
        assert_eq!(validated, source.canonicalize().unwrap());

        // Point home at a temp tree so export does not touch the real ~/.keel.
        // `dirs::home_dir` reads HOME on Unix and USERPROFILE on Windows.
        let fake_home = dir.join("home");
        std::fs::create_dir_all(&fake_home).unwrap();
        let prev_home = std::env::var_os("HOME");
        let prev_profile = std::env::var_os("USERPROFILE");
        std::env::set_var("HOME", &fake_home);
        std::env::set_var("USERPROFILE", &fake_home);
        let exported =
            export_keel_session_log(&source, "sess-1", "My Thread").expect("export should succeed");
        match prev_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        match prev_profile {
            Some(value) => std::env::set_var("USERPROFILE", value),
            None => std::env::remove_var("USERPROFILE"),
        }
        assert!(
            exported.starts_with(fake_home.join(".keel").join("sessions")),
            "exported={exported:?} fake_home={fake_home:?}"
        );
        assert_eq!(
            exported.file_name().and_then(|n| n.to_str()),
            Some("keel-desktop-sess-1-my-thread.jsonl")
        );
        assert_eq!(
            std::fs::read_to_string(&exported).unwrap(),
            std::fs::read_to_string(&source).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validate_keel_session_log_rejects_missing_paths() {
        let missing =
            std::env::temp_dir().join(format!("no-journal-{}.jsonl", uuid::Uuid::new_v4()));
        let err = validate_keel_session_log(&missing).unwrap_err().to_string();
        assert!(err.contains("Cannot find"), "unexpected: {err}");
    }
}
