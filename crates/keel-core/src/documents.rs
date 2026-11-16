//! Document content is imported explicitly and kept with a conversation or project.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;

const MAX_FILE: u64 = 20 * 1024 * 1024;
pub const MAX_TEXT_CHARS: usize = 300_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub text: String,
    pub truncated: bool,
}

pub fn import_document(path: &Path) -> Result<Document> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_FILE {
        bail!(
            "{} is larger than the 20 MB attachment limit",
            path.display()
        );
    }
    let bytes = std::fs::read(path)?;
    let kind = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let text = match kind.as_str() {
        "pdf" => pdf_extract::extract_text_from_mem(&bytes).context("Could not read this PDF. Scanned pages need OCR; try a text-based PDF or paste its text.")?,
        "docx" => extract_docx(&bytes)?,
        "txt" | "md" | "csv" | "tsv" | "json" | "html" | "htm" | "xml" | "yaml" | "yml" | "toml" | "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "css" | "sql" | "log" => String::from_utf8(bytes).context("This file is not UTF-8 text. Export it as UTF-8 and try again.")?,
        _ => bail!("Attach a PDF, DOCX, text, Markdown, CSV, or source-code file. Images and other binary formats are not supported in Chat yet."),
    };
    if text.trim().is_empty() {
        bail!("No readable text found. Scanned PDFs need OCR before they can be attached.");
    }
    let truncated = text.chars().count() > MAX_TEXT_CHARS;
    let text = text.chars().take(MAX_TEXT_CHARS).collect();
    Ok(Document {
        id: uuid::Uuid::new_v4().to_string(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        kind,
        text,
        truncated,
    })
}

fn extract_docx(bytes: &[u8]) -> Result<String> {
    use quick_xml::{events::Event, Reader};
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let file = archive
        .by_name("word/document.xml")
        .context("DOCX has no document content")?;
    let mut xml = String::new();
    file.take(4 * 1024 * 1024 + 1).read_to_string(&mut xml)?;
    if xml.len() > 4 * 1024 * 1024 {
        bail!("DOCX content is too large; export the relevant pages as text");
    }
    let mut reader = Reader::from_str(&xml);
    let mut text = String::new();
    let mut in_text = false;
    loop {
        match reader.read_event()? {
            Event::Start(e) if e.name().as_ref() == b"w:t" => in_text = true,
            Event::Text(e) if in_text => text.push_str(&e.unescape()?),
            Event::End(e) if e.name().as_ref() == b"w:t" => in_text = false,
            Event::End(e) if e.name().as_ref() == b"w:p" => text.push('\n'),
            Event::Empty(e) if e.name().as_ref() == b"w:tab" => text.push('\t'),
            Event::Empty(e) if e.name().as_ref() == b"w:br" => text.push('\n'),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(text)
}

/// Include relevant sections under a strict character budget, retaining source order.
/// This is retrieval of source text, never a fabricated summary of unseen pages.
pub fn excerpt(text: &str, query: &str, budget: usize) -> (String, bool) {
    if text.chars().count() <= budget {
        return (text.to_string(), false);
    }
    let chars: Vec<_> = text.chars().collect();
    let chunks: Vec<String> = chars.chunks(1000).map(|c| c.iter().collect()).collect();
    let words: Vec<_> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 3)
        .map(str::to_lowercase)
        .collect();
    let mut ranked: Vec<_> = chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let lower = c.to_lowercase();
            let score = words
                .iter()
                .map(|w| lower.matches(w).count().min(10))
                .sum::<usize>();
            (i, score)
        })
        .collect();
    ranked.sort_by_key(|&(i, score)| (std::cmp::Reverse(score), i));
    let count = (budget / 1050).max(1);
    let mut selected = vec![0];
    for (i, _) in ranked {
        if selected.len() >= count {
            break;
        }
        if !selected.contains(&i) {
            selected.push(i);
        }
    }
    selected.sort_unstable();
    let result = selected
        .iter()
        .map(|&i| format!("[Excerpt {}]\n{}", i + 1, chunks[i]))
        .collect::<Vec<_>>()
        .join("\n\n");
    (result.chars().take(budget).collect(), true)
}
