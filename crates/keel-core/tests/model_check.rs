mod support;

use keel_core::model_check::run_model_check;
use serde_json::json;
use std::time::Duration;

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
