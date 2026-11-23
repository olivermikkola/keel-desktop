//! Opt-in real-model checks. Run against a server you deliberately start locally.
use anyhow::Result;
use keel_core::{
    agent::{run_agent_turn, ApprovalDecision, TurnParams},
    apps::AppPreset,
    compaction::ChatTurnMessage,
    documents::Document,
    mcp::McpConfigFile,
};
use serde_json::json;
use std::{collections::HashSet, time::Instant};

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::var("KEEL_EVAL_URL").unwrap_or_else(|_| "http://127.0.0.1:19181/v1".into());
    let model = std::env::var("KEEL_EVAL_MODEL").unwrap_or_else(|_| "bonsai4b".into());
    let root = std::env::temp_dir().join(format!("keel_eval_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join("sales.csv"),
        "region,revenue\nNorth,19\nSouth,31\nWest,50\n",
    )?;
    std::fs::write(
        root.join("calculator.py"),
        "def add(a, b):\n    return a - b\n",
    )?;
    let cases = [
        ("friendly_email", AppPreset::Chat, "Draft a friendly two-sentence email declining a meeting tomorrow and proposing Friday instead.", ""),
        ("document_facts", AppPreset::Chat, "What are the budget and launch date in the attached brief? Cite the filename and keep the answer short.", ""),
        ("csv_total", AppPreset::General, "Use run_sql to query sales.csv and report the sum of its revenue column. Give the exact total.", ""),
        ("code_fix", AppPreset::Code, "Read calculator.py and stage a fix so add(2, 3) returns 5. Use file tools and propose the edit; do not run shell commands. I will apply and test it after review.", ""),
        ("saved_preference", AppPreset::Chat, "Suggest one quick lunch for me in one sentence.", "User-saved preferences: I am vegetarian and prefer concise answers."),
    ];
    let mut results = Vec::new();
    for (name, preset, prompt, custom) in cases {
        if std::env::var("KEEL_EVAL_CASE").is_ok_and(|selected| selected != name) {
            continue;
        }
        let mut history = vec![ChatTurnMessage::text("user", prompt)];
        if name == "document_facts" {
            let session = keel_core::sessions::ChatSession { id: "fixture".into(), title: "Brief".into(), app_preset: preset, workspace_path: None, keel_session_log: None, code_checkout: None, applied_code_patch: None, last_code_test: None, project_id: None, pinned: false, staged_edits: vec![], updated_at: 0, messages: vec![keel_core::sessions::UiMessage { id: "user".into(), role: "user".into(), content: prompt.into(), thinking: None, tool_calls: vec![], status: "complete".into(), attachments: vec![Document { id: "brief".into(), name: "harbor-brief.txt".into(), kind: "txt".into(), text: "Project Harbor. Approved budget: 4,800 euros. Launch date: 14 May 2027. The audience is local cafes.".into(), truncated: false, image_data_url: None }] }] };
            history = keel_core::context::history(&session, None, 8192);
        }
        let mut config = keel_core::sessions::DesktopConfig::default();
        if name == "saved_preference" {
            config.personal_context = "I am vegetarian and prefer concise answers.".into();
            let session = keel_core::sessions::ChatSession {
                id: "fixture".into(),
                title: "Preference".into(),
                app_preset: preset,
                workspace_path: None,
                keel_session_log: None,
                code_checkout: None,
                applied_code_patch: None,
                last_code_test: None,
                project_id: None,
                pinned: false,
                staged_edits: vec![],
                updated_at: 0,
                messages: vec![keel_core::sessions::UiMessage {
                    id: "user".into(),
                    role: "user".into(),
                    content: prompt.into(),
                    thinking: None,
                    tool_calls: vec![],
                    status: "complete".into(),
                    attachments: vec![],
                }],
            };
            history = keel_core::context::history_for_turn(&session, &config, None, 8192);
        }
        let start = Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(240),
            run_agent_turn(
                TurnParams {
                    base_url: url.clone(),
                    model_name: model.clone(),
                    context_size: 8192,
                    app_preset: preset,
                    custom_instructions: if name == "saved_preference" {
                        keel_core::context::instructions(&config, None)
                    } else {
                        custom.into()
                    },
                    workspace_root: if preset == AppPreset::Chat {
                        None
                    } else {
                        Some(root.clone())
                    },
                    auto_apply_edits: false,
                    enable_thinking: false,
                    history,
                    mcp_config: McpConfigFile::default(),
                    mcp_tools: vec![],
                    max_steps: 6,
                api_key: None,
                },
                &mut HashSet::new(),
                |_| {},
                |_, _, _, _| async { ApprovalDecision::Deny },
            ),
        )
        .await;
        let seconds = start.elapsed().as_secs_f64();
        let row = match result {
            Ok(Ok(mut outcome)) => {
                let answer = outcome.content.to_lowercase();
                let passed = match name {
                    "friendly_email" => {
                        answer.contains("friday")
                            && answer.contains("tomorrow")
                            && !answer.contains("<think>")
                    }
                    "document_facts" => {
                        (answer.contains("4,800") || answer.contains("4800"))
                            && answer.contains("14")
                            && answer.contains("may")
                            && answer.contains("harbor-brief.txt")
                    }
                    "csv_total" => {
                        answer.contains("100")
                            && outcome
                                .tool_logs
                                .iter()
                                .any(|t| t.name == "run_sql" && !t.is_error)
                    }
                    "code_fix" => {
                        let has_edit = !outcome.staged_edits.is_empty();
                        for edit in &mut outcome.staged_edits {
                            keel_core::tools::apply_staged_edit(&root, edit)?;
                        }
                        has_edit && std::process::Command::new("python3").arg("-c").arg("from calculator import add; assert add(2,3)==5 and add(-1,1)==0").current_dir(&root).status()?.success()
                    }
                    "saved_preference" => {
                        !["chicken", "beef", "pork", "salmon", "tuna", "turkey"]
                            .iter()
                            .any(|word| answer.contains(word))
                            && !answer.trim().is_empty()
                    }
                    _ => false,
                };
                json!({"case":name,"passed":passed,"seconds":seconds,"answer":outcome.content,"tools":outcome.tool_logs,"prompt_tokens":outcome.prompt_tokens,"completion_tokens":outcome.completion_tokens})
            }
            Ok(Err(error)) => {
                json!({"case":name,"passed":false,"seconds":seconds,"error":error.to_string()})
            }
            Err(_) => {
                json!({"case":name,"passed":false,"seconds":seconds,"error":"Timed out after 240 seconds"})
            }
        };
        eprintln!(
            "{}: {} ({:.1}s)",
            name,
            if row["passed"] == true {
                "PASS"
            } else {
                "FAIL"
            },
            seconds
        );
        results.push(row);
    }
    let report = json!({"model":model,"cases":results,"note":"Small opt-in task checks, not a comparison with Claude or a broad quality benchmark. Read answers as well as pass flags."});
    println!("{}", serde_json::to_string_pretty(&report)?);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
