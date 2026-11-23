//! Three short questions that say whether a model is fit to offer.
//!
//! The questions stay inside Keel: a saved preference, a number written in a
//! note, and a question that note does not answer. Nothing here is a benchmark
//! suite. A model that fails is not offered as the one for this computer.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::hardware::lighter_model_id;

const MARCH_TOTAL: &str = "1842";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCheckItem {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCheckReport {
    pub model_id: String,
    pub passed: bool,
    pub seconds: u64,
    pub summary: String,
    pub items: Vec<ModelCheckItem>,
}

#[derive(Clone, Copy)]
struct CheckCase {
    name: &'static str,
    system: &'static str,
    user: &'static str,
    grade: fn(&str) -> bool,
    pass_detail: &'static str,
    fail_detail: &'static str,
}

fn cases() -> [CheckCase; 3] {
    [
        CheckCase {
            name: "Saved preference",
            system: "The person is vegetarian. Never suggest meat, fish, or poultry. Answer with one dinner only.",
            user: "What is one dinner I would eat?",
            grade: follows_saved_preference,
            pass_detail: "It followed a saved preference.",
            fail_detail: "It ignored a saved preference.",
        },
        CheckCase {
            name: "Number in a note",
            system: "Answer from the note only. Reply with the number only.",
            user: "Note: March total is 1842 euros. What is the March total?",
            grade: reads_the_number,
            pass_detail: "It found the number in the note.",
            fail_detail: "It missed the number in the note.",
        },
        CheckCase {
            name: "Missing answer",
            system: "Answer from the note only. If the note does not contain the answer, say that it is not in the document.",
            user: "Note: March total is 1842 euros. What is the office phone number?",
            grade: admits_it_is_missing,
            pass_detail: "It said when the note did not have the answer.",
            fail_detail: "It invented an answer that was not in the note.",
        },
    ]
}

pub fn follows_saved_preference(answer: &str) -> bool {
    let text = answer.trim();
    if text.chars().count() < 3 {
        return false;
    }
    let lower = text.to_lowercase();
    !MEAT.iter().any(|word| contains_word(&lower, word))
}

pub fn reads_the_number(answer: &str) -> bool {
    let numbers = digit_runs(answer);
    numbers.iter().any(|n| n == MARCH_TOTAL) && numbers.iter().all(|n| n == MARCH_TOTAL)
}

pub fn admits_it_is_missing(answer: &str) -> bool {
    let lower = answer.to_lowercase();
    let honest = [
        "not in the document",
        "not in the note",
        "isn't in the",
        "is not in the",
        "not mentioned",
        "does not contain",
        "doesn't contain",
        "do not see",
        "don't see",
        "cannot find",
        "can't find",
        "no phone",
        "not provided",
        "not listed",
        "not included",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase));
    if !honest {
        return false;
    }
    let numbers = digit_runs(answer);
    let long = numbers.iter().any(|n| n.len() >= 5);
    let phone_like = numbers.iter().filter(|n| n.len() >= 3).count() >= 2;
    !long && !phone_like
}

const MEAT: &[&str] = &[
    "chicken", "beef", "pork", "fish", "steak", "bacon", "ham", "turkey", "lamb", "shrimp",
    "salmon", "tuna", "meat", "sausage", "burger", "poultry", "seafood",
];

fn contains_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == word || (word.len() >= 4 && part.contains(word)))
}

fn digit_runs(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
        } else if !current.is_empty() {
            runs.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs
}

/// Walks to a lighter model when the hardware pick has already failed the check.
/// The lightest model is still returned when every model has failed, so setup is
/// never empty. Callers label that model as not having passed.
pub fn offered_model_id(recommended: &str, failed: &[String]) -> String {
    let mut current = recommended.to_string();
    for _ in 0..4 {
        if !failed.iter().any(|id| id == &current) {
            return current;
        }
        match lighter_model_id(&current) {
            Some(next) => current = next.to_string(),
            None => return current,
        }
    }
    current
}

pub fn completion_text(body: &Value) -> String {
    let message = &body["choices"][0]["message"];
    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !content.is_empty() {
        return content.to_string();
    }
    message
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

pub async fn run_model_check(base_url: &str, model_id: &str) -> Result<ModelCheckReport> {
    run_model_check_with_auth(base_url, model_id, None).await
}

/// Same as [`run_model_check`], optionally sending `Authorization: Bearer <key>`
/// for an explicitly connected remote text provider.
pub async fn run_model_check_with_auth(
    base_url: &str,
    model_id: &str,
    api_key: Option<&str>,
) -> Result<ModelCheckReport> {
    let started = Instant::now();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(45))
        .build()
        .context("Could not start the model check")?;
    let mut items = Vec::with_capacity(3);
    for case in cases() {
        let graded =
            match ask_model(&client, base_url, model_id, case.system, case.user, api_key).await {
                Ok(answer) => {
                    let passed = (case.grade)(&answer);
                    ModelCheckItem {
                        name: case.name.to_string(),
                        passed,
                        detail: (if passed {
                            case.pass_detail
                        } else {
                            case.fail_detail
                        })
                        .to_string(),
                    }
                }
                Err(_) => ModelCheckItem {
                    name: case.name.to_string(),
                    passed: false,
                    detail: "This question didn't finish.".to_string(),
                },
            };
        items.push(graded);
    }
    Ok(finish_report(model_id, items, started.elapsed()))
}

fn finish_report(
    model_id: &str,
    items: Vec<ModelCheckItem>,
    elapsed: Duration,
) -> ModelCheckReport {
    let passed = items.iter().all(|item| item.passed);
    let seconds = elapsed.as_secs();
    let summary = if passed {
        if seconds <= 1 {
            "This model passed the check.".to_string()
        } else {
            format!("This model passed the check in {seconds} seconds.")
        }
    } else {
        "This model didn't pass the check. Keel will offer a lighter one when it can.".to_string()
    };
    ModelCheckReport {
        model_id: model_id.to_string(),
        passed,
        seconds,
        summary,
        items,
    }
}

async fn ask_model(
    client: &reqwest::Client,
    base_url: &str,
    model_id: &str,
    system: &str,
    user: &str,
    api_key: Option<&str>,
) -> Result<String> {
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model_id,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user}
        ],
        "stream": false,
        "temperature": 0,
        "max_tokens": 80,
        "chat_template_kwargs": { "enable_thinking": false }
    });
    let response = post_completion(client, &endpoint, &body, api_key).await?;
    let text = completion_text(&response);
    if text.is_empty() {
        anyhow::bail!("The model returned an empty answer");
    }
    Ok(text)
}

fn authorized_post(
    client: &reqwest::Client,
    endpoint: &str,
    body: &Value,
    api_key: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut req = client.post(endpoint).json(body);
    if let Some(auth) = crate::remote::bearer_authorization(api_key) {
        req = req.header(reqwest::header::AUTHORIZATION, auth);
    }
    req
}

async fn post_completion(
    client: &reqwest::Client,
    endpoint: &str,
    body: &Value,
    api_key: Option<&str>,
) -> Result<Value> {
    let first = authorized_post(client, endpoint, body, api_key)
        .send()
        .await
        .context("The model didn't answer")?;
    let status = first.status();
    if matches!(status.as_u16(), 400 | 422) {
        let mut plain = body.clone();
        if let Some(obj) = plain.as_object_mut() {
            obj.remove("chat_template_kwargs");
        }
        let retry = authorized_post(client, endpoint, &plain, api_key)
            .send()
            .await
            .context("The model didn't answer")?;
        if retry.status().is_success() {
            return retry
                .json()
                .await
                .context("The model returned an unreadable answer");
        }
    }
    if !status.is_success() {
        anyhow::bail!("The model didn't answer");
    }
    first
        .json()
        .await
        .context("The model returned an unreadable answer")
}

fn checks_path(data: &Path) -> std::path::PathBuf {
    data.join("model-checks.json")
}

pub fn load_checks(data: &Path) -> BTreeMap<String, ModelCheckReport> {
    let raw = std::fs::read_to_string(checks_path(data)).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_report(data: &Path, report: &ModelCheckReport) -> Result<()> {
    let mut book = load_checks(data);
    book.insert(report.model_id.clone(), report.clone());
    if let Some(parent) = checks_path(data).parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(&book)?;
    std::fs::write(checks_path(data), raw)?;
    Ok(())
}

pub fn failed_model_ids(data: &Path) -> Vec<String> {
    load_checks(data)
        .into_iter()
        .filter(|(_, report)| !report.passed)
        .map(|(id, _)| id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_rejects_meat_words_but_not_a_vegetable_dinner() {
        assert!(follows_saved_preference("Vegetable pasta"));
        assert!(!follows_saved_preference("Grilled chicken"));
        assert!(!follows_saved_preference("A hamburger"));
        assert!(!follows_saved_preference("ok"));
        assert!(follows_saved_preference("Lentil soup"));
    }

    #[test]
    fn number_must_be_the_one_in_the_note() {
        assert!(reads_the_number("1842"));
        assert!(reads_the_number("The March total is 1842 euros."));
        assert!(!reads_the_number("18420"));
        assert!(!reads_the_number("1842 or maybe 1900"));
        assert!(!reads_the_number("I don't know"));
    }

    #[test]
    fn missing_answer_must_admit_the_gap_without_inventing_a_phone() {
        assert!(admits_it_is_missing("It is not in the document."));
        assert!(admits_it_is_missing(
            "The note does not contain a phone number. The March total is 1842 euros."
        ));
        assert!(!admits_it_is_missing("The office number is 555-0100."));
        assert!(!admits_it_is_missing("Call 5551234."));
        assert!(!admits_it_is_missing("1842"));
    }

    #[test]
    fn a_failed_model_is_not_the_one_we_offer() {
        assert_eq!(offered_model_id("bonsai-9b", &[]), "bonsai-9b");
        assert_eq!(
            offered_model_id("bonsai-9b", &["bonsai-9b".into()]),
            "bonsai-4b"
        );
        assert_eq!(
            offered_model_id("bonsai-9b", &["bonsai-9b".into(), "bonsai-4b".into()]),
            "lfm3-1.7b"
        );
        assert_eq!(
            offered_model_id(
                "bonsai-4b",
                &["bonsai-9b".into(), "bonsai-4b".into(), "lfm3-1.7b".into()]
            ),
            "lfm3-1.7b"
        );
    }

    #[test]
    fn completion_text_prefers_the_answer_over_reasoning() {
        let body =
            json!({"choices":[{"message":{"content":"1842","reasoning_content":"thinking"}}]});
        assert_eq!(completion_text(&body), "1842");
        let only_reason =
            json!({"choices":[{"message":{"content":"","reasoning_content":"pasta"}}]});
        assert_eq!(completion_text(&only_reason), "pasta");
    }

    #[test]
    fn saved_report_round_trips_and_marks_a_failure() {
        let dir = std::env::temp_dir().join(format!("keel-checks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = ModelCheckReport {
            model_id: "bonsai-4b".into(),
            passed: false,
            seconds: 12,
            summary: "no".into(),
            items: vec![ModelCheckItem {
                name: "Saved preference".into(),
                passed: false,
                detail: "It ignored a saved preference.".into(),
            }],
        };
        save_report(&dir, &report).unwrap();
        assert_eq!(load_checks(&dir).get("bonsai-4b"), Some(&report));
        assert_eq!(failed_model_ids(&dir), vec!["bonsai-4b".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
