use anyhow::{bail, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::sql_sandbox::run_sql_in_workspace;

const MAX_READ_BYTES: usize = 64 * 1024;
const MAX_LIST_ENTRIES: usize = 300;
const MAX_SEARCH_MATCHES: usize = 80;
const MAX_SHELL_OUTPUT_BYTES: usize = 32 * 1024;
const SHELL_TIMEOUT_SECS: u64 = 45;

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
                if p == ".git" {
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
    let start = match subdir {
        Some(s) if !s.trim().is_empty() && s.trim() != "." => confine_path(root, s)?,
        _ => validate_workspace_root(root)?,
    };
    let canon_root = root.canonicalize()?;
    let mut files = Vec::new();
    collect_files_recursive(&canon_root, &start, &mut files)?;
    files.sort();
    let truncated = files.len() > MAX_LIST_ENTRIES;
    files.truncate(MAX_LIST_ENTRIES);
    let mut out = files.join("\n");
    if truncated {
        out.push_str(&format!("\n... (truncated at {MAX_LIST_ENTRIES} entries)"));
    }
    if out.is_empty() {
        out = "(workspace is empty)".to_string();
    }
    Ok(out)
}

fn collect_files_recursive(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<()> {
    if out.len() > MAX_LIST_ENTRIES {
        return Ok(());
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if out.len() > MAX_LIST_ENTRIES {
            break;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            ".git" | "target" | "node_modules" | ".venv" | "dist"
        ) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if path.is_dir() {
            out.push(format!("{rel}/"));
            collect_files_recursive(root, &path, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

pub fn read_workspace_file(
    root: &Path,
    rel: &str,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> Result<String> {
    let path = confine_path(root, rel)?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let (bytes, extracted_truncated) = if extension.eq_ignore_ascii_case("pdf")
        || extension.eq_ignore_ascii_case("docx")
        || crate::documents::is_image_extension(extension)
    {
        let document = crate::documents::import_document(&path)
            .with_context(|| format!("Failed to extract text from {rel}"))?;
        (document.text.into_bytes(), document.truncated)
    } else {
        let mut bytes = Vec::new();
        std::fs::File::open(&path)
            .and_then(|file| {
                file.take((MAX_READ_BYTES + 1) as u64)
                    .read_to_end(&mut bytes)
            })
            .with_context(|| format!("Failed to read {rel}"))?;
        (bytes, false)
    };
    let truncated_bytes = extracted_truncated || bytes.len() > MAX_READ_BYTES;
    let slice = &bytes[..bytes.len().min(MAX_READ_BYTES)];
    let text = String::from_utf8_lossy(slice);
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let start = start_line.unwrap_or(1).max(1);
    let end = end_line.unwrap_or(total.max(1)).min(total.max(1));

    let mut out = String::new();
    for (idx, line) in lines.iter().enumerate() {
        let ln = idx + 1;
        if ln >= start && ln <= end {
            out.push_str(&format!("{ln:>4}: {line}\n"));
        }
    }
    if out.is_empty() {
        out = "(empty file or line range)".to_string();
    }
    if truncated_bytes {
        out.push_str(&format!("\n... (file truncated at {MAX_READ_BYTES} bytes)"));
    }
    Ok(out)
}

pub fn search_workspace_files(root: &Path, pattern: &str) -> Result<String> {
    if pattern.trim().is_empty() {
        bail!("Search pattern must not be empty");
    }
    let re = Regex::new(pattern)
        .unwrap_or_else(|_| Regex::new(&regex::escape(pattern)).expect("escaped regex"));
    let canon_root = validate_workspace_root(root)?;
    let mut matches = Vec::new();
    search_dir(&canon_root, &canon_root, &re, &mut matches);
    if matches.is_empty() {
        return Ok(format!("No matches found for `{pattern}`."));
    }
    let truncated = matches.len() >= MAX_SEARCH_MATCHES;
    let mut out = matches.join("\n");
    if truncated {
        out.push_str(&format!("\n... (stopped at {MAX_SEARCH_MATCHES} matches)"));
    }
    Ok(out)
}

fn search_dir(root: &Path, dir: &Path, re: &Regex, out: &mut Vec<String>) {
    if out.len() >= MAX_SEARCH_MATCHES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_SEARCH_MATCHES {
            break;
        }
        if entry.file_type().is_ok_and(|kind| kind.is_symlink()) {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            ".git" | "target" | "node_modules" | ".venv" | "dist"
        ) {
            continue;
        }
        if path.is_dir() {
            search_dir(root, &path, re, out);
        } else if path.is_file() {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if meta.len() > 512 * 1024 {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            for (idx, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    let clipped: String = line.chars().take(180).collect();
                    out.push(format!("{rel}:{}: {clipped}", idx + 1));
                    if out.len() >= MAX_SEARCH_MATCHES {
                        return;
                    }
                }
            }
        }
    }
}

pub fn build_staged_edit(
    root: &Path,
    kind: StagedEditKind,
    path: &str,
    target: Option<&str>,
    replacement: Option<&str>,
    new_path: Option<&str>,
) -> Result<StagedEdit> {
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
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("Failed to spawn shell command")?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    let fut = async {
        let mut out_buf = Vec::new();
        let mut err_buf = Vec::new();
        if let Some(ref mut s) = stdout {
            let _ = s
                .take(MAX_SHELL_OUTPUT_BYTES as u64)
                .read_to_end(&mut out_buf)
                .await;
        }
        if let Some(ref mut s) = stderr {
            let _ = s
                .take(MAX_SHELL_OUTPUT_BYTES as u64)
                .read_to_end(&mut err_buf)
                .await;
        }
        let status = child.wait().await?;
        Ok::<_, anyhow::Error>((status, out_buf, err_buf))
    };

    match tokio::time::timeout(Duration::from_secs(SHELL_TIMEOUT_SECS), fut).await {
        Ok(res) => {
            let (status, out_buf, err_buf) = res?;
            let out = String::from_utf8_lossy(&out_buf);
            let err = String::from_utf8_lossy(&err_buf);
            let code = status.code().unwrap_or(-1);
            let mut text = format!("exit code: {code}\n");
            if !out.trim().is_empty() {
                text.push_str(&format!("stdout:\n{out}\n"));
            }
            if !err.trim().is_empty() {
                text.push_str(&format!("stderr:\n{err}\n"));
            }
            Ok(text)
        }
        Err(_) => {
            let _ = child.kill().await;
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
                "description": "List files and directories inside the current workspace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Optional relative subdirectory" }
                    }
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a workspace file with line numbers. Use a relative path such as src/app.js, never an absolute path.",
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
                "description": "Search workspace files for text or a regex pattern.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" }
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
}
