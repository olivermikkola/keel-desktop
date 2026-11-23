use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurnMessage {
    pub role: String,
    pub content: String,
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
            // Budget roughly for vision tokens when image parts are attached.
            let images = m.image_urls.len() * 768;
            (m.content.len() + tc) / 4 + 8 + images
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
        let non_tool_cost = estimate_tokens(
            &out.iter()
                .filter(|m| m.role != "tool")
                .cloned()
                .collect::<Vec<_>>(),
        );
        let tool_count = out.iter().filter(|m| m.role == "tool").count().max(1);
        let each = (budget_tokens.saturating_sub(non_tool_cost + tool_count * 32) * 4 / tool_count)
            .clamp(128, 8000);
        for message in out.iter_mut().filter(|m| m.role == "tool") {
            if message.content.chars().count() > each {
                message.content = format!("{}\n[Tool output shortened to fit context. Read a smaller line range or query fewer rows for details.]",message.content.chars().take(each).collect::<String>());
                changed = true;
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
}
