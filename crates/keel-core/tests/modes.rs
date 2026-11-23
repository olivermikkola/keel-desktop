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
            history: vec![ChatTurnMessage::text("user", "Write a file")],
            mcp_config: McpConfigFile::default(),
            mcp_tools: vec![],
            max_steps: 2,
            api_key: None,
        },
        &mut HashSet::new(),
        |_| {},
        |_, _, _, _| async {
            panic!("Chat must never ask for tool approval for unavailable tools");
        },
    )
    .await
    .unwrap();
    let requests = server.await.unwrap();
    let tools = requests[0]["tools"]
        .as_array()
        .expect("Chat exposes web_search");
    assert!(tools.iter().any(|t| t["function"]["name"] == "web_search"));
    assert!(!tools.iter().any(|t| t["function"]["name"] == "create_file"));
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
async fn chat_web_search_requires_approval_and_work_exposes_it() {
    let (endpoint, server) = support::server(vec![
        completion(json!({"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"web_search","arguments":"{\"query\":\"Helsinki weather\"}"}}]})),
        completion(json!({"content":"Denied search, answering from general knowledge."})),
    ])
    .await;
    let mut approvals = 0u32;
    let outcome = run_agent_turn(
        TurnParams {
            base_url: endpoint,
            model_name: "test".into(),
            context_size: 8192,
            app_preset: AppPreset::Chat,
            custom_instructions: String::new(),
            workspace_root: None,
            auto_apply_edits: false,
            enable_thinking: false,
            history: vec![ChatTurnMessage::text(
                "user",
                "What is the weather in Helsinki?",
            )],
            mcp_config: McpConfigFile::default(),
            mcp_tools: vec![],
            max_steps: 2,
            api_key: None,
        },
        &mut HashSet::new(),
        |_| {},
        |_, tool, _, _| {
            approvals += 1;
            assert_eq!(tool, "web_search");
            async { ApprovalDecision::Deny }
        },
    )
    .await
    .unwrap();
    let _ = server.await.unwrap();
    assert_eq!(approvals, 1);
    assert!(outcome.tool_logs[0].is_error);
    assert!(outcome.tool_logs[0]
        .result
        .as_ref()
        .unwrap()
        .contains("denied"));
    assert!(AppPreset::General.allowed_tools().contains(&"web_search"));
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
            api_key: None,
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
    assert!(tools.iter().any(|t| t["function"]["name"] == "web_search"));
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

#[tokio::test]
async fn repeated_sql_reminder_keeps_the_tool_result_and_a_single_initial_system_message() {
    let root = std::env::temp_dir().join(format!("keel_repeat_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("orders.csv"),
        "quantity,price\n3,12\n8,2.50\n2,15\n",
    )
    .unwrap();
    let mut replies: Vec<_> = (0..3).map(|i| {
        completion(json!({"tool_calls":[{"index":0,"id":format!("call_{i}"),"type":"function","function":{"name":"run_sql","arguments":json!({"query":"SELECT SUM(quantity * price) FROM \"orders.csv\""}).to_string()}}]}))
    }).collect();
    replies.push(completion(json!({"content":"€86"})));
    let (endpoint, server) = support::server(replies).await;
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
            history: vec![ChatTurnMessage::text(
                "user",
                "What is the total of orders.csv?",
            )],
            mcp_config: McpConfigFile::default(),
            mcp_tools: vec![],
            max_steps: 4,
            api_key: None,
        },
        &mut HashSet::new(),
        |_| {},
        |_, _, _, _| async { ApprovalDecision::Deny },
    )
    .await
    .unwrap();
    let requests = server.await.unwrap();
    assert_eq!(outcome.content, "€86");
    let messages = requests[3]["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert!(messages
        .iter()
        .skip(1)
        .all(|message| message["role"] != "system"));
    let last = messages.last().unwrap();
    assert_eq!(last["role"], "tool");
    assert_eq!(last["tool_call_id"], "call_2");
    assert!(last["content"]
        .as_str()
        .unwrap()
        .contains("86\n\n\nKeel tool-loop note:"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_general_sessions_still_map_to_work_and_code_has_shell() {
    let preset: AppPreset = serde_json::from_str("\"general\"").unwrap();
    assert_eq!(preset.mode(), "work");
    assert!(AppPreset::Code.allowed_tools().contains(&"run_shell"));
    assert!(!AppPreset::Chat.allowed_tools().contains(&"read_file"));
    assert!(AppPreset::Chat.allowed_tools().contains(&"web_search"));
}
