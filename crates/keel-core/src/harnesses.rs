use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessOption {
    pub id: String,
    pub name: String,
    pub description: String,
    pub available: bool,
    pub binary_path: Option<String>,
}

/// Lists built-in Keel Harness plus any installed external CLI coding agents.
pub fn detect_harnesses() -> Vec<HarnessOption> {
    let mut list = vec![HarnessOption {
        id: "keel".to_string(),
        name: "Keel Harness".to_string(),
        description:
            "Built-in small-model engine with staged diffs, SQLite/CSV sandbox, and stdio MCP"
                .to_string(),
        available: true,
        binary_path: None,
    }];

    let externals = [
        (
            "opencode",
            "OpenCode",
            "Route workspace prompts through the installed `opencode` CLI",
        ),
        (
            "keel-cli",
            "Keel CLI",
            "Route workspace prompts through the standalone `keel` CLI binary",
        ),
        (
            "claude",
            "Claude Code",
            "Route workspace prompts through the installed `claude` CLI",
        ),
    ];

    for (id, name, desc) in externals {
        let bin = match id {
            "keel-cli" => find_on_path("keel"),
            other => find_on_path(other),
        };
        list.push(HarnessOption {
            id: id.to_string(),
            name: name.to_string(),
            description: desc.to_string(),
            available: bin.is_some(),
            binary_path: bin.map(|p| p.to_string_lossy().to_string()),
        });
    }

    list
}

fn find_on_path(bin: &str) -> Option<PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        format!("{bin}.exe")
    } else {
        bin.to_string()
    };
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(&exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        for extra in [".local/bin", ".cargo/bin", ".bun/bin"] {
            let cand = home.join(extra).join(&exe_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Runs a single prompt through a supported external CLI harness in `workspace`.
pub async fn run_external_harness(
    harness_id: &str,
    workspace: &Path,
    prompt: &str,
    base_url: Option<&str>,
    model: Option<&str>,
) -> Result<String> {
    let mut cmd = match harness_id {
        "opencode" => {
            let bin = find_on_path("opencode").context("`opencode` is not installed on PATH")?;
            let mut c = Command::new(bin);
            c.arg("run").arg(prompt);
            c
        }
        "keel-cli" => {
            let bin = find_on_path("keel").context("`keel` CLI is not installed on PATH")?;
            let mut c = Command::new(bin);
            if let Some(url) = base_url {
                c.arg("--base-url").arg(url);
            }
            if let Some(m) = model {
                c.arg("--model").arg(m);
            }
            c.arg(prompt);
            c
        }
        "claude" => {
            let bin = find_on_path("claude").context("`claude` CLI is not installed on PATH")?;
            let mut c = Command::new(bin);
            c.arg("-p").arg(prompt);
            c
        }
        other => bail!("Unsupported external harness: {other}"),
    };

    cmd.current_dir(workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("Failed to launch external harness")?;
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    let res = tokio::time::timeout(Duration::from_secs(120), async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        if let Some(ref mut s) = stdout {
            let _ = s.take(128 * 1024).read_to_end(&mut out).await;
        }
        if let Some(ref mut s) = stderr {
            let _ = s.take(32 * 1024).read_to_end(&mut err).await;
        }
        let status = child.wait().await?;
        Ok::<_, anyhow::Error>((status, out, err))
    })
    .await;

    match res {
        Ok(inner) => {
            let (status, out, err) = inner?;
            let out_str = String::from_utf8_lossy(&out).trim().to_string();
            let err_str = String::from_utf8_lossy(&err).trim().to_string();
            if !status.success() && out_str.is_empty() {
                bail!("External harness failed ({status}): {err_str}");
            }
            if out_str.is_empty() {
                Ok(err_str)
            } else {
                Ok(out_str)
            }
        }
        Err(_) => {
            let _ = child.kill().await;
            bail!("External harness timed out after 120s")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_builtin_keel_harness() {
        let list = detect_harnesses();
        assert!(!list.is_empty());
        assert_eq!(list[0].id, "keel");
        assert!(list[0].available);
    }
}
