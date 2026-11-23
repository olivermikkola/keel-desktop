use keel_core::agent::{run_agent_turn, ApprovalDecision, TurnParams};
use keel_core::apps::AppPreset;
use keel_core::compaction::ChatTurnMessage;
use keel_core::mcp::McpConfigFile;
use serde_json::json;
use std::collections::HashSet;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn chat_completions_sends_authorization_bearer_when_key_set() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let header_end;
        loop {
            let mut buf = [0; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                header_end = index + 4;
                break;
            }
        }
        let header = String::from_utf8_lossy(&bytes[..header_end]).to_string();
        let length = header
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length:")
                    .map(|s| s.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        while bytes.len() < header_end + length {
            let mut buf = [0; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
        }
        let body = format!(
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"ok"}}]})
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(response.as_bytes()).await;
        header
    });

    let outcome = run_agent_turn(
        TurnParams {
            base_url: format!("http://{addr}/v1"),
            model_name: "remote-model".into(),
            context_size: 4096,
            app_preset: AppPreset::Chat,
            custom_instructions: String::new(),
            workspace_root: None,
            auto_apply_edits: false,
            enable_thinking: false,
            history: vec![ChatTurnMessage::text("user", "Hello")],
            mcp_config: McpConfigFile::default(),
            mcp_tools: vec![],
            max_steps: 1,
            api_key: Some("sk-chat-fixture".into()),
        },
        &mut HashSet::new(),
        |_| {},
        |_, _, _, _| async { ApprovalDecision::Deny },
    )
    .await
    .unwrap();
    assert_eq!(outcome.content, "ok");
    let header = server.await.unwrap();
    assert!(
        header.contains("Authorization: Bearer sk-chat-fixture"),
        "missing auth header in: {header}"
    );
    assert!(header.contains("POST /v1/chat/completions"));
}
