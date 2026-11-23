use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A tool call parsed either from OpenAI structured `tool_calls` or `<tool_call>` fallback text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
}

/// Extracts `<think>...</think>` blocks from model text so UI can show reasoning separately.
/// Preserves literal `<think>` mentions that appear inside fenced code blocks (```).
pub fn split_thinking(raw: &str) -> (String, String) {
    let mut thinking = String::new();
    let mut visible = String::new();
    let mut in_think = false;
    let mut in_fence = false;
    let mut rest = raw;

    while !rest.is_empty() {
        if !in_think {
            let next_fence = rest.find("```");
            let next_think = if in_fence { None } else { rest.find("<think>") };
            match (next_fence, next_think) {
                (Some(f), Some(t)) if f < t => {
                    visible.push_str(&rest[..f + 3]);
                    rest = &rest[f + 3..];
                    in_fence = !in_fence;
                }
                (Some(f), None) => {
                    visible.push_str(&rest[..f + 3]);
                    rest = &rest[f + 3..];
                    in_fence = !in_fence;
                }
                (_, Some(t)) => {
                    visible.push_str(&rest[..t]);
                    rest = &rest[t + "<think>".len()..];
                    in_think = true;
                }
                (None, None) => {
                    visible.push_str(rest);
                    break;
                }
            }
        } else if let Some(end) = rest.find("</think>") {
            if !thinking.is_empty() {
                thinking.push('\n');
            }
            thinking.push_str(rest[..end].trim());
            rest = &rest[end + "</think>".len()..];
            in_think = false;
        } else {
            if !thinking.is_empty() {
                thinking.push('\n');
            }
            thinking.push_str(rest.trim());
            break;
        }
    }
    (thinking.trim().to_string(), visible.trim().to_string())
}

/// Tolerant fallback parser for `<tool_call>{...}</tool_call>` blocks emitted by small local models.
pub fn parse_tool_calls_from_text(text: &str) -> (Vec<ParsedToolCall>, String) {
    let mut calls = Vec::new();
    let mut cleaned = String::new();
    let mut rest = text;
    let mut idx = 0usize;

    while let Some(start) = rest.find("<tool_call>") {
        cleaned.push_str(&rest[..start]);
        let after_start = &rest[start + "<tool_call>".len()..];
        let (block, remainder) = if let Some(end) = after_start.find("</tool_call>") {
            (
                &after_start[..end],
                &after_start[end + "</tool_call>".len()..],
            )
        } else {
            (after_start, "")
        };

        if let Some(parsed) = parse_json_call_object(block.trim(), idx) {
            calls.push(parsed);
            idx += 1;
        } else {
            // Preserve unparseable blocks so nothing is silently lost.
            cleaned.push_str("<tool_call>");
            cleaned.push_str(block);
            if remainder.is_empty() && !after_start.contains("</tool_call>") {
                // nothing
            } else {
                cleaned.push_str("</tool_call>");
            }
        }
        rest = remainder;
    }

    cleaned.push_str(rest);
    (calls, cleaned.trim().to_string())
}

fn strip_json_fence(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("```") {
        let after_lang = rest.split_once('\n').map(|(_, body)| body).unwrap_or(rest);
        if let Some(inner) = after_lang.strip_suffix("```") {
            return inner.trim();
        }
    }
    trimmed
}

fn parse_json_call_object(raw: &str, idx: usize) -> Option<ParsedToolCall> {
    let clean = strip_json_fence(raw);
    let val: serde_json::Value = serde_json::from_str(clean).ok()?;
    let name = val.get("name")?.as_str()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let raw_args = val.get("arguments").or_else(|| val.get("parameters"));
    let (arguments, parse_error) = match raw_args {
        Some(serde_json::Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                (serde_json::Value::Object(Default::default()), None)
            } else {
                match serde_json::from_str::<serde_json::Value>(trimmed) {
                    Ok(parsed) => (parsed, None),
                    Err(err) => (
                        serde_json::Value::Object(Default::default()),
                        Some(format!("Invalid JSON arguments: {err}")),
                    ),
                }
            }
        }
        Some(other) => (other.clone(), None),
        None => (serde_json::Value::Object(Default::default()), None),
    };
    Some(ParsedToolCall {
        id: format!("fallback-call-{idx}"),
        name,
        arguments,
        parse_error,
    })
}

/// Detects identical repeated tool calls in a turn and returns a reminder message when tripped.
#[derive(Debug, Default)]
pub struct RepeatToolGuard {
    counts: HashMap<String, usize>,
}

impl RepeatToolGuard {
    pub fn record(&mut self, name: &str, args: &serde_json::Value) -> Option<String> {
        let key = format!("{name}:{}", serde_json::to_string(args).unwrap_or_default());
        let count = self.counts.entry(key).or_insert(0);
        *count += 1;
        if *count >= 3 {
            Some(format!(
                "You already called `{name}` with these exact arguments {} times. Use the previous result or try a different approach instead of repeating the same call.",
                *count
            ))
        } else {
            None
        }
    }
}

/// Stops runaway turns after repeated consecutive tool errors.
#[derive(Debug)]
pub struct CircuitBreaker {
    max_consecutive_failures: usize,
    consecutive_failures: usize,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self {
            max_consecutive_failures: 4,
            consecutive_failures: 0,
        }
    }
}

impl CircuitBreaker {
    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    pub fn record_failure(&mut self) -> bool {
        self.consecutive_failures += 1;
        self.is_tripped()
    }

    pub fn is_tripped(&self) -> bool {
        self.consecutive_failures >= self.max_consecutive_failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_thinking_tags() {
        let (think, vis) = split_thinking("<think>step 1\nstep 2</think>\nHello there.");
        assert_eq!(think, "step 1\nstep 2");
        assert_eq!(vis, "Hello there.");
    }

    #[test]
    fn parses_fallback_tool_calls() {
        let raw = r#"Let me check.
<tool_call>{"name":"read_file","arguments":{"path":"README.md"}}</tool_call>"#;
        let (calls, text) = parse_tool_calls_from_text(raw);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].arguments["path"], "README.md");
        assert_eq!(text, "Let me check.");

        let fenced = "<tool_call>\n```json\n{\"name\":\"list_files\",\"parameters\":{\"path\":\".\"}}\n```\n</tool_call>";
        let (calls2, _) = parse_tool_calls_from_text(fenced);
        assert_eq!(calls2.len(), 1);
        assert_eq!(calls2[0].name, "list_files");
        assert_eq!(calls2[0].arguments["path"], ".");
        assert!(calls2[0].parse_error.is_none());

        let bad_args = r#"<tool_call>{"name":"read_file","arguments":"{broken"}</tool_call>"#;
        let (calls3, _) = parse_tool_calls_from_text(bad_args);
        assert_eq!(calls3.len(), 1);
        assert!(calls3[0].parse_error.is_some());
    }

    #[test]
    fn repeat_guard_and_circuit_breaker_trip() {
        let mut guard = RepeatToolGuard::default();
        let args = serde_json::json!({"path": "a.txt"});
        assert!(guard.record("read_file", &args).is_none());
        assert!(guard.record("read_file", &args).is_none());
        assert!(guard.record("read_file", &args).is_some());

        let mut cb = CircuitBreaker::default();
        assert!(!cb.record_failure());
        assert!(!cb.record_failure());
        cb.record_success();
        assert!(!cb.record_failure());
        assert!(!cb.record_failure());
        assert!(!cb.record_failure());
        assert!(!cb.is_tripped());
        assert!(cb.record_failure());
        assert!(cb.is_tripped());
        cb.record_success();
        assert!(!cb.is_tripped());
    }
}
