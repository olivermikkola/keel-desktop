//! Guided branch, commit, push, and GitHub pull request flow for Desktop Code.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishStatus {
    pub repo_root: String,
    pub branch: String,
    pub head: String,
    pub dirty_files: Vec<String>,
    pub remote_name: Option<String>,
    pub remote_url: Option<String>,
    pub gh_available: bool,
    pub gh_authenticated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishPlan {
    pub branch_name: String,
    pub commit_message: String,
    pub pr_title: String,
    pub pr_body: String,
    pub push_remote: bool,
    pub open_pr: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub branch: String,
    pub commit_sha: String,
    pub pushed: bool,
    pub pr_url: Option<String>,
    pub next_command: Option<String>,
}

pub fn inspect_repo(folder: &Path) -> Result<PublishStatus> {
    let folder = crate::tools::validate_workspace_root(folder)?;
    let root = normalize_path(
        PathBuf::from(text(&folder, ["rev-parse", "--show-toplevel"])?.trim_end())
            .canonicalize()?,
    );
    let branch = text(&root, ["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_owned();
    let head = text(&root, ["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_owned();
    let porcelain = text(&root, ["status", "--porcelain", "--untracked-files=all"])?;
    let dirty_files = porcelain
        .lines()
        .filter(|line| line.len() > 3)
        .map(|line| line[3..].trim().to_owned())
        .collect();
    let remote_name = text(&root, ["remote"]).ok().and_then(|out| {
        let names: Vec<_> = out
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if names.contains(&"origin") {
            Some("origin".to_owned())
        } else {
            names.first().map(|s| (*s).to_owned())
        }
    });
    let remote_url = remote_name.as_deref().and_then(|name| {
        text(&root, ["remote", "get-url", name])
            .ok()
            .map(|url| redact_remote_url(url.trim()))
    });
    let gh_available = Command::new("gh")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    let gh_authenticated = gh_available
        && Command::new("gh")
            .args(["auth", "status"])
            .output()
            .is_ok_and(|o| o.status.success());

    Ok(PublishStatus {
        repo_root: root.to_string_lossy().into_owned(),
        branch,
        head,
        dirty_files,
        remote_name,
        remote_url,
        gh_available,
        gh_authenticated,
    })
}

pub fn publish_changes(
    folder: &Path,
    plan: &PublishPlan,
    test_passed: Option<bool>,
) -> Result<PublishResult> {
    if test_passed == Some(false) {
        bail!("The latest test check failed. Fix the failure and re-run tests before publishing.");
    }
    let status = inspect_repo(folder)?;
    let root = PathBuf::from(&status.repo_root);
    let branch = plan.branch_name.trim();
    if branch.is_empty() {
        bail!("Enter a feature branch name before publishing.");
    }
    if matches!(branch, "main" | "master" | "HEAD") {
        bail!("Choose a feature branch name instead of publishing directly to `{branch}`.");
    }
    run(&root, ["check-ref-format", "--branch", branch])
        .with_context(|| format!("`{branch}` is not a valid Git branch name"))?;

    let message = plan.commit_message.trim();
    if message.is_empty() {
        bail!("Enter a commit message before publishing.");
    }
    if status.dirty_files.is_empty() && status.branch == branch {
        bail!("There are no new changes to commit on `{branch}`.");
    }

    if status.branch != branch {
        let branch_exists = git(&root)
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .status()
            .is_ok_and(|s| s.success());
        if branch_exists {
            bail!("Branch `{branch}` already exists. Choose a fresh branch name.");
        }
        run(&root, ["checkout", "-b", branch])?;
    }

    if !status.dirty_files.is_empty() {
        run(&root, ["add", "--all", "--"])?;
        run(&root, ["commit", "-m", message])?;
    }
    let commit_sha = text(&root, ["rev-parse", "--short", "HEAD"])?
        .trim()
        .to_owned();

    let mut pushed = false;
    let mut pr_url = None;
    let mut next_command = None;

    if plan.push_remote {
        let remote = status
            .remote_name
            .as_deref()
            .context("This repository has no Git remote configured.")?;
        run(&root, ["push", "-u", remote, branch])?;
        pushed = true;

        let title = if plan.pr_title.trim().is_empty() {
            message
        } else {
            plan.pr_title.trim()
        };
        let body = plan.pr_body.trim();
        if plan.open_pr && status.gh_authenticated {
            let output = Command::new("gh")
                .current_dir(&root)
                .args([
                    "pr", "create", "--head", branch, "--title", title, "--body", body,
                ])
                .output()
                .context("Could not start GitHub CLI (`gh`)")?;
            if !output.status.success() {
                bail!(
                    "Branch pushed, but `gh pr create` failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !url.is_empty() {
                pr_url = Some(url);
            }
        } else {
            next_command = Some(format!(
                "gh pr create --head {} --title {}",
                shell_quote(branch),
                shell_quote(title)
            ));
        }
    }

    Ok(PublishResult {
        branch: branch.to_owned(),
        commit_sha,
        pushed,
        pr_url,
        next_command,
    })
}

fn redact_remote_url(raw: &str) -> String {
    if let Some((scheme, rest)) = raw.split_once("://") {
        if let Some((_, host_path)) = rest.split_once('@') {
            return format!("{scheme}://{host_path}");
        }
    }
    raw.to_owned()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn normalize_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(s) = path.to_str() {
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                return PathBuf::from(format!(r"\\{rest}"));
            }
            if let Some(rest) = s.strip_prefix(r"\\?\") {
                return PathBuf::from(rest);
            }
        }
    }
    path
}

fn git(folder: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(folder)
        .args([
            "-c",
            if cfg!(windows) {
                "core.hooksPath=NUL"
            } else {
                "core.hooksPath=/dev/null"
            },
            "-c",
            "core.fsmonitor=false",
        ])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

fn run<I, S>(folder: &Path, args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git(folder)
        .args(args)
        .output()
        .context("Could not start Git")?;
    if !output.status.success() {
        bail!(
            "Git could not complete the operation: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

fn text<I, S>(folder: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    String::from_utf8(run(folder, args)?.stdout)
        .context("Git returned text that Keel could not display")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guided_publish_enforces_test_gate_blocks_main_and_pushes_feature_branch() {
        let scratch =
            std::env::temp_dir().join(format!("keel-publish-test-{}", uuid::Uuid::new_v4()));
        let remote = scratch.join("remote.git");
        let repo = scratch.join("repo");
        std::fs::create_dir_all(&remote).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        run(&remote, ["init", "--bare", "-q"]).unwrap();
        run(&repo, ["init", "-q"]).unwrap();
        run(&repo, ["config", "user.name", "Keel Test"]).unwrap();
        run(&repo, ["config", "user.email", "test@example.invalid"]).unwrap();
        run(&repo, ["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(repo.join("app.txt"), "v1\n").unwrap();
        run(&repo, ["add", "--all"]).unwrap();
        run(&repo, ["commit", "-qm", "Initial"]).unwrap();
        run(
            &repo,
            [
                OsStr::new("remote"),
                OsStr::new("add"),
                OsStr::new("origin"),
                remote.as_os_str(),
            ],
        )
        .unwrap();
        std::fs::write(repo.join("app.txt"), "v2\n").unwrap();

        let status = inspect_repo(&repo).unwrap();
        assert_eq!(status.dirty_files, vec!["app.txt"]);
        assert_eq!(status.remote_name.as_deref(), Some("origin"));

        let plan = PublishPlan {
            branch_name: "fix/v2".into(),
            commit_message: "Update app to v2".into(),
            pr_title: "Update app to v2".into(),
            pr_body: "## Summary\n- Update app".into(),
            push_remote: true,
            open_pr: false,
        };
        assert!(publish_changes(&repo, &plan, Some(false))
            .unwrap_err()
            .to_string()
            .contains("test check failed"));

        let main_plan = PublishPlan {
            branch_name: "main".into(),
            ..plan.clone()
        };
        assert!(publish_changes(&repo, &main_plan, Some(true))
            .unwrap_err()
            .to_string()
            .contains("feature branch"));

        let published = publish_changes(&repo, &plan, Some(true)).unwrap();
        assert_eq!(published.branch, "fix/v2");
        assert!(published.pushed);
        assert!(published
            .next_command
            .as_deref()
            .is_some_and(|cmd| cmd.contains("gh pr create --head 'fix/v2'")));
        let remote_ref = text(&remote, ["rev-parse", "--short", "refs/heads/fix/v2"]).unwrap();
        assert_eq!(remote_ref.trim(), published.commit_sha);

        assert_eq!(
            redact_remote_url("https://user:secret@github.com/org/repo.git"),
            "https://github.com/org/repo.git"
        );
        std::fs::remove_dir_all(scratch).unwrap();
    }
}
