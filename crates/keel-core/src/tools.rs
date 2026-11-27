use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::sql_sandbox::run_sql_in_workspace;

const DEFAULT_READ_BYTES: usize = 16 * 1024;
const MAX_RANGE_READ_BYTES: usize = 24 * 1024;
const DEFAULT_READ_LINES: usize = 250;
const MAX_LIST_ENTRIES: usize = 200;
const AGENT_LIST_ENTRIES: usize = 120;
const MAX_SEARCH_MATCHES: usize = 50;
const MAX_SHELL_OUTPUT_BYTES: usize = 16 * 1024;
const SHELL_TIMEOUT_SECS: u64 = 45;

fn is_ignored_dir_name(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "target"
            | "node_modules"
            | ".venv"
            | "venv"
            | "dist"
            | "build"
            | "__pycache__"
            | ".cache"
            | "cache"
            | ".pytest_cache"
            | ".mypy_cache"
            | ".ruff_cache"
            | ".next"
            | ".nuxt"
            | ".keel"
            | ".gradle"
            | ".idea"
            | ".vscode"
            | "Pods"
            | "vendor"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StagedEditKind {
    Edit,
    Create,
    Delete,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StagedEditStatus {
    Pending,
    Applied,
    Discarded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedEdit {
    pub id: String,
    pub kind: StagedEditKind,
    pub path: String,
    pub new_path: Option<String>,
    pub summary: String,
    pub old_content: Option<String>,
    pub new_content: Option<String>,
    pub diff: String,
    #[serde(default)]
    pub added_lines: usize,
    #[serde(default)]
    pub removed_lines: usize,
    pub status: StagedEditStatus,
}

/// Validates that `rel` stays strictly inside `root` without escaping via `..` or symlinks.
pub fn confine_path(root: &Path, rel: &str) -> Result<PathBuf> {
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        bail!("Path must not be empty");
    }
    let rel_path = Path::new(trimmed);
    if rel_path.is_absolute() {
        bail!("Path must be relative to the workspace: {trimmed}");
    }

    let mut clean = PathBuf::new();
    for comp in rel_path.components() {
        match comp {
            Component::Normal(p) => {
                let name = p.to_string_lossy();
                let normalized = name.trim_end_matches(['.', ' ']);
                if normalized.eq_ignore_ascii_case(".git") {
                    bail!("Refusing to modify internal .git metadata");
                }
                clean.push(p);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("Path escapes workspace: {trimmed}");
            }
        }
    }

    if clean.as_os_str().is_empty() {
        bail!("Path must point to a file or directory inside the workspace");
    }

    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("Workspace folder does not exist: {}", root.display()))?;
    let candidate = canonical_root.join(&clean);

    // Validate the nearest existing ancestor, including broken symlinks. A new
    // nested file must not bypass confinement through an earlier linked folder.
    for ancestor in candidate.ancestors() {
        if std::fs::symlink_metadata(ancestor).is_ok() {
            let canonical = ancestor
                .canonicalize()
                .with_context(|| format!("Cannot resolve workspace path: {trimmed}"))?;
            if !canonical.starts_with(&canonical_root) {
                bail!("Symlink escapes workspace: {trimmed}");
            }
            break;
        }
    }

    Ok(candidate)
}

/// Checks if a workspace path is safe (never `/` or user home root directly).
pub fn validate_workspace_root(root: &Path) -> Result<PathBuf> {
    let canon = root
        .canonicalize()
        .with_context(|| format!("Workspace directory not found: {}", root.display()))?;
    if !canon.is_dir() {
        bail!("Workspace path is not a directory: {}", canon.display());
    }
    if canon.parent().is_none() {
        bail!("Refusing filesystem root `/` as a workspace");
    }
    if let Some(home) = dirs::home_dir() {
        if let Ok(canon_home) = home.canonicalize() {
            if canon == canon_home {
                bail!(
                    "Refusing home directory `~` as a workspace; pick a project subfolder instead"
                );
            }
        }
    }
    Ok(canon)
}

pub fn list_workspace_files(root: &Path, subdir: Option<&str>) -> Result<String> {
    list_workspace_files_bounded(root, subdir, None, MAX_LIST_ENTRIES)
}

/// Sorted breadth-first workspace file listing so top-level files and folders
/// always appear first instead of being drowned out by one deep subdirectory.
pub fn list_workspace_files_bounded(
    root: &Path,
    subdir: Option<&str>,
    max_depth: Option<usize>,
    max_entries: usize,
) -> Result<String> {
    let start = match subdir {
        Some(s) if !s.trim().is_empty() && s.trim() != "." => confine_path(root, s)?,
        _ => validate_workspace_root(root)?,
    };
    let canon_root = root.canonicalize()?;
    let limit = max_entries.max(1);
    let mut files = Vec::new();
    let mut truncated = false;
    let mut queue = std::collections::VecDeque::new();
    queue.push_back((start, 0usize));

    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<_> = entries
            .flatten()
            .filter(|entry| !entry.file_type().is_ok_and(|kind| kind.is_symlink()))
            .collect();
        children.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());

        for entry in children {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if path.is_dir() && is_ignored_dir_name(&name_str) {
                continue;
            }
            if files.len() >= limit {
                truncated = true;
                break;
            }
            let rel = path
                .strip_prefix(&canon_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                files.push(format!("{rel}/"));
                if max_depth.is_none_or(|d| depth + 1 < d) {
                    queue.push_back((path, depth + 1));
                }
            } else {
                files.push(rel);
            }
        }
        if truncated {
            break;
        }
    }

    let mut out = files.join("\n");
    if truncated {
        out.push_str(&format!(
            "\n... (truncated at {limit} entries; pass a subdirectory in `path` to inspect further)"
        ));
    }
    if out.is_empty() {
        out = "(workspace is empty)".to_string();
    }
    Ok(out)
}

pub fn list_workspace_files_for_agent(
    root: &Path,
    subdir: Option<&str>,
    max_depth: Option<usize>,
) -> Result<String> {
    let default_depth = if subdir.is_none_or(|s| s.trim().is_empty() || s.trim() == ".") {
        2
    } else {
        3
    };
    let depth = max_depth.map(|d| d.clamp(1, 6)).unwrap_or(default_depth);
    list_workspace_files_bounded(root, subdir, Some(depth), AGENT_LIST_ENTRIES)
}

pub fn read_workspace_file(
    root: &Path,
    rel: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> Result<String> {
    use std::io::{BufRead, BufReader};

    let path = confine_path(root, rel)?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let explicit_range = start_line.is_some() || end_line.is_some();
    let byte_cap = if explicit_range {
        MAX_RANGE_READ_BYTES
    } else {
        DEFAULT_READ_BYTES
    };
    let start = start_line.unwrap_or(1).max(1);
    let requested_end = end_line.unwrap_or_else(|| {
        if explicit_range {
            usize::MAX
        } else {
            start.saturating_add(DEFAULT_READ_LINES - 1)
        }
    });

    let (lines_out, total_lines, shown_end, hit_byte_cap, extracted_truncated) = if extension
        .eq_ignore_ascii_case("pdf")
        || extension.eq_ignore_ascii_case("docx")
        || crate::documents::is_image_extension(extension)
    {
        let document = crate::documents::import_document(&path)
            .with_context(|| format!("Failed to extract text from {rel}"))?;
        let all_lines: Vec<&str> = document.text.lines().collect();
        let total = all_lines.len();
        let mut out = String::new();
        let mut last_shown = 0usize;
        let mut capped = false;
        for (idx, line) in all_lines.iter().enumerate() {
            let ln = idx + 1;
            if ln < start || ln > requested_end {
                continue;
            }
            let formatted = format!("{ln:>4}: {line}\n");
            if out.len() + formatted.len() > byte_cap && !out.is_empty() {
                capped = true;
                break;
            }
            out.push_str(&formatted);
            last_shown = ln;
        }
        (out, total, last_shown, capped, document.truncated)
    } else {
        let file = std::fs::File::open(&path).with_context(|| format!("Failed to read {rel}"))?;
        let mut reader = BufReader::new(file);
        let mut out = String::new();
        let mut total = 0usize;
        let mut last_shown = 0usize;
        let mut capped = false;
        let mut raw = Vec::new();
        loop {
            raw.clear();
            let n = reader
                .read_until(b'\n', &mut raw)
                .with_context(|| format!("Failed to read {rel}"))?;
            if n == 0 {
                break;
            }
            total += 1;
            if total < start || total > requested_end || capped {
                continue;
            }
            while matches!(raw.last(), Some(b'\n' | b'\r')) {
                raw.pop();
            }
            let line = String::from_utf8_lossy(&raw);
            let formatted = format!("{total:>4}: {line}\n");
            if out.len() + formatted.len() > byte_cap && !out.is_empty() {
                capped = true;
                continue;
            }
            out.push_str(&formatted);
            last_shown = total;
        }
        (out, total, last_shown, capped, false)
    };

    let mut out = lines_out;
    if out.is_empty() {
        out = if total_lines == 0 {
            "(empty file)".to_string()
        } else {
            format!(
                "(no lines in range {start}-{} — file has {total_lines} lines)",
                requested_end.min(total_lines)
            )
        };
    }
    let has_more = shown_end > 0 && shown_end < total_lines;
    if hit_byte_cap || has_more || extracted_truncated {
        let next_start = shown_end.saturating_add(1);
        out.push_str(&format!(
            "\n... (showing lines {start}-{shown_end} of {total_lines} total; pass start_line={next_start} and end_line to read further)"
        ));
    }
    Ok(out)
}

pub fn search_workspace_files(root: &Path, pattern: &str) -> Result<String> {
    search_workspace_files_in(root, pattern, None)
}

pub fn search_workspace_files_in(
    root: &Path,
    pattern: &str,
    subdir: Option<&str>,
) -> Result<String> {
    if pattern.trim().is_empty() {
        bail!("Search pattern must not be empty");
    }
    let re = Regex::new(pattern)
        .unwrap_or_else(|_| Regex::new(&regex::escape(pattern)).expect("escaped regex"));
    let canon_root = validate_workspace_root(root)?;
    let start = match subdir {
        Some(s) if !s.trim().is_empty() && s.trim() != "." => confine_path(&canon_root, s)?,
        _ => canon_root.clone(),
    };
    let mut matches = Vec::new();
    if start.is_file() {
        search_file(&canon_root, &start, &re, &mut matches);
    } else {
        search_dir(&canon_root, &start, &re, &mut matches);
    }
    if matches.is_empty() {
        return Ok(format!("No matches found for `{pattern}`."));
    }
    let truncated = matches.len() >= MAX_SEARCH_MATCHES;
    let mut out = matches.join("\n");
    if truncated {
        out.push_str(&format!(
            "\n... (stopped at {MAX_SEARCH_MATCHES} matches; narrow `pattern` or pass a subdirectory in `path`)"
        ));
    }
    Ok(out)
}

fn search_file(root: &Path, path: &Path, re: &Regex, out: &mut Vec<String>) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() > 512 * 1024 {
        return;
    }
    let Ok(content) = std::fs::read_to_string(path) else {
        return;
    };
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    for (idx, line) in content.lines().enumerate() {
        if re.is_match(line) {
            let clipped: String = line.chars().take(140).collect();
            out.push(format!("{rel}:{}: {clipped}", idx + 1));
            if out.len() >= MAX_SEARCH_MATCHES {
                return;
            }
        }
    }
}

fn search_dir(root: &Path, dir: &Path, re: &Regex, out: &mut Vec<String>) {
    if out.len() >= MAX_SEARCH_MATCHES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = entries
        .flatten()
        .filter(|entry| !entry.file_type().is_ok_and(|kind| kind.is_symlink()))
        .collect();
    children.sort_by_key(|entry| entry.file_name().to_string_lossy().to_ascii_lowercase());

    for entry in children {
        if out.len() >= MAX_SEARCH_MATCHES {
            break;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if path.is_dir() {
            if is_ignored_dir_name(&name_str) {
                continue;
            }
            search_dir(root, &path, re, out);
        } else if path.is_file() {
            search_file(root, &path, re, out);
        }
    }
}

fn ensure_not_mcp_config(rel: &str) -> Result<()> {
    let parts: Vec<String> = Path::new(rel.trim())
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(
                s.to_string_lossy()
                    .trim_end_matches(['.', ' '])
                    .to_ascii_lowercase(),
            ),
            _ => None,
        })
        .collect();
    if parts.len() == 2 && parts[0] == ".keel" && parts[1] == "mcp.json" {
        bail!("Refusing to modify .keel/mcp.json through AI tools. Edit MCP servers in Connectors instead.");
    }
    Ok(())
}

pub fn build_staged_edit(
    root: &Path,
    kind: StagedEditKind,
    path: &str,
    target: Option<&str>,
    replacement: Option<&str>,
    new_path: Option<&str>,
) -> Result<StagedEdit> {
    ensure_not_mcp_config(path)?;
    if let Some(np) = new_path {
        ensure_not_mcp_config(np)?;
    }
    let resolved = confine_path(root, path)?;
    match kind {
        StagedEditKind::Edit => {
            let old_content = std::fs::read_to_string(&resolved)
                .with_context(|| format!("Cannot read {path} for edit"))?;
            let new_content = match target {
                Some(t) if !t.is_empty() => {
                    if !old_content.contains(t) {
                        bail!("Target text was not found in {path}");
                    }
                    old_content.replacen(t, replacement.unwrap_or(""), 1)
                }
                _ => replacement
                    .context("Missing replacement content for edit")?
                    .to_string(),
            };
            let (diff, added_lines, removed_lines) =
                compute_diff(path, Some(&old_content), Some(&new_content));
            Ok(StagedEdit {
                id: uuid::Uuid::new_v4().to_string(),
                kind,
                path: path.to_string(),
                new_path: None,
                summary: format!("Edit {path}"),
                old_content: Some(old_content),
                new_content: Some(new_content),
                diff,
                added_lines,
                removed_lines,
                status: StagedEditStatus::Pending,
            })
        }
        StagedEditKind::Create => {
            let content = replacement.unwrap_or("").to_string();
            let old_content = if resolved.exists() {
                Some(std::fs::read_to_string(&resolved)?)
            } else {
                None
            };
            let (diff, added_lines, removed_lines) =
                compute_diff(path, old_content.as_deref(), Some(&content));
            Ok(StagedEdit {
                id: uuid::Uuid::new_v4().to_string(),
                kind,
                path: path.to_string(),
                new_path: None,
                summary: if old_content.is_some() {
                    format!("Overwrite {path}")
                } else {
                    format!("Create {path}")
                },
                old_content,
                new_content: Some(content),
                diff,
                added_lines,
                removed_lines,
                status: StagedEditStatus::Pending,
            })
        }
        StagedEditKind::Delete => {
            if !resolved.exists() {
                bail!("File does not exist: {path}");
            }
            let old_content = std::fs::read_to_string(&resolved).ok();
            let (diff, added_lines, removed_lines) =
                compute_diff(path, old_content.as_deref(), None);
            Ok(StagedEdit {
                id: uuid::Uuid::new_v4().to_string(),
                kind,
                path: path.to_string(),
                new_path: None,
                summary: format!("Delete {path}"),
                old_content,
                new_content: None,
                diff,
                added_lines,
                removed_lines,
                status: StagedEditStatus::Pending,
            })
        }
        StagedEditKind::Rename => {
            if !resolved.exists() {
                bail!("Source file does not exist: {path}");
            }
            let dest = new_path.context("Missing new_path for rename")?;
            let _ = confine_path(root, dest)?;
            let diff = format!("--- {path}\n+++ {dest}\nrename {path} -> {dest}");
            Ok(StagedEdit {
                id: uuid::Uuid::new_v4().to_string(),
                kind,
                path: path.to_string(),
                new_path: Some(dest.to_string()),
                summary: format!("Rename {path} -> {dest}"),
                old_content: None,
                new_content: None,
                diff,
                added_lines: 0,
                removed_lines: 0,
                status: StagedEditStatus::Pending,
            })
        }
    }
}

pub fn apply_staged_edit(root: &Path, edit: &mut StagedEdit) -> Result<()> {
    ensure_not_mcp_config(&edit.path)?;
    if let Some(ref np) = edit.new_path {
        ensure_not_mcp_config(np)?;
    }
    let resolved = confine_path(root, &edit.path)?;
    match edit.kind {
        StagedEditKind::Edit | StagedEditKind::Create => {
            if let Some(parent) = resolved.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let content = edit.new_content.as_deref().unwrap_or("");
            std::fs::write(&resolved, content)?;
        }
        StagedEditKind::Delete => {
            if resolved.exists() {
                std::fs::remove_file(&resolved)?;
            }
        }
        StagedEditKind::Rename => {
            let dest_rel = edit.new_path.as_deref().context("Missing new_path")?;
            let dest = confine_path(root, dest_rel)?;
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(&resolved, &dest)?;
        }
    }
    edit.status = StagedEditStatus::Applied;
    Ok(())
}

fn compute_diff(path: &str, old: Option<&str>, new: Option<&str>) -> (String, usize, usize) {
    let old_lines: Vec<&str> = old.map(|s| s.lines().collect()).unwrap_or_default();
    let new_lines: Vec<&str> = new.map(|s| s.lines().collect()).unwrap_or_default();

    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }

    let mut suffix = 0;
    while suffix < (old_lines.len() - prefix)
        && suffix < (new_lines.len() - prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let removed_slice = &old_lines[prefix..old_lines.len() - suffix];
    let added_slice = &new_lines[prefix..new_lines.len() - suffix];

    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    for line in removed_slice.iter().take(80) {
        out.push_str(&format!("- {line}\n"));
    }
    if removed_slice.len() > 80 {
        out.push_str(&format!(
            "- ... ({} more removed lines)\n",
            removed_slice.len() - 80
        ));
    }
    for line in added_slice.iter().take(80) {
        out.push_str(&format!("+ {line}\n"));
    }
    if added_slice.len() > 80 {
        out.push_str(&format!(
            "+ ... ({} more added lines)\n",
            added_slice.len() - 80
        ));
    }

    (out, added_slice.len(), removed_slice.len())
}

pub fn is_destructive_shell_command(cmd: &str) -> Option<&'static str> {
    let lower = cmd.to_ascii_lowercase();
    if lower.contains("rm -rf /") || lower.contains("rm -rf ~") {
        return Some("Removes root or home directory");
    }
    if lower.contains("mkfs") || lower.contains("dd if=") {
        return Some("Raw disk write command");
    }
    if lower.contains(":(){") {
        return Some("Fork bomb pattern");
    }
    None
}

async fn read_capped_and_drain<R: tokio::io::AsyncRead + Unpin>(
    mut reader: Option<R>,
    limit: usize,
) -> (Vec<u8>, bool) {
    let Some(ref mut r) = reader else {
        return (Vec::new(), false);
    };
    let mut out = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if out.len() < limit {
                    let take = (limit - out.len()).min(n);
                    out.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
        }
    }
    (out, truncated)
}

pub async fn run_shell_command(root: &Path, command: &str) -> Result<String> {
    let canon_root = validate_workspace_root(root)?;
    if let Some(reason) = is_destructive_shell_command(command) {
        bail!("Blocked destructive command ({reason}): {command}");
    }

    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-lc", command]);
        c
    };

    cmd.current_dir(&canon_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().context("Failed to spawn shell command")?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let fut = async {
        let ((out_buf, out_trunc), (err_buf, err_trunc), status) = tokio::join!(
            read_capped_and_drain(stdout, MAX_SHELL_OUTPUT_BYTES),
            read_capped_and_drain(stderr, MAX_SHELL_OUTPUT_BYTES),
            child.wait()
        );
        let status = status?;
        Ok::<_, anyhow::Error>((status, out_buf, out_trunc, err_buf, err_trunc))
    };

    match tokio::time::timeout(Duration::from_secs(SHELL_TIMEOUT_SECS), fut).await {
        Ok(res) => {
            let (status, out_buf, out_trunc, err_buf, err_trunc) = res?;
            let out = String::from_utf8_lossy(&out_buf);
            let err = String::from_utf8_lossy(&err_buf);
            let code = status.code().unwrap_or(-1);
            let mut text = format!("exit code: {code}\n");
            if !out.trim().is_empty() {
                text.push_str(&format!("stdout:\n{out}\n"));
                if out_trunc {
                    text.push_str("... (stdout truncated)\n");
                }
            }
            if !err.trim().is_empty() {
                text.push_str(&format!("stderr:\n{err}\n"));
                if err_trunc {
                    text.push_str("... (stderr truncated)\n");
                }
            }
            Ok(text)
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            bail!("Shell command timed out after {SHELL_TIMEOUT_SECS}s");
        }
    }
}

pub fn execute_sql_tool(root: Option<&Path>, query: &str) -> Result<String> {
    let validated = match root {
        Some(r) => Some(validate_workspace_root(r)?),
        None => None,
    };
    run_sql_in_workspace(validated.as_deref(), query)
}

/// Returns OpenAI tool definitions filtered to `allowed_names`, plus any active MCP tools.
pub fn openai_tool_schemas(
    allowed_names: &[&str],
    extra_mcp_tools: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let all = vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "list_files",
                "description": "List files and directories inside the current workspace (breadth-first; top-level entries appear first). Pass a subdirectory in `path` to inspect deeper.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Optional relative subdirectory" },
                        "max_depth": { "type": "integer", "description": "Optional max folder depth (1-6; default 2 at root, 3 in a subfolder)" }
                    }
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a workspace file with line numbers (defaults to first 250 lines; pass start_line/end_line to read later sections). Use a relative path such as src/app.js, never an absolute path.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "start_line": { "type": "integer" },
                        "end_line": { "type": "integer" }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "search_files",
                "description": "Search workspace files for text or a regex pattern. Pass optional `path` to restrict search to one subfolder or file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string", "description": "Optional relative subdirectory or file to search within" }
                    },
                    "required": ["pattern"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "propose_edit",
                "description": "Propose an edit to an existing workspace file. Replaces `target` with `replacement`.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "target": { "type": "string" },
                        "replacement": { "type": "string" }
                    },
                    "required": ["path", "target", "replacement"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "create_file",
                "description": "Create or overwrite a file in the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "delete_file",
                "description": "Delete a file in the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "rename_file",
                "description": "Rename or move a file in the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "new_path": { "type": "string" }
                    },
                    "required": ["path", "new_path"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_sql",
                "description": "Query workspace CSVs with SQLite. Quote filenames: SELECT SUM(revenue) FROM \"sales.csv\". The bare stem sales also works. Use this for exact calculations.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" }
                    },
                    "required": ["query"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_shell",
                "description": "Run a shell command in the workspace folder (requires user approval).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" }
                    },
                    "required": ["command"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the public web for current facts or sources (requires user approval). Use for news, documentation, or anything that may have changed since training. Do not invent results.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Short search query" }
                    },
                    "required": ["query"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_url",
                "description": "Fetch and read the text of a public http/https web page (requires user approval). Use to inspect a link shared by the user or returned by web_search.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "Public http:// or https:// URL to read" }
                    },
                    "required": ["url"]
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "create_short_video",
                "description": "Render one short-form video (.mp4) from workspace photos or video clips with Ken Burns camera motion, safe-zone overlays, and optional local neural TTS narration via FFmpeg. Pass scenes as blank-line blocks (`image:`, `say:`, optional `overlay:`, optional `motion:` in none|slow_zoom|pan_left|pan_right) or a JSON array. Saves the .mp4 plus editable .storyboard.md and .json manifest inside the workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative .mp4 output path inside workspace, e.g. videos/launch.mp4" },
                        "scenes": { "type": "string", "description": "Storyboard block or JSON array of scenes" },
                        "aspect_ratio": { "type": "string", "enum": ["9:16", "1:1", "16:9"] },
                        "title": { "type": "string" },
                        "voice": { "type": "string" },
                        "music_path": { "type": "string", "description": "Optional relative workspace audio file for background music" }
                    },
                    "required": ["path", "scenes"]
                }
            }
        }),
    ];

    let mut filtered: Vec<serde_json::Value> = all
        .into_iter()
        .filter(|item| {
            item.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .is_some_and(|name| allowed_names.contains(&name))
        })
        .collect();

    filtered.extend(extra_mcp_tools.iter().cloned());
    filtered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confines_paths_and_blocks_escapes() {
        let dir = std::env::temp_dir().join(format!("keel_tools_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hello.txt"), "hello\nworld\n").unwrap();

        assert!(confine_path(&dir, "hello.txt").is_ok());
        assert!(confine_path(&dir, "../etc/passwd").is_err());
        assert!(confine_path(&dir, "/etc/passwd").is_err());
        assert!(confine_path(&dir, ".git/config").is_err());
        assert!(confine_path(&dir, ".GIT/config").is_err());
        assert!(confine_path(&dir, ".Git/hooks/pre-commit").is_err());
        assert!(confine_path(&dir, ".git./config").is_err());
        assert!(build_staged_edit(
            &dir,
            StagedEditKind::Create,
            ".keel/mcp.json",
            None,
            Some("{}"),
            None,
        )
        .is_err());

        let mut edit = build_staged_edit(
            &dir,
            StagedEditKind::Edit,
            "hello.txt",
            Some("world"),
            Some("keel"),
            None,
        )
        .unwrap();
        assert_eq!(edit.status, StagedEditStatus::Pending);
        assert_eq!(edit.added_lines, 1);
        assert_eq!(edit.removed_lines, 1);
        // File should still be unchanged until applied.
        assert_eq!(
            std::fs::read_to_string(dir.join("hello.txt")).unwrap(),
            "hello\nworld\n"
        );
        apply_staged_edit(&dir, &mut edit).unwrap();
        assert_eq!(edit.status, StagedEditStatus::Applied);
        assert_eq!(
            std::fs::read_to_string(dir.join("hello.txt")).unwrap(),
            "hello\nkeel\n"
        );

        let created = build_staged_edit(
            &dir,
            StagedEditKind::Create,
            "new.txt",
            None,
            Some("alpha\nbeta\ngamma"),
            None,
        )
        .unwrap();
        assert_eq!(created.added_lines, 3);
        assert_eq!(created.removed_lines, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_files_is_breadth_first_and_skips_cache_dirs_and_read_pages_large_files() {
        let dir = std::env::temp_dir().join(format!("keel_tools_bfs_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("aaa_cache/cache/deep")).unwrap();
        std::fs::create_dir_all(dir.join("game")).unwrap();
        for i in 0..150 {
            std::fs::write(dir.join(format!("aaa_cache/cache/deep/f{i}.txt")), "x").unwrap();
        }
        std::fs::write(dir.join("README.md"), "hello").unwrap();
        std::fs::write(dir.join("game/main.lua"), "print('hi')\n").unwrap();

        let listed = list_workspace_files_for_agent(&dir, None, None).unwrap();
        assert!(
            listed.contains("README.md"),
            "top-level file must appear: {listed}"
        );
        assert!(
            listed.contains("game/main.lua"),
            "game file must appear: {listed}"
        );
        assert!(
            !listed.contains("f100.txt"),
            "cache dir must be skipped: {listed}"
        );

        let searched = search_workspace_files_in(&dir, "print", Some("game")).unwrap();
        assert!(searched.contains("game/main.lua:1:"));

        // Verify read_workspace_file defaults to 250 lines and can page past line 250 / 64 KB.
        let big: String = (1..=400)
            .map(|n| format!("line-{n:04}-{}", "z".repeat(200)))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.join("big.txt"), &big).unwrap();
        let page1 = read_workspace_file(&dir, "big.txt", None, None).unwrap();
        assert!(page1.contains("of 400 total"));
        let tail = read_workspace_file(&dir, "big.txt", Some(395), Some(400)).unwrap();
        assert!(tail.contains("395: line-0395-"));
        assert!(tail.contains("400: line-0400-"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
