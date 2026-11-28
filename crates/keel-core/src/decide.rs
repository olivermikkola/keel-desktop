//! Check Documents ("Decide") studio: ask the same Yes/No, multiple-choice,
//! or rating questions across a stack of bills, contracts, letters, or PDFs
//! privately using the active local model, then export a clean spreadsheet.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

pub const MAX_DOCUMENTS: usize = 50;
pub const MAX_QUESTIONS: usize = 10;
pub const MAX_CHUNK_CHARS: usize = 8_000;
pub const UNSURE_THRESHOLD: f64 = 0.65;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckQuestionKind {
    YesNo,
    Choice,
    Scale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckQuestion {
    pub id: String,
    pub kind: CheckQuestionKind,
    pub prompt: String,
    #[serde(default)]
    pub options: Vec<String>,
}

impl CheckQuestion {
    pub fn validate(&self) -> Result<()> {
        let q = self.prompt.trim();
        if q.is_empty() || q.chars().count() > 400 {
            bail!("Each question must be between 1 and 400 characters.");
        }
        match self.kind {
            CheckQuestionKind::YesNo => Ok(()),
            CheckQuestionKind::Choice => {
                let clean: Vec<&str> = self
                    .options
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                if clean.len() < 2 || clean.len() > 12 {
                    bail!("A pick-one question needs between 2 and 12 options.");
                }
                Ok(())
            }
            CheckQuestionKind::Scale => {
                let clean: Vec<&str> = self
                    .options
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                if clean.len() < 2 || clean.len() > 10 {
                    bail!("A rating scale needs between 2 and 10 levels.");
                }
                Ok(())
            }
        }
    }

    pub fn effective_options(&self) -> Vec<String> {
        match self.kind {
            CheckQuestionKind::YesNo => vec!["Yes".into(), "No".into()],
            CheckQuestionKind::Choice | CheckQuestionKind::Scale => self
                .options
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckAnswer {
    pub question_id: String,
    pub answer: String,
    pub confidence: f64,
    pub reason: String,
    pub unsure: bool,
    pub part: Option<usize>,
    pub total_parts: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckDocumentInput {
    pub id: String,
    pub name: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckDocumentResult {
    pub document_id: String,
    pub document_name: String,
    pub answers: Vec<CheckAnswer>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Splits a long document into bounded chunks along line breaks so small local
/// models can read every part without silently truncating the end.
pub fn split_document_chunks(text: &str, max_chars: usize) -> Vec<String> {
    let limit = max_chars.max(200);
    let normalized = text.replace("\r\n", "\n");
    let trimmed = normalized.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.chars().count() <= limit {
        return vec![trimmed.to_string()];
    }
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for line in trimmed.lines() {
        let line_len = line.chars().count();
        if line_len > limit {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_len = 0;
            }
            let chars: Vec<char> = line.chars().collect();
            for slice in chars.chunks(limit) {
                chunks.push(slice.iter().collect());
            }
            continue;
        }
        let add_len = if current.is_empty() {
            line_len
        } else {
            line_len + 1
        };
        if !current.is_empty() && current_len + add_len > limit {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if !current.is_empty() {
            current.push('\n');
            current_len += 1;
        }
        current.push_str(line);
        current_len += line_len;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Builds a concise prompt asking the local model for one grounded answer,
/// confidence percentage, and 1-sentence reason for a single document chunk.
pub fn build_check_prompt(
    doc_name: &str,
    chunk_text: &str,
    question: &CheckQuestion,
) -> (String, String) {
    let options = question.effective_options();
    let options_list = options
        .iter()
        .enumerate()
        .map(|(i, opt)| format!("{}. {opt}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let system = "You carefully check a document and answer one question using ONLY the text shown.\n\
Never guess facts that are not in the document. If the document does not give enough information, set CONFIDENCE below 50%.\n\
Reply using exactly these 3 lines and nothing else:\n\
ANSWER: <exact option from the list>\n\
CONFIDENCE: <number from 0 to 100>%\n\
REASON: <one short sentence quoting or citing the document>";

    let user = format!(
        "Document: {doc_name}\n\n{chunk_text}\n\nQuestion: {}\nAllowed answers:\n{options_list}",
        question.prompt.trim()
    );
    (system.to_string(), user)
}

/// Parses the local model's structured 3-line answer tolerantly.
pub fn parse_check_answer(raw: &str, question: &CheckQuestion) -> CheckAnswer {
    let body = raw
        .rsplit_once("</think>")
        .map(|(_, after)| after)
        .unwrap_or(raw)
        .trim();
    let options = question.effective_options();
    let mut raw_answer = String::new();
    let mut confidence = 0.5_f64;
    let mut reason = String::new();

    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("answer:") {
            let offset = trimmed.len() - rest.len();
            raw_answer = trimmed[offset..].trim().to_string();
        } else if let Some(rest) = lower.strip_prefix("confidence:") {
            let offset = trimmed.len() - rest.len();
            let digits: String = trimmed[offset..]
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(val) = digits.parse::<f64>() {
                confidence = if val > 1.0 {
                    (val / 100.0).clamp(0.0, 1.0)
                } else {
                    val.clamp(0.0, 1.0)
                };
            }
        } else if let Some(rest) = lower.strip_prefix("reason:") {
            let offset = trimmed.len() - rest.len();
            reason = trimmed[offset..].trim().to_string();
        }
    }

    if raw_answer.is_empty() {
        raw_answer = body.lines().next().unwrap_or("").trim().to_string();
    }
    let matched = match_option(&raw_answer, &options);
    let (answer, final_conf) = match matched {
        Some(opt) => (opt, confidence),
        None => (
            options
                .first()
                .cloned()
                .unwrap_or_else(|| "Unsure".to_string()),
            confidence.min(0.4),
        ),
    };
    CheckAnswer {
        question_id: question.id.clone(),
        answer,
        confidence: final_conf,
        reason,
        unsure: final_conf < UNSURE_THRESHOLD,
        part: None,
        total_parts: 1,
    }
}

fn match_option(raw: &str, options: &[String]) -> Option<String> {
    let cleaned = raw
        .trim()
        .trim_matches(['"', '\'', '*', '.', ' '])
        .to_ascii_lowercase();
    if cleaned.is_empty() {
        return None;
    }
    // Exact match
    if let Some(opt) = options.iter().find(|o| o.to_ascii_lowercase() == cleaned) {
        return Some(opt.clone());
    }
    // Numbered match ("1", "2. Bill")
    let leading_digits: String = cleaned.chars().take_while(|c| c.is_ascii_digit()).collect();
    if let Ok(num) = leading_digits.parse::<usize>() {
        if num >= 1 && num <= options.len() {
            return Some(options[num - 1].clone());
        }
    }
    // Substring match (longest option first to avoid partial ambiguity)
    let mut by_len: Vec<&String> = options.iter().collect();
    by_len.sort_by_key(|s| std::cmp::Reverse(s.len()));
    for opt in by_len {
        let lower = opt.to_ascii_lowercase();
        if cleaned.contains(&lower) {
            return Some(opt.clone());
        }
    }
    None
}

/// Combines answers from multiple chunks of a long document by taking the part
/// the model is most confident about.
pub fn combine_chunk_answers(parts: &[CheckAnswer]) -> Option<CheckAnswer> {
    let (best_idx, best) = parts.iter().enumerate().max_by(|(_, a), (_, b)| {
        a.confidence
            .partial_cmp(&b.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;
    let mut combined = best.clone();
    combined.total_parts = parts.len();
    combined.part = if parts.len() > 1 {
        Some(best_idx + 1)
    } else {
        None
    };
    Some(combined)
}

pub fn tsv_cell(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n', '\t'], " ")
}

/// Sanitizes a CSV cell (RFC 4180 + spreadsheet formula neutralization).
pub fn csv_cell(text: &str) -> String {
    let mut clean = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
    if clean
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@'))
    {
        clean.insert(0, '\'');
    }
    if clean.contains(',') || clean.contains('"') {
        format!("\"{}\"", clean.replace('"', "\"\""))
    } else {
        clean
    }
}

pub fn export_csv(questions: &[CheckQuestion], results: &[CheckDocumentResult]) -> String {
    let mut lines = Vec::with_capacity(results.len() + 1);
    let mut header = vec![csv_cell("Document")];
    for q in questions {
        header.push(csv_cell(&q.prompt));
    }
    lines.push(header.join(","));

    for doc in results {
        let mut row = vec![csv_cell(&doc.document_name)];
        for q in questions {
            let cell = if let Some(ans) = doc.answers.iter().find(|a| a.question_id == q.id) {
                let pct = (ans.confidence * 100.0).round() as i32;
                let part_note = match ans.part {
                    Some(p) if ans.total_parts > 1 => format!(" (part {p}/{})", ans.total_parts),
                    _ => String::new(),
                };
                format!("{} ({pct}%){part_note}", ans.answer)
            } else if let Some(err) = &doc.error {
                format!("Error: {err}")
            } else {
                String::new()
            };
            row.push(csv_cell(&cell));
        }
        lines.push(row.join(","));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_questions_and_splits_long_documents() {
        let q = CheckQuestion {
            id: "q1".into(),
            kind: CheckQuestionKind::YesNo,
            prompt: "Do I owe money?".into(),
            options: vec![],
        };
        assert!(q.validate().is_ok());
        assert_eq!(q.effective_options(), vec!["Yes", "No"]);

        let bad_choice = CheckQuestion {
            id: "q2".into(),
            kind: CheckQuestionKind::Choice,
            prompt: "Type?".into(),
            options: vec!["Only one".into()],
        };
        assert!(bad_choice.validate().is_err());

        let text = (1..=30)
            .map(|i| format!("Line {i}: clause about monthly rent and notice."))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = split_document_chunks(&text, 300);
        assert!(chunks.len() >= 3);
        assert_eq!(chunks.join("\n"), text);
    }

    #[test]
    fn parses_answers_strips_think_and_combines_chunks() {
        let q = CheckQuestion {
            id: "q1".into(),
            kind: CheckQuestionKind::Choice,
            prompt: "What kind of document?".into(),
            options: vec!["Bill".into(), "Receipt".into(), "Contract".into()],
        };
        let raw = "<think>Looks like an invoice.</think>\nANSWER: Bill\nCONFIDENCE: 92%\nREASON: Lists amount due 68 EUR.";
        let parsed = parse_check_answer(raw, &q);
        assert_eq!(parsed.answer, "Bill");
        assert!((parsed.confidence - 0.92).abs() < 1e-6);
        assert!(!parsed.unsure);
        assert_eq!(parsed.reason, "Lists amount due 68 EUR.");

        let low = parse_check_answer("ANSWER: Receipt\nCONFIDENCE: 35%\nREASON: Unclear.", &q);
        assert!(low.unsure);
        let best = combine_chunk_answers(&[low, parsed]).unwrap();
        assert_eq!(best.answer, "Bill");
        assert_eq!(best.part, Some(2));
        assert_eq!(best.total_parts, 2);
    }

    #[test]
    fn csv_export_neutralizes_formulas_and_line_breaks() {
        assert_eq!(csv_cell("=CMD|' /C calc'!A0"), "'=CMD|' /C calc'!A0");
        assert_eq!(csv_cell("+SUM(A1,B1)"), "\"'+SUM(A1,B1)\"");
        assert_eq!(csv_cell("a, \"b\"\rc"), "\"a, \"\"b\"\" c\"");
        let q = CheckQuestion {
            id: "q1".into(),
            kind: CheckQuestionKind::YesNo,
            prompt: "Pay money?".into(),
            options: vec![],
        };
        let csv = export_csv(
            &[q],
            &[CheckDocumentResult {
                document_id: "d1".into(),
                document_name: "bill, 2026.pdf".into(),
                answers: vec![CheckAnswer {
                    question_id: "q1".into(),
                    answer: "Yes".into(),
                    confidence: 0.95,
                    reason: "Due now".into(),
                    unsure: false,
                    part: None,
                    total_parts: 1,
                }],
                error: None,
            }],
        );
        assert_eq!(csv, "Document,Pay money?\n\"bill, 2026.pdf\",Yes (95%)");
    }
}
