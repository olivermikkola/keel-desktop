//! A durable draft keeps partial output and staged edits when generation is stopped.
use crate::agent::AgentEvent;
use crate::sessions::{ChatSession, UiMessage};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct TurnRecorder {
    data: PathBuf,
    pub session: ChatSession,
    last_save: Instant,
}
impl TurnRecorder {
    pub fn new(data: &Path, mut session: ChatSession) -> anyhow::Result<Self> {
        session.messages.push(UiMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: "assistant".into(),
            content: String::new(),
            thinking: None,
            tool_calls: vec![],
            attachments: vec![],
            status: "streaming".into(),
        });
        crate::sessions::save_session(data, &session)?;
        Ok(Self {
            data: data.to_path_buf(),
            session,
            last_save: Instant::now(),
        })
    }
    pub fn record(&mut self, event: &AgentEvent) -> anyhow::Result<()> {
        let message = self.session.messages.last_mut().expect("draft message");
        let durable = match event {
            AgentEvent::ContentDelta { text } => {
                message.content.push_str(text);
                false
            }
            AgentEvent::ThinkingDelta { text } => {
                message.thinking.get_or_insert_default().push_str(text);
                false
            }
            AgentEvent::ToolCallStart { name, arguments } => {
                message.tool_calls.push(crate::sessions::UiToolLog {
                    name: name.clone(),
                    arguments: arguments.clone(),
                    result: None,
                    is_error: false,
                    edit_id: None,
                });
                false
            }
            AgentEvent::ToolCallResult {
                name,
                result,
                is_error,
                edit_id,
            } => {
                if let Some(tool) = message
                    .tool_calls
                    .iter_mut()
                    .rev()
                    .find(|t| t.name == *name && t.result.is_none())
                {
                    tool.result = Some(result.clone());
                    tool.is_error = *is_error;
                    tool.edit_id = edit_id.clone();
                }
                true
            }
            AgentEvent::StagedEditAdded { edit } => {
                self.session.staged_edits.push(edit.clone());
                true
            }
            AgentEvent::TurnFinished {
                content, thinking, ..
            } => {
                message.content = content.clone();
                message.thinking = thinking.clone();
                message.status = "complete".into();
                true
            }
            AgentEvent::ApprovalRequired { .. } => true,
            _ => false,
        };
        if durable || self.last_save.elapsed() >= Duration::from_millis(250) {
            crate::sessions::save_session(&self.data, &self.session)?;
            self.last_save = Instant::now();
        }
        Ok(())
    }
    pub fn fail(&mut self, reason: &str) -> anyhow::Result<()> {
        if let Some(message) = self.session.messages.last_mut() {
            message.status = "failed".into();
            if !message.content.is_empty() {
                message.content.push_str("\n\n");
            }
            message.content.push_str(reason);
        }
        crate::sessions::save_session(&self.data, &self.session)
    }
}
