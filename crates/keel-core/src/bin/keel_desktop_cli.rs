use anyhow::{bail, Result};
use keel_core::agent::{run_agent_turn, AgentEvent, ApprovalDecision, TurnParams};
use keel_core::apps::AppPreset;
use keel_core::compaction::ChatTurnMessage;
use keel_core::harnesses::detect_harnesses;
use keel_core::mcp::McpConfigFile;
use keel_core::models::default_catalog;
use keel_core::runtime::find_llama_server;
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "keel-desktop-cli {}\n\nUsage:\n  keel-desktop-cli --info\n  keel-desktop-cli --base-url <url> --model <name> [--workspace <dir>] [--auto-apply] [--auto-approve] <prompt>",
            keel_core::version()
        );
        return Ok(());
    }

    if args.iter().any(|a| a == "--info") {
        println!("Keel Desktop Core v{}", keel_core::version());
        println!(
            "llama-server: {}",
            find_llama_server(None)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not found".to_string())
        );
        println!("Catalog models: {}", default_catalog().len());
        for h in detect_harnesses() {
            println!("Harness: {} (available={})", h.id, h.available);
        }
        return Ok(());
    }

    let mut base_url = "http://127.0.0.1:8080/v1".to_string();
    let mut model = "default".to_string();
    let mut workspace: Option<PathBuf> = None;
    let mut auto_apply = false;
    let mut auto_approve = false;
    let mut prompt_parts = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--base-url" => {
                i += 1;
                base_url = args.get(i).cloned().unwrap_or_default();
            }
            "--model" => {
                i += 1;
                model = args.get(i).cloned().unwrap_or_default();
            }
            "--workspace" => {
                i += 1;
                workspace = args.get(i).map(PathBuf::from);
            }
            "--auto-apply" => auto_apply = true,
            "--auto-approve" => auto_approve = true,
            other => prompt_parts.push(other.to_string()),
        }
        i += 1;
    }

    let prompt = prompt_parts.join(" ");
    if prompt.trim().is_empty() {
        bail!("Prompt must not be empty");
    }

    let mut allowed = HashSet::new();
    let outcome = run_agent_turn(
        TurnParams {
            base_url,
            model_name: model,
            context_size: 8192,
            app_preset: AppPreset::General,
            custom_instructions: String::new(),
            workspace_root: workspace,
            auto_apply_edits: auto_apply,
            enable_thinking: true,
            history: vec![ChatTurnMessage {
                role: "user".to_string(),
                content: prompt,
                tool_call_id: None,
                tool_calls: None,
            }],
            mcp_config: McpConfigFile::default(),
            mcp_tools: Vec::new(),
            max_steps: 8,
        },
        &mut allowed,
        |ev| match ev {
            AgentEvent::ContentDelta { text } => {
                print!("{text}");
                let _ = std::io::stdout().flush();
            }
            AgentEvent::ToolCallStart { name, .. } => {
                eprintln!("\n[tool: {name}]");
            }
            AgentEvent::ToolCallResult { name, is_error, .. } => {
                eprintln!("[tool done: {name}, error={is_error}]");
            }
            AgentEvent::StagedEditAdded { edit } => {
                eprintln!(
                    "[staged edit: {} (+{} -{}, {:?})]",
                    edit.path, edit.added_lines, edit.removed_lines, edit.status
                );
            }
            _ => {}
        },
        |_id, tool, summary, _args| async move {
            if auto_approve {
                eprintln!("[auto-approved {tool}: {summary}]");
                ApprovalDecision::AllowOnce
            } else {
                eprintln!("[denied {tool}: {summary}]");
                ApprovalDecision::Deny
            }
        },
    )
    .await?;

    println!("\n---");
    println!(
        "tokens: {} prompt / {} completion ({:.1} tok/s), staged_edits: {}",
        outcome.prompt_tokens,
        outcome.completion_tokens,
        outcome.tokens_per_sec,
        outcome.staged_edits.len()
    );
    Ok(())
}
