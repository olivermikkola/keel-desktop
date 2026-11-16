mod support;
use keel_core::agent::{run_agent_turn, ApprovalDecision, TurnParams};
use keel_core::apps::AppPreset;
use keel_core::compaction::ChatTurnMessage;
use keel_core::mcp::McpConfigFile;
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;

fn completion(delta: serde_json::Value) -> (String, String, Duration) {
    (
        "text/event-stream".into(),
        format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":delta}]})
        ),
        Duration::ZERO,
    )
}

#[tokio::test]
async fn chat_cannot_execute_hallucinated_tools_even_with_a_workspace() {
    let root = std::env::temp_dir().join(format!("keel_mode_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let (endpoint, server) = support::server(vec![
        completion(json!({"content":"<tool_call>{\"name\":\"create_file\",\"arguments\":{\"path\":\"surprise.txt\",\"content\":\"no\"}}</tool_call>"})),
        completion(json!({"content":"I can help with writing here. Switch to Work to create a file."})),
    ]).await;
    let outcome = run_agent_turn(
        TurnParams {
            base_url: endpoint,
            model_name: "test".into(),
            context_size: 8192,
            app_preset: AppPreset::Chat,
            custom_instructions: String::new(),
            workspace_root: Some(root.clone()),
            auto_apply_edits: true,
            enable_thinking: false,
            history: vec![ChatTurnMessage {
                role: "user".into(),
                content: "Write a file".into(),
                tool_call_id: None,
                tool_calls: None,
            }],
            mcp_config: McpConfigFile::default(),
            mcp_tools: vec![],
            max_steps: 2,
        },
        &mut HashSet::new(),
        |_| {},
        |_, _, _, _| async {
            panic!("Chat must never ask for tool approval");
        },
    )
    .await
    .unwrap();
    let requests = server.await.unwrap();
    assert!(requests.iter().all(|r| r.get("tools").is_none()));
    assert!(!requests[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains(&root.to_string_lossy().to_string()));
    assert!(!root.join("surprise.txt").exists());
    assert!(outcome.staged_edits.is_empty());
    assert!(outcome.tool_logs[0].is_error);
    assert!(outcome.tool_logs[0]
        .result
        .as_ref()
        .unwrap()
        .contains("not available"));
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn work_stages_real_edits_without_writing_and_has_no_shell() {
    let root = std::env::temp_dir().join(format!("keel_work_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let (endpoint, server) = support::server(vec![
        completion(json!({"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"create_file","arguments":"{\"path\":\"brief.md\",\"content\":\"# Brief\",\"summary\":\"Write brief\"}"}}]})),
        completion(json!({"content":"The brief is ready for review."})),
    ]).await;
    let outcome = run_agent_turn(
        TurnParams {
            base_url: endpoint,
            model_name: "test".into(),
            context_size: 8192,
            app_preset: AppPreset::General,
            custom_instructions: String::new(),
            workspace_root: Some(root.clone()),
            auto_apply_edits: false,
            enable_thinking: false,
            history: vec![],
            mcp_config: McpConfigFile::default(),
            mcp_tools: vec![],
            max_steps: 2,
        },
        &mut HashSet::new(),
        |_| {},
        |_, _, _, _| async { ApprovalDecision::Deny },
    )
    .await
    .unwrap();
    let requests = server.await.unwrap();
    let tools = requests[0]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["function"]["name"] == "create_file"));
    assert!(!tools.iter().any(|t| t["function"]["name"] == "run_shell"));
    assert_eq!(outcome.staged_edits.len(), 1);
    assert!(!outcome.tool_logs[0].is_error);
    assert!(!root.join("brief.md").exists());
    keel_core::tools::apply_staged_edit(&root, &mut outcome.staged_edits[0].clone()).unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("brief.md")).unwrap(),
        "# Brief"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_general_sessions_still_map_to_work_and_code_has_shell() {
    let preset: AppPreset = serde_json::from_str("\"general\"").unwrap();
    assert_eq!(preset.mode(), "work");
    assert!(AppPreset::Code.allowed_tools().contains(&"run_shell"));
    assert!(!AppPreset::Chat.allowed_tools().contains(&"read_file"));
}
