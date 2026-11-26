use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurnMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
    /// Image data URLs for OpenAI-compatible multimodal `image_url` parts.
    /// Skipped in ordinary JSON; `to_api_value` expands them into content parts.
    #[serde(default, skip_serializing)]
    pub image_urls: Vec<String>,
}

impl ChatTurnMessage {
    pub fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            reasoning_content: None,
            tool_call_id: None,
            tool_calls: None,
            image_urls: Vec::new(),
        }
    }

    /// OpenAI chat-completions message object, including multimodal content when present.
    pub fn to_api_value(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("role".into(), Value::String(self.role.clone()));
        if self.image_urls.is_empty() {
            obj.insert("content".into(), Value::String(self.content.clone()));
        } else {
            let mut parts = vec![json!({"type": "text", "text": self.content})];
            for url in &self.image_urls {
                parts.push(json!({
                    "type": "image_url",
                    "image_url": { "url": url }
                }));
            }
            obj.insert("content".into(), Value::Array(parts));
        }
        if let Some(reasoning) = self
            .reasoning_content
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            obj.insert(
                "reasoning_content".into(),
                Value::String(reasoning.to_string()),
            );
        }
        if let Some(id) = &self.tool_call_id {
            obj.insert("tool_call_id".into(), Value::String(id.clone()));
        }
        if let Some(calls) = &self.tool_calls {
            obj.insert("tool_calls".into(), calls.clone());
        }
        Value::Object(obj)
    }

    pub fn strip_images(&mut self) {
        self.image_urls.clear();
    }

    pub fn has_images(&self) -> bool {
        !self.image_urls.is_empty()
    }
}

/// Rough token estimate (~4 characters per token, conservative for small local contexts).
pub fn estimate_tokens(messages: &[ChatTurnMessage]) -> usize {
    messages
        .iter()
        .map(|m| {
            let tc = m
                .tool_calls
                .as_ref()
                .map(|v| v.to_string().len())
                .unwrap_or(0);
            let reasoning = m.reasoning_content.as_ref().map(|r| r.len()).unwrap_or(0);
            // Budget roughly for vision tokens when image parts are attached.
            let images = m.image_urls.len() * 768;
            (m.content.len() + tc + reasoning) / 4 + 8 + images
        })
        .sum()
}

/// Compacts older middle turns into a deterministic summary when approaching `budget_tokens`,
/// always preserving the system prompt (index 0) and the most recent `keep_tail` messages.
pub fn compact_messages(
    messages: Vec<ChatTurnMessage>,
    budget_tokens: usize,
    keep_tail: usize,
) -> (Vec<ChatTurnMessage>, bool) {
    if estimate_tokens(&messages) <= budget_tokens {
        return (messages, false);
    }

    let original_len = messages.len();
    let mut split_idx = messages.len().saturating_sub(keep_tail.max(1)).max(1);
    while split_idx > 1 && messages[split_idx].role != "user" {
        split_idx -= 1;
    }
    let head = &messages[..1];
    let middle = &messages[1..split_idx];
    let tail = &messages[split_idx..];

    let mut summary_lines = Vec::new();
    for m in middle {
        let snippet: String = m.content.chars().take(160).collect();
        let snippet = snippet.replace('\n', " ");
        if !snippet.trim().is_empty() {
            summary_lines.push(format!("- {}: {}", m.role, snippet.trim()));
        }
    }
    if summary_lines.len() > 16 {
        let omitted = summary_lines.len() - 16;
        summary_lines.truncate(16);
        summary_lines.push(format!("- ... ({omitted} earlier steps compacted)"));
    }

    let mut out = Vec::with_capacity(1 + tail.len());
    out.extend_from_slice(head);
    out.extend_from_slice(tail);
    let mut changed = out.len() < original_len;
    // Drop complete older turns when snippets plus the tail still exceed the budget.
    while estimate_tokens(&out) > budget_tokens {
        let latest_user = out.iter().rposition(|m| m.role == "user").unwrap_or(1);
        if latest_user <= 1 {
            break;
        }
        let end = out
            .iter()
            .enumerate()
            .skip(2)
            .find(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
            .unwrap_or(latest_user);
        out.drain(1..end);
        changed = true;
    }
    if estimate_tokens(&out) > budget_tokens {
        // Protect tool outputs from the latest assistant step so the model always sees
        // what it just requested. Compact oldest tool outputs first into a fixed,
        // stable stub so already-compacted prefixes stay KV-cache-friendly.
        const STABLE_STUB_CHARS: usize = 420;
        const STUB_SUFFIX: &str =
            "\n[Earlier tool output compacted to save context. Re-read with a smaller line range if needed.]";
        let last_assistant = out.iter().rposition(|m| m.role == "assistant");
        let newest_tool_start = last_assistant.map(|idx| idx + 1).unwrap_or(out.len());

        // Pass 1: compact older tool results (before the latest assistant step) oldest-first.
        for idx in 0..newest_tool_start {
            if estimate_tokens(&out) <= budget_tokens {
                break;
            }
            if out[idx].role == "tool"
                && !out[idx].content.ends_with(STUB_SUFFIX)
                && out[idx].content.chars().count() > STABLE_STUB_CHARS + STUB_SUFFIX.len()
            {
                let head: String = out[idx].content.chars().take(STABLE_STUB_CHARS).collect();
                out[idx].content = format!("{head}{STUB_SUFFIX}");
                changed = true;
            }
        }

        // Pass 2: if still over budget, drop reasoning_content from older assistant steps
        // (keeping the most recent assistant step's reasoning intact when possible).
        if estimate_tokens(&out) > budget_tokens {
            let keep_reasoning_idx = last_assistant.unwrap_or(out.len());
            for (idx, message) in out.iter_mut().enumerate() {
                if idx == keep_reasoning_idx {
                    continue;
                }
                if message.role == "assistant" && message.reasoning_content.is_some() {
                    message.reasoning_content = None;
                    changed = true;
                }
            }
        }

        // Pass 3: only if even the latest tool outputs alone blow past the budget,
        // trim them proportionally to fit without dropping the user request.
        if estimate_tokens(&out) > budget_tokens {
            let non_tool_cost = estimate_tokens(
                &out.iter()
                    .filter(|m| m.role != "tool")
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            let recent_tools: Vec<usize> = (newest_tool_start..out.len())
                .filter(|&i| out[i].role == "tool")
                .collect();
            let tool_count = recent_tools.len().max(1);
            let each = (budget_tokens.saturating_sub(non_tool_cost + tool_count * 32) * 4
                / tool_count)
                .clamp(256, 12_000);
            for idx in recent_tools {
                if out[idx].content.chars().count() > each {
                    let head: String = out[idx].content.chars().take(each).collect();
                    out[idx].content = format!(
                        "{head}\n[Tool output shortened to fit context. Read a smaller line range or query fewer rows for details.]"
                    );
                    changed = true;
                }
            }
        }
    }
    // Keep history excerpts at user authority. Several local templates accept
    // a single system message and reject one anywhere else in the transcript.
    if !summary_lines.is_empty() {
        let intro = "Quoted excerpts from earlier conversation:\n";
        let separator = "\n\nMessage at this point in the conversation:\n";
        let mut room = budget_tokens
            .saturating_sub(estimate_tokens(&out))
            .saturating_mul(4)
            .saturating_sub(intro.len() + separator.len() + 4);
        let excerpts: Vec<_> = summary_lines
            .iter()
            .take_while(|line| {
                let needed = line.len() + 1;
                if needed > room {
                    return false;
                }
                room -= needed;
                true
            })
            .collect();
        if !excerpts.is_empty() {
            if let Some(user) = out.iter_mut().find(|message| message.role == "user") {
                let excerpts = excerpts
                    .into_iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join("\n");
                user.content = format!("{intro}{excerpts}{separator}{}", user.content);
                changed = true;
            }
        }
    }
    (out, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_when_exceeding_budget() {
        let mut msgs = vec![ChatTurnMessage::text("system", "You are Keel.")];
        for i in 0..12 {
            msgs.push(ChatTurnMessage::text(
                if i % 2 == 0 { "user" } else { "assistant" },
                "x".repeat(400),
            ));
        }
        let (compacted, did_compact) = compact_messages(msgs, 500, 4);
        assert!(did_compact);
        assert!(estimate_tokens(&compacted) <= 500);
        assert!(compacted.len() >= 5);
        assert_eq!(compacted[0].content, "You are Keel.");
        assert!(compacted[1..]
            .iter()
            .all(|message| message.role != "system"));
        assert_eq!(compacted.last().unwrap().content, "x".repeat(400));
    }

    #[test]
    fn multimodal_api_value_includes_image_parts() {
        let mut message = ChatTurnMessage::text("user", "What is in this picture?");
        message.image_urls = vec!["data:image/png;base64,abc".into()];
        let value = message.to_api_value();
        let parts = value["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,abc");
    }

    #[test]
    fn compacts_oldest_tool_outputs_first_and_preserves_newest_and_reasoning() {
        let mut msgs = vec![
            ChatTurnMessage::text("system", "You are Keel."),
            ChatTurnMessage::text("user", "Inspect the project."),
            ChatTurnMessage {
                role: "assistant".into(),
                content: String::new(),
                reasoning_content: Some("First I will list files.".into()),
                tool_call_id: None,
                tool_calls: Some(
                    json!([{"id":"c1","type":"function","function":{"name":"list_files","arguments":"{}"}}]),
                ),
                image_urls: vec![],
            },
            ChatTurnMessage {
                role: "tool".into(),
                content: "A".repeat(4000),
                reasoning_content: None,
                tool_call_id: Some("c1".into()),
                tool_calls: None,
                image_urls: vec![],
            },
            ChatTurnMessage {
                role: "assistant".into(),
                content: String::new(),
                reasoning_content: Some("Now read main.lua.".into()),
                tool_call_id: None,
                tool_calls: Some(
                    json!([{"id":"c2","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"main.lua\"}"}}]),
                ),
                image_urls: vec![],
            },
            ChatTurnMessage {
                role: "tool".into(),
                content: "NEWEST_FILE_CONTENT_12345".repeat(20),
                reasoning_content: None,
                tool_call_id: Some("c2".into()),
                tool_calls: None,
                image_urls: vec![],
            },
        ];
        // Verify reasoning_content serializes into OpenAI API value.
        assert_eq!(
            msgs[2].to_api_value()["reasoning_content"],
            "First I will list files."
        );

        let (compacted, did_compact) = compact_messages(msgs.clone(), 600, 6);
        assert!(did_compact);
        // Oldest tool result compacted to stub, while newest tool result stays 100% intact.
        assert!(compacted[3]
            .content
            .contains("Earlier tool output compacted"));
        assert_eq!(compacted[5].content, msgs[5].content);
        assert_eq!(
            compacted[4].reasoning_content.as_deref(),
            Some("Now read main.lua.")
        );

        // Adding another step keeps the already-compacted oldest tool stub byte-identical (KV stable).
        let stub_before = compacted[3].content.clone();
        msgs = compacted;
        msgs.push(ChatTurnMessage::text("assistant", ""));
        msgs.push(ChatTurnMessage {
            role: "tool".into(),
            content: "NEXT_RESULT".repeat(40),
            reasoning_content: None,
            tool_call_id: Some("c3".into()),
            tool_calls: None,
            image_urls: vec![],
        });
        let (compacted2, _) = compact_messages(msgs, 600, 6);
        assert_eq!(compacted2[3].content, stub_before);
    }
}
