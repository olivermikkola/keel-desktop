use anyhow::{bail, Context, Result};

/// Keep UTF-8 bytes intact until an entire SSE data line is available.
#[derive(Default)]
pub struct SseLines {
    pending: Vec<u8>,
}
impl SseLines {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > 2 * 1024 * 1024 {
            bail!("Your AI sent a reply chunk that was too large.");
        }
        let mut result = Vec::new();
        while let Some(index) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<_> = self.pending.drain(..=index).collect();
            let line = std::str::from_utf8(&line)
                .context("Your AI sent an unreadable response.")?
                .trim();
            if let Some(data) = line.strip_prefix("data:") {
                if !data.trim().is_empty() {
                    result.push(data.trim().to_string());
                }
            }
        }
        Ok(result)
    }
    pub fn finish(&mut self) -> Result<Vec<String>> {
        self.push(b"\n")
    }
}

/// Tags can straddle arbitrary model-token boundaries. Tool JSON stays out of the reply.
#[derive(Default)]
pub struct TextRouter {
    pending: String,
    mode: u8,
    in_code_fence: bool,
    backtick_run: usize,
}
impl TextRouter {
    pub fn push(&mut self, text: &str) -> Vec<(bool, String)> {
        const TAGS: [&str; 4] = ["<think>", "</think>", "<tool_call>", "</tool_call>"];
        self.pending.push_str(text);
        let mut result: Vec<(bool, String)> = Vec::new();
        while !self.pending.is_empty() {
            if !self.in_code_fence {
                if let Some(tag) = TAGS.iter().find(|tag| self.pending.starts_with(**tag)) {
                    self.mode = match *tag {
                        "<think>" => 1,
                        "<tool_call>" => 2,
                        _ => 0,
                    };
                    self.pending.drain(..tag.len());
                    self.backtick_run = 0;
                    continue;
                }
                if TAGS.iter().any(|tag| tag.starts_with(&self.pending)) {
                    break;
                }
            }

            // Advance in chunks up to the next `<` or `` ` `` (avoiding O(N^2) single-char drain).
            let next_special = self
                .pending
                .char_indices()
                .skip(1)
                .find(|&(_, ch)| ch == '<' || ch == '`')
                .map(|(idx, _)| idx)
                .unwrap_or(self.pending.len());
            let first = self.pending.chars().next().unwrap();
            let take_bytes = if first == '<' || first == '`' {
                first.len_utf8()
            } else {
                next_special
            };
            let chunk: String = self.pending.drain(..take_bytes).collect();
            if self.mode == 0 {
                if chunk == "`" {
                    self.backtick_run += 1;
                    if self.backtick_run == 3 {
                        self.in_code_fence = !self.in_code_fence;
                        self.backtick_run = 0;
                    }
                } else {
                    self.backtick_run = 0;
                }
            }
            if self.mode == 2 {
                continue;
            }
            let thinking = self.mode == 1;
            if let Some(last) = result.last_mut().filter(|last| last.0 == thinking) {
                last.1.push_str(&chunk);
            } else {
                result.push((thinking, chunk));
            }
        }
        result
    }
    pub fn finish(&mut self) -> Vec<(bool, String)> {
        if self.pending.is_empty() || self.mode == 2 {
            self.pending.clear();
            return vec![];
        }
        vec![(self.mode == 1, std::mem::take(&mut self.pending))]
    }
}
