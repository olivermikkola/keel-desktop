use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurnMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
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
            (m.content.len() + tc) / 4 + 8
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
    if messages.len() <= keep_tail + 2 || estimate_tokens(&messages) <= budget_tokens {
        return (messages, false);
    }

    let split_idx = messages.len().saturating_sub(keep_tail);
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

    let summary_msg = ChatTurnMessage {
        role: "system".to_string(),
        content: format!(
            "Earlier conversation summary (compacted to fit context):\n{}",
            summary_lines.join("\n")
        ),
        tool_call_id: None,
        tool_calls: None,
    };

    let mut out = Vec::with_capacity(2 + tail.len());
    out.extend_from_slice(head);
    out.push(summary_msg);
    out.extend_from_slice(tail);
    (out, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacts_when_exceeding_budget() {
        let mut msgs = vec![ChatTurnMessage {
            role: "system".to_string(),
            content: "You are Keel.".to_string(),
            tool_call_id: None,
            tool_calls: None,
        }];
        for i in 0..12 {
            msgs.push(ChatTurnMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                content: "x".repeat(400),
                tool_call_id: None,
                tool_calls: None,
            });
        }
        let (compacted, did_compact) = compact_messages(msgs, 500, 4);
        assert!(did_compact);
        assert_eq!(compacted.len(), 6);
        assert_eq!(compacted[0].content, "You are Keel.");
        assert!(compacted[1]
            .content
            .contains("Earlier conversation summary"));
    }
}
