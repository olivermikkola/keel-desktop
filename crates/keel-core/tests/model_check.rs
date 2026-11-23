mod support;

use keel_core::model_check::{run_model_check, run_model_check_with_auth};
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn answer(text: &str) -> (String, String, Duration) {
    (
        "application/json".into(),
        json!({"choices":[{"message":{"content": text}}]}).to_string(),
        Duration::from_millis(1),
    )
}

#[tokio::test]
async fn three_honest_answers_pass_and_the_request_keeps_thinking_off() {
    let (base, handle) = support::server(vec![
        answer("Lentil soup"),
        answer("1842"),
        answer("It is not in the document."),
    ])
    .await;
    let report = run_model_check(&format!("{base}/v1"), "bonsai-4b")
        .await
        .unwrap();
    let requests = handle.await.unwrap();
    assert!(report.passed, "{report:?}");
    assert_eq!(report.items.len(), 3);
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[0]["chat_template_kwargs"]["enable_thinking"],
        false
    );
    assert_eq!(requests[0]["max_tokens"], 80);
    assert!(requests[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("vegetarian"));
}

#[tokio::test]
async fn a_meat_dinner_fails_the_check_without_hiding_which_question() {
    let (base, handle) = support::server(vec![
        answer("Grilled chicken"),
        answer("1842"),
        answer("It is not in the document."),
    ])
    .await;
    let report = run_model_check(&format!("{base}/v1"), "bonsai-9b")
        .await
        .unwrap();
    handle.await.unwrap();
    assert!(!report.passed);
    assert!(!report.items[0].passed);
    assert!(report.items[0].detail.contains("preference"));
    assert!(report.items[1].passed && report.items[2].passed);
    assert!(report.summary.contains("lighter"));
}

#[tokio::test]
async fn model_check_sends_bearer_when_api_key_is_set() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let answers = [
        json!({"choices":[{"message":{"content":"Lentil soup"}}]}).to_string(),
        json!({"choices":[{"message":{"content":"1842"}}]}).to_string(),
        json!({"choices":[{"message":{"content":"It is not in the document."}}]}).to_string(),
    ];
    let handle = tokio::spawn(async move {
        let mut saw_bearer = 0usize;
        for response in answers.iter() {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8_lossy(&bytes);
            if header
                .to_lowercase()
                .contains("authorization: bearer sk-check-fixture")
            {
                saw_bearer += 1;
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            let _ = socket.write_all(header.as_bytes()).await;
            let _ = socket.write_all(response.as_bytes()).await;
        }
        saw_bearer
    });

    let report = run_model_check_with_auth(
        &format!("http://{address}/v1"),
        "remote-model",
        Some("sk-check-fixture"),
    )
    .await
    .unwrap();
    let saw_bearer = handle.await.unwrap();
    assert_eq!(saw_bearer, 3, "every check question should send Bearer");
    assert!(report.passed, "{report:?}");
}
