//! Document content is imported explicitly and kept with a conversation or project.
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const MAX_FILE: u64 = 20 * 1024 * 1024;
const MAX_IMAGE_PREVIEW_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TEXT_CHARS: usize = 300_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub text: String,
    pub truncated: bool,
    #[serde(default)]
    pub image_data_url: Option<String>,
    #[serde(default)]
    pub image_only: bool,
}

pub fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "bmp" | "tif" | "tiff"
    )
}

fn image_mime(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        _ => "application/octet-stream",
    }
}

fn find_tesseract() -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "tesseract.exe"
    } else {
        "tesseract"
    };
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(exe);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    for extra in [
        "/opt/homebrew/bin/tesseract",
        "/usr/local/bin/tesseract",
        "/usr/bin/tesseract",
    ] {
        let cand = PathBuf::from(extra);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

pub fn ocr_available() -> bool {
    find_tesseract().is_some()
}

pub fn extract_image_ocr(path: &Path) -> Result<String> {
    let bin = find_tesseract().context(
        "Offline image reading needs Tesseract OCR installed locally (e.g. 'sudo apt install tesseract-ocr' or 'brew install tesseract').",
    )?;
    let output = Command::new(bin)
        .arg(path)
        .arg("stdout")
        .args(["--psm", "6"])
        .output()
        .context("Could not start local Tesseract OCR")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Could not read text from this image: {}", stderr.trim());
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if text.is_empty() {
        bail!("No readable text found in this image.");
    }
    Ok(text)
}

const VISION_FALLBACK_CAPTION: &str =
    "(Image pixels attached for vision. No OCR text was extracted.)";

fn image_text_with_optional_ocr(path: &Path, has_pixels: bool) -> Result<(String, bool)> {
    match extract_image_ocr(path) {
        Ok(text) => Ok((text, false)),
        Err(_) if has_pixels => Ok((VISION_FALLBACK_CAPTION.to_string(), true)),
        Err(err) => Err(err),
    }
}

struct TempFileGuard(PathBuf);
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn write_private_temp(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

pub fn import_document_base64(name: &str, base64_data: &str) -> Result<Document> {
    if base64_data.len() > 28 * 1024 * 1024 {
        bail!("{name} is larger than the 20 MB attachment limit");
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data.trim())
        .context("Could not decode pasted or dropped file data.")?;
    import_document_bytes(name, &bytes)
}

fn is_text_extension(ext: &str) -> bool {
    matches!(
        ext,
        "txt"
            | "md"
            | "csv"
            | "tsv"
            | "json"
            | "html"
            | "htm"
            | "xml"
            | "yaml"
            | "yml"
            | "toml"
            | "ini"
            | "conf"
            | "cfg"
            | "rs"
            | "py"
            | "js"
            | "mjs"
            | "cjs"
            | "ts"
            | "tsx"
            | "jsx"
            | "vue"
            | "svelte"
            | "css"
            | "scss"
            | "less"
            | "sql"
            | "log"
            | "go"
            | "c"
            | "cc"
            | "cpp"
            | "h"
            | "hpp"
            | "java"
            | "kt"
            | "swift"
            | "rb"
            | "php"
            | "sh"
            | "bash"
            | "zsh"
    )
}

pub fn import_document_bytes(name: &str, bytes: &[u8]) -> Result<Document> {
    if (bytes.len() as u64) > MAX_FILE {
        bail!("{name} is larger than the 20 MB attachment limit");
    }
    let clean_name = Path::new(name)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "attachment.txt".to_string());
    let kind = Path::new(&clean_name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut image_data_url = None;
    let mut image_only = false;
    let text = match kind.as_str() {
        "pdf" => pdf_extract::extract_text_from_mem(bytes).context("Could not read this PDF. Scanned pages need OCR; try a text-based PDF or paste its text.")?,
        "docx" => extract_docx(bytes)?,
        ext if is_image_extension(ext) => {
            if bytes.len() <= MAX_IMAGE_PREVIEW_BYTES {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                image_data_url = Some(format!("data:{};base64,{encoded}", image_mime(ext)));
            }
            let tmp = std::env::temp_dir().join(format!("keel_ocr_{}.{ext}", uuid::Uuid::new_v4()));
            write_private_temp(&tmp, bytes)?;
            let _guard = TempFileGuard(tmp.clone());
            let (text, only_pixels) = image_text_with_optional_ocr(&tmp, image_data_url.is_some())?;
            image_only = only_pixels;
            text
        }
        ext if is_text_extension(ext) => {
            String::from_utf8(bytes.to_vec()).context("This file is not UTF-8 text. Export it as UTF-8 and try again.")?
        }
        _ => bail!("Attach a PDF, DOCX, text, Markdown, CSV, source-code file, or image (PNG, JPG, WebP, BMP, TIFF)."),
    };
    if text.trim().is_empty() {
        bail!("No readable text found. Scanned PDFs need OCR before they can be attached.");
    }
    let truncated = text.chars().count() > MAX_TEXT_CHARS;
    let text = text.chars().take(MAX_TEXT_CHARS).collect();
    Ok(Document {
        id: uuid::Uuid::new_v4().to_string(),
        name: clean_name,
        kind,
        text,
        truncated,
        image_data_url,
        image_only,
    })
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
    let mut image_data_url = None;
    let mut image_only = false;
    let text = match kind.as_str() {
        "pdf" => pdf_extract::extract_text_from_mem(&bytes).context("Could not read this PDF. Scanned pages need OCR; try a text-based PDF or paste its text.")?,
        "docx" => extract_docx(&bytes)?,
        ext if is_image_extension(ext) => {
            if bytes.len() <= MAX_IMAGE_PREVIEW_BYTES {
                let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                image_data_url = Some(format!("data:{};base64,{encoded}", image_mime(ext)));
            }
            let (text, only_pixels) = image_text_with_optional_ocr(path, image_data_url.is_some())?;
            image_only = only_pixels;
            text
        }
        ext if is_text_extension(ext) => String::from_utf8(bytes).context("This file is not UTF-8 text. Export it as UTF-8 and try again.")?,
        _ => bail!("Attach a PDF, DOCX, text, Markdown, CSV, source-code file, or image (PNG, JPG, WebP, BMP, TIFF)."),
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
        image_data_url,
        image_only,
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

fn is_stopword(word: &str) -> bool {
    matches!(
        word,
        "am" | "an"
            | "as"
            | "at"
            | "be"
            | "by"
            | "do"
            | "go"
            | "he"
            | "if"
            | "in"
            | "is"
            | "it"
            | "me"
            | "my"
            | "no"
            | "of"
            | "on"
            | "or"
            | "so"
            | "to"
            | "up"
            | "us"
            | "we"
            | "and"
            | "are"
            | "for"
            | "the"
            | "was"
            | "you"
    )
}

fn split_boundary_chunks(text: &str, target_chars: usize, overlap_chars: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= target_chars {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    let step_min = target_chars / 2;
    while start < chars.len() {
        let hard_end = (start + target_chars).min(chars.len());
        let mut end = hard_end;
        if hard_end < chars.len() {
            // Prefer splitting on a newline or whitespace boundary near target_chars.
            let min_pos = start + step_min;
            if let Some(nl) = (min_pos..hard_end).rev().find(|&i| chars[i] == '\n') {
                end = nl + 1;
            } else if let Some(sp) = (min_pos..hard_end)
                .rev()
                .find(|&i| chars[i].is_whitespace())
            {
                end = sp + 1;
            }
        }
        chunks.push(chars[start..end].iter().collect());
        if end >= chars.len() {
            break;
        }
        start = end.saturating_sub(overlap_chars).max(start + 1);
    }
    chunks
}

/// Include relevant sections under a strict character budget, retaining source order.
/// This is retrieval of source text, never a fabricated summary of unseen pages.
pub fn excerpt(text: &str, query: &str, budget: usize) -> (String, bool) {
    if text.chars().count() <= budget {
        return (text.to_string(), false);
    }
    let chunks = split_boundary_chunks(text, 1000, 120);
    let words: Vec<_> = query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 2 && !is_stopword(w))
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
