use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::apps::AppPreset;
use crate::compaction::{compact_messages, ChatTurnMessage};
use crate::guard::{parse_tool_calls_from_text, split_thinking, CircuitBreaker, RepeatToolGuard};
use crate::mcp::{invoke_mcp_tool, mcp_tools_to_openai_schemas, McpConfigFile, McpToolInfo};
use crate::tools::{
    apply_staged_edit, build_staged_edit, execute_sql_tool, list_workspace_files,
    openai_tool_schemas, read_workspace_file, run_shell_command, search_workspace_files,
    validate_workspace_root, StagedEdit, StagedEditKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    AllowOnce,
    AllowAlways,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    ThinkingDelta {
        text: String,
    },
    ContentDelta {
        text: String,
    },
    ToolCallStart {
        name: String,
        arguments: Value,
    },
    ToolCallResult {
        name: String,
        result: String,
        is_error: bool,
        #[serde(default)]
        edit_id: Option<String>,
    },
    StagedEditAdded {
        edit: StagedEdit,
    },
    ApprovalRequired {
        id: String,
        tool: String,
        summary: String,
        arguments: Value,
    },
    Compacted {
        note: String,
    },
    TurnFinished {
        content: String,
        thinking: Option<String>,
        prompt_tokens: u64,
        completion_tokens: u64,
        tokens_per_sec: f64,
    },
}

pub struct TurnParams {
    pub base_url: String,
    pub model_name: String,
    pub context_size: usize,
    pub app_preset: AppPreset,
    pub custom_instructions: String,
    pub workspace_root: Option<PathBuf>,
    pub auto_apply_edits: bool,
    pub enable_thinking: bool,
    pub history: Vec<ChatTurnMessage>,
    pub mcp_config: McpConfigFile,
    pub mcp_tools: Vec<McpToolInfo>,
    pub max_steps: usize,
}

pub struct TurnOutcome {
    pub content: String,
    pub thinking: Option<String>,
    pub staged_edits: Vec<StagedEdit>,
    pub tool_logs: Vec<crate::sessions::UiToolLog>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub tokens_per_sec: f64,
}

/// Runs a multi-step local tool-calling agent turn over an OpenAI-compatible SSE endpoint.
pub async fn run_agent_turn<E, A, AFut>(
    params: TurnParams,
    always_allowed_tools: &mut HashSet<String>,
    mut emit: E,
    mut ask_approval: A,
) -> Result<TurnOutcome>
where
    E: FnMut(AgentEvent),
    A: FnMut(String, String, String, Value) -> AFut,
    AFut: std::future::Future<Output = ApprovalDecision>,
{
    let validated_ws = match &params.workspace_root {
        _ if params.app_preset == AppPreset::Chat => None,
        Some(p) => Some(validate_workspace_root(p)?),
        None => None,
    };

    let mut messages = Vec::new();
    let system_prompt = build_system_prompt(
        params.app_preset,
        &params.custom_instructions,
        validated_ws.as_deref(),
        params.auto_apply_edits,
        params.enable_thinking,
    );
    messages.push(ChatTurnMessage {
        role: "system".to_string(),
        content: system_prompt,
        tool_call_id: None,
        tool_calls: None,
    });
    let latest_request = params
        .history
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.clone())
        .unwrap_or_default();
    messages.extend(params.history);

    let mcp_schemas = if params.app_preset == AppPreset::Chat {
        Vec::new()
    } else {
        mcp_tools_to_openai_schemas(&params.mcp_tools)
    };
    let allow_shell = crate::harnesses::code_execution_boundary().can_run_commands;
    let tool_schemas = if params.app_preset != AppPreset::Chat
        && (validated_ws.is_some() || !mcp_schemas.is_empty())
    {
        let base_allowed = if validated_ws.is_some() {
            params.app_preset.tools_for_request(&latest_request)
        } else {
            &["run_sql"][..]
        };
        let filtered: Vec<&str> = base_allowed
            .iter()
            .copied()
            .filter(|name| allow_shell || *name != "run_shell")
            .collect();
        openai_tool_schemas(&filtered, &mcp_schemas)
    } else {
        Vec::new()
    };

    let exposed_tools: HashSet<String> = tool_schemas
        .iter()
        .filter_map(|s| s.get("function")?.get("name")?.as_str().map(String::from))
        .collect();

    let mcp_lookup: HashMap<String, McpToolInfo> = params
        .mcp_tools
        .iter()
        .map(|t| (t.full_name.clone(), t.clone()))
        .collect();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()?;
    let endpoint = format!("{}/chat/completions", params.base_url.trim_end_matches('/'));

    let mut repeat_guard = RepeatToolGuard::default();
    let mut breaker = CircuitBreaker::default();
    let mut staged_edits = Vec::new();
    let mut tool_logs = Vec::new();
    let mut total_prompt_tokens = 0u64;
    let mut total_completion_tokens = 0u64;
    let mut total_thinking = String::new();
    let mut final_content = String::new();
    let turn_start = Instant::now();

    let budget = (params.context_size * 3) / 4;
    let steps = params.max_steps.max(1);

    for _step in 0..steps {
        let (compacted, did_compact) = compact_messages(messages, budget.max(2048), 6);
        messages = compacted;
        if did_compact {
            emit(AgentEvent::Compacted {
                note: "Compacted earlier messages to stay inside context budget.".to_string(),
            });
        }

        if crate::compaction::estimate_tokens(&messages) > budget.max(2048) {
            bail!("This request is larger than the selected model’s context. Shorten the message or project instructions, or attach a document so Keel can select excerpts.");
        }

        let resp = stream_completion(
            &client,
            &endpoint,
            &params.model_name,
            &messages,
            &tool_schemas,
            params.enable_thinking,
            &mut emit,
        )
        .await?;

        total_prompt_tokens += resp.prompt_tokens;
        total_completion_tokens += resp.completion_tokens;

        let (inline_think, clean_content) = split_thinking(&resp.content);
        let step_thinking = combine_thinking(&resp.reasoning_content, &inline_think);
        if !step_thinking.is_empty() {
            if !total_thinking.is_empty() {
                total_thinking.push('\n');
            }
            total_thinking.push_str(&step_thinking);
        }

        let (tool_calls, visible_text) = if !resp.tool_calls.is_empty() {
            (resp.tool_calls, clean_content)
        } else {
            parse_tool_calls_from_text(&clean_content)
        };

        if tool_calls.is_empty() {
            final_content = visible_text;
            break;
        }

        let openai_tool_calls: Vec<Value> = tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.name,
                        "arguments": tc.arguments.to_string()
                    }
                })
            })
            .collect();

        messages.push(ChatTurnMessage {
            role: "assistant".to_string(),
            content: visible_text,
            tool_call_id: None,
            tool_calls: Some(Value::Array(openai_tool_calls)),
        });

        for tc in tool_calls {
            emit(AgentEvent::ToolCallStart {
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            });

            let repeated_nudge = repeat_guard.record(&tc.name, &tc.arguments);

            let exec = if !exposed_tools.contains(&tc.name) {
                Err(anyhow::anyhow!(
                    "Tool `{}` is not available in this mode",
                    tc.name
                ))
            } else {
                execute_tool_call(
                    &tc.name,
                    &tc.arguments,
                    validated_ws.as_deref(),
                    params.auto_apply_edits,
                    &params.mcp_config,
                    &mcp_lookup,
                    always_allowed_tools,
                    &mut emit,
                    &mut ask_approval,
                )
                .await
            };

            let (result_text, is_error, staged) = match exec {
                Ok((txt, maybe_edit)) => {
                    breaker.record_success();
                    (txt, false, maybe_edit)
                }
                Err(e) => {
                    let tripped = breaker.record_failure();
                    let msg = if tripped {
                        format!("Error: {e}\nCircuit breaker tripped after repeated errors; stopping tool loop.")
                    } else {
                        format!("Error: {e}")
                    };
                    (msg, true, None)
                }
            };

            let staged_id = staged.as_ref().map(|e| e.id.clone());
            if let Some(edit) = staged {
                emit(AgentEvent::StagedEditAdded { edit: edit.clone() });
                staged_edits.push(edit);
            }

            emit(AgentEvent::ToolCallResult {
                name: tc.name.clone(),
                result: result_text.clone(),
                is_error,
                edit_id: staged_id.clone(),
            });

            tool_logs.push(crate::sessions::UiToolLog {
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
                result: Some(result_text.clone()),
                is_error,
                edit_id: staged_id,
            });

            let content = match repeated_nudge {
                Some(nudge) => format!("{result_text}\n\nKeel tool-loop note: {nudge}"),
                None => result_text,
            };
            messages.push(ChatTurnMessage {
                role: "tool".to_string(),
                content,
                tool_call_id: Some(tc.id),
                tool_calls: None,
            });

            if breaker.record_failure_peek() {
                final_content =
                    "Stopped after repeated tool failures. Check the tool output above."
                        .to_string();
                let secs = turn_start.elapsed().as_secs_f64().max(0.01);
                let tps = total_completion_tokens as f64 / secs;
                let thinking = (!total_thinking.is_empty()).then_some(total_thinking);
                emit(AgentEvent::TurnFinished {
                    content: final_content.clone(),
                    thinking: thinking.clone(),
                    prompt_tokens: total_prompt_tokens,
                    completion_tokens: total_completion_tokens,
                    tokens_per_sec: tps,
                });
                return Ok(TurnOutcome {
                    content: final_content,
                    thinking,
                    staged_edits,
                    tool_logs,
                    prompt_tokens: total_prompt_tokens,
                    completion_tokens: total_completion_tokens,
                    tokens_per_sec: tps,
                });
            }
        }
    }

    if final_content.trim().is_empty() {
        final_content = "I reached the step limit for this turn. The tool results and any proposed edits are available above; tell me which step to continue.".into();
    }
    let secs = turn_start.elapsed().as_secs_f64().max(0.01);
    let tps = total_completion_tokens as f64 / secs;
    let thinking = (!total_thinking.is_empty()).then_some(total_thinking);

    emit(AgentEvent::TurnFinished {
        content: final_content.clone(),
        thinking: thinking.clone(),
        prompt_tokens: total_prompt_tokens,
        completion_tokens: total_completion_tokens,
        tokens_per_sec: tps,
    });

    Ok(TurnOutcome {
        content: final_content,
        thinking,
        staged_edits,
        tool_logs,
        prompt_tokens: total_prompt_tokens,
        completion_tokens: total_completion_tokens,
        tokens_per_sec: tps,
    })
}

trait CircuitPeek {
    fn record_failure_peek(&self) -> bool;
}

impl CircuitPeek for CircuitBreaker {
    fn record_failure_peek(&self) -> bool {
        // Expose only via existing failure counting: handled inside record_failure error string
        false
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_tool_call<E, A, AFut>(
    name: &str,
    args: &Value,
    workspace: Option<&std::path::Path>,
    auto_apply: bool,
    mcp_config: &McpConfigFile,
    mcp_lookup: &HashMap<String, McpToolInfo>,
    always_allowed: &mut HashSet<String>,
    emit: &mut E,
    ask_approval: &mut A,
) -> Result<(String, Option<StagedEdit>)>
where
    E: FnMut(AgentEvent),
    A: FnMut(String, String, String, Value) -> AFut,
    AFut: std::future::Future<Output = ApprovalDecision>,
{
    match name {
        "list_files" => {
            let root = workspace.context("No workspace folder is selected")?;
            let sub = args.get("path").and_then(|v| v.as_str());
            Ok((list_workspace_files(root, sub)?, None))
        }
        "read_file" => {
            let root = workspace.context("No workspace folder is selected")?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .context("Missing `path` argument")?;
            let start = args
                .get("start_line")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            let end = args
                .get("end_line")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);
            Ok((read_workspace_file(root, path, start, end)?, None))
        }
        "search_files" => {
            let root = workspace.context("No workspace folder is selected")?;
            let pattern = args
                .get("pattern")
                .and_then(|v| v.as_str())
                .context("Missing `pattern` argument")?;
            Ok((search_workspace_files(root, pattern)?, None))
        }
        "propose_edit" => {
            let root = workspace.context("No workspace folder is selected")?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .context("Missing `path`")?;
            let target = args.get("target").and_then(|v| v.as_str());
            let replacement = args.get("replacement").and_then(|v| v.as_str());
            let mut edit =
                build_staged_edit(root, StagedEditKind::Edit, path, target, replacement, None)?;
            if auto_apply {
                apply_staged_edit(root, &mut edit)?;
                Ok((format!("Applied edit to {path}."), Some(edit)))
            } else {
                Ok((
                    format!("Staged edit for {path} (awaiting user review)."),
                    Some(edit),
                ))
            }
        }
        "create_file" => {
            let root = workspace.context("No workspace folder is selected")?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .context("Missing `path`")?;
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let mut edit = build_staged_edit(
                root,
                StagedEditKind::Create,
                path,
                None,
                Some(content),
                None,
            )?;
            if auto_apply {
                apply_staged_edit(root, &mut edit)?;
                Ok((format!("Created/updated {path}."), Some(edit)))
            } else {
                Ok((
                    format!("Staged new file {path} (awaiting user review)."),
                    Some(edit),
                ))
            }
        }
        "delete_file" => {
            let root = workspace.context("No workspace folder is selected")?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .context("Missing `path`")?;
            let mut edit = build_staged_edit(root, StagedEditKind::Delete, path, None, None, None)?;
            if auto_apply {
                apply_staged_edit(root, &mut edit)?;
                Ok((format!("Deleted {path}."), Some(edit)))
            } else {
                Ok((
                    format!("Staged deletion of {path} (awaiting user review)."),
                    Some(edit),
                ))
            }
        }
        "rename_file" => {
            let root = workspace.context("No workspace folder is selected")?;
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .context("Missing `path`")?;
            let new_path = args
                .get("new_path")
                .and_then(|v| v.as_str())
                .context("Missing `new_path`")?;
            let mut edit = build_staged_edit(
                root,
                StagedEditKind::Rename,
                path,
                None,
                None,
                Some(new_path),
            )?;
            if auto_apply {
                apply_staged_edit(root, &mut edit)?;
                Ok((format!("Renamed {path} -> {new_path}."), Some(edit)))
            } else {
                Ok((
                    format!("Staged rename {path} -> {new_path} (awaiting user review)."),
                    Some(edit),
                ))
            }
        }
        "run_sql" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .context("Missing `query`")?;
            Ok((execute_sql_tool(workspace, query)?, None))
        }
        "run_shell" => {
            let root = workspace.context("No workspace folder is selected")?;
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .context("Missing `command`")?;

            if !always_allowed.contains("run_shell") {
                let req_id = uuid::Uuid::new_v4().to_string();
                let summary = format!("Run shell command: `{command}`");
                emit(AgentEvent::ApprovalRequired {
                    id: req_id.clone(),
                    tool: "run_shell".to_string(),
                    summary: summary.clone(),
                    arguments: args.clone(),
                });
                match ask_approval(req_id, "run_shell".to_string(), summary, args.clone()).await {
                    ApprovalDecision::AllowOnce => {}
                    ApprovalDecision::AllowAlways => {
                        always_allowed.insert("run_shell".to_string());
                    }
                    ApprovalDecision::Deny => {
                        bail!("User denied shell command `{command}`");
                    }
                }
            }
            Ok((run_shell_command(root, command).await?, None))
        }
        other => {
            let info = mcp_lookup
                .get(other)
                .with_context(|| format!("Unknown tool `{other}`"))?;
            if !info.read_only && !always_allowed.contains(other) {
                let req_id = uuid::Uuid::new_v4().to_string();
                let summary = format!("Run MCP tool `{}` on `{}`", info.name, info.server);
                emit(AgentEvent::ApprovalRequired {
                    id: req_id.clone(),
                    tool: other.to_string(),
                    summary: summary.clone(),
                    arguments: args.clone(),
                });
                match ask_approval(req_id, other.to_string(), summary, args.clone()).await {
                    ApprovalDecision::AllowOnce => {}
                    ApprovalDecision::AllowAlways => {
                        always_allowed.insert(other.to_string());
                    }
                    ApprovalDecision::Deny => {
                        bail!("User denied MCP tool `{other}`");
                    }
                }
            }
            let out = invoke_mcp_tool(
                mcp_config,
                workspace,
                &info.server,
                &info.name,
                args.clone(),
            )
            .await?;
            Ok((out, None))
        }
    }
}

struct StreamedCompletion {
    content: String,
    reasoning_content: String,
    tool_calls: Vec<crate::guard::ParsedToolCall>,
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

async fn stream_completion<E>(
    client: &reqwest::Client,
    endpoint: &str,
    model: &str,
    messages: &[ChatTurnMessage],
    tools: &[Value],
    enable_thinking: bool,
    emit: &mut E,
) -> Result<StreamedCompletion>
where
    E: FnMut(AgentEvent),
{
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
        "chat_template_kwargs": { "enable_thinking": enable_thinking },
        "temperature": 0.5,
        "max_tokens": 2048
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
        body["tool_choice"] = json!("auto");
    }

    let resp = client
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("Could not reach model endpoint {endpoint}"))?
        .error_for_status()
        .with_context(|| format!("Model server returned an error at {endpoint}"))?;

    let mut content = String::new();
    let mut reasoning_content = String::new();
    let mut partial_calls: HashMap<usize, PartialToolCall> = HashMap::new();
    let mut prompt_tokens = 0u64;
    let mut completion_tokens = 0u64;

    let mut decoder = crate::streaming::SseLines::default();
    let mut router = crate::streaming::TextRouter::default();
    let mut stream = resp.bytes_stream();
    let mut received = false;
    let mut finished = false;

    loop {
        let data_lines = match stream.next().await {
            Some(chunk) => decoder.push(&chunk?)?,
            None => {
                finished = true;
                decoder.finish()?
            }
        };
        for data in data_lines {
            if data == "[DONE]" {
                finished = true;
                continue;
            }
            let val: Value = serde_json::from_str(&data)
                .context("Model server sent an invalid streaming event")?;
            if let Some(error) = val.get("error") {
                bail!("Model server: {}", error.get("message").unwrap_or(error));
            }
            received = true;

            if let Some(usage) = val.get("usage") {
                if let Some(p) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                    prompt_tokens = p;
                }
                if let Some(c) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                    completion_tokens = c;
                }
            }

            let Some(choice) = val
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|a| a.first())
            else {
                continue;
            };
            let Some(delta) = choice.get("delta") else {
                continue;
            };

            if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                reasoning_content.push_str(r);
                emit(AgentEvent::ThinkingDelta {
                    text: r.to_string(),
                });
            }

            if let Some(c) = delta.get("content").and_then(|v| v.as_str()) {
                content.push_str(c);
                for (thinking, text) in router.push(c) {
                    if thinking {
                        emit(AgentEvent::ThinkingDelta { text });
                    } else {
                        emit(AgentEvent::ContentDelta { text });
                    }
                }
            }

            if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for tc in tcs {
                    let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    let entry = partial_calls.entry(idx).or_default();
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        entry.id = id.to_string();
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(n) = func.get("name").and_then(|v| v.as_str()) {
                            entry.name.push_str(n);
                        }
                        if let Some(a) = func.get("arguments").and_then(|v| v.as_str()) {
                            entry.arguments.push_str(a);
                        }
                    }
                }
            }
        }
        if finished {
            break;
        }
    }
    for (thinking, text) in router.finish() {
        if thinking {
            emit(AgentEvent::ThinkingDelta { text });
        } else {
            emit(AgentEvent::ContentDelta { text });
        }
    }
    if !received {
        bail!(
            "The model server returned no response. Check that a model is loaded, then try again."
        );
    }

    let mut ordered_indices: Vec<usize> = partial_calls.keys().copied().collect();
    ordered_indices.sort_unstable();
    let tool_calls = ordered_indices
        .into_iter()
        .filter_map(|idx| {
            let pc = partial_calls.remove(&idx)?;
            if pc.name.trim().is_empty() {
                return None;
            }
            let args: Value =
                serde_json::from_str(pc.arguments.trim()).unwrap_or_else(|_| json!({}));
            Some(crate::guard::ParsedToolCall {
                id: if pc.id.is_empty() {
                    format!("call-{idx}")
                } else {
                    pc.id
                },
                name: pc.name,
                arguments: args,
            })
        })
        .collect();

    if completion_tokens == 0 {
        completion_tokens = ((content.len() + reasoning_content.len()) / 4).max(1) as u64;
    }

    Ok(StreamedCompletion {
        content,
        reasoning_content,
        tool_calls,
        prompt_tokens,
        completion_tokens,
    })
}

fn combine_thinking(reasoning_field: &str, inline_think: &str) -> String {
    match (reasoning_field.trim(), inline_think.trim()) {
        ("", "") => String::new(),
        (a, "") => a.to_string(),
        ("", b) => b.to_string(),
        (a, b) => format!("{a}\n{b}"),
    }
}

fn build_system_prompt(
    preset: AppPreset,
    custom_instructions: &str,
    workspace: Option<&std::path::Path>,
    auto_apply: bool,
    enable_thinking: bool,
) -> String {
    let mut parts = vec![preset.instructions().to_string()];

    if preset == AppPreset::Chat {
        // Chat deliberately receives no workspace context or tool instructions.
    } else if let Some(ws) = workspace {
        let mode_note = if auto_apply {
            "File edits you make apply immediately to the workspace."
        } else {
            "File edits (`propose_edit`, `create_file`, `delete_file`, `rename_file`) are staged for user diff review."
        };
        parts.push(format!("Workspace root: `{}`. All tool paths must be relative to this folder; never pass absolute paths. {mode_note}", ws.display()));
    } else {
        parts.push(
            "No workspace folder is currently selected. You can still answer directly or run SQL queries via `run_sql`."
                .to_string(),
        );
    }

    if !enable_thinking {
        parts.push(
            "Keep reasoning concise and answer directly without long chain-of-thought.".to_string(),
        );
    }

    if !custom_instructions.trim().is_empty() {
        parts.push(format!(
            "User standing instructions:\n{}",
            custom_instructions.trim()
        ));
    }

    parts.join("\n\n")
}
