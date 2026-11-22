//! Detached Code worktrees and reviewed Git patches.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const MAX_PATCH_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeCheckout {
    pub original: PathBuf,
    pub isolated: PathBuf,
    pub base: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodePatch {
    pub paths: Vec<String>,
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeTestResult {
    pub command: String,
    pub exit_code: i32,
    pub output: String,
    pub passed: bool,
}

impl CodeCheckout {
    /// Copies the selected repository's committed revision into a separate worktree.
    pub fn create(folder: &Path, destination: &Path) -> Result<Self> {
        let folder = crate::tools::validate_workspace_root(folder)?;
        let original = normalize_git_path(
            PathBuf::from(text(&folder, ["rev-parse", "--show-toplevel"])?.trim_end())
                .canonicalize()?,
        );
        let base = revision(&original)?;
        let parent = destination
            .parent()
            .context("Code worktree needs a parent folder")?;
        std::fs::create_dir_all(parent)?;
        let isolated = normalize_git_path(parent.canonicalize()?).join(
            destination
                .file_name()
                .context("Code worktree needs a directory name")?,
        );
        if isolated.starts_with(&original) || isolated.exists() {
            bail!("Choose a new worktree directory outside the original checkout");
        }
        run(
            &original,
            [
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("--detach"),
                OsStr::new("--"),
                isolated.as_os_str(),
                OsStr::new(&base),
            ],
        )?;
        Ok(Self {
            original,
            isolated,
            base,
        })
    }

    /// Include new, changed and removed tracked content by staging only in the detached worktree.
    pub fn review(&self) -> Result<CodePatch> {
        run(&self.isolated, ["add", "--all", "--"])?;
        let paths = text(
            &self.isolated,
            [
                "diff",
                "--cached",
                "--name-only",
                "-z",
                "--no-renames",
                &self.base,
                "--",
            ],
        )?
        .split_terminator('\0')
        .map(str::to_owned)
        .collect();
        let diff = text(
            &self.isolated,
            [
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "--full-index",
                &self.base,
                "--",
            ],
        )?;
        if diff.len() > MAX_PATCH_BYTES {
            bail!("The patch is larger than 2 MB. Review it in the isolated checkout.");
        }
        Ok(CodePatch { paths, diff })
    }

    /// Apply exactly the reviewed bytes. Refuse target changes while allowing unrelated dirty files.
    pub fn apply(&self, reviewed: &CodePatch) -> Result<()> {
        if &self.review()? != reviewed {
            bail!("The isolated checkout changed. Review the new patch before applying it.");
        }
        self.verify_base()?;
        if reviewed.paths.is_empty() {
            return Ok(());
        }
        let status = text(
            &self.original,
            [
                "--literal-pathspecs",
                "status",
                "--porcelain",
                "--untracked-files=all",
                "--",
            ]
            .into_iter()
            .chain(reviewed.paths.iter().map(String::as_str)),
        )?;
        if !status.is_empty() {
            bail!("A file in the patch already changed in the original checkout. Review or commit those changes first.");
        }
        apply_patch(&self.original, &reviewed.diff, false)
    }

    /// Reverse the applied patch, preserving unrelated changes to the original checkout.
    pub fn undo(&self, applied: &CodePatch) -> Result<()> {
        self.verify_base()?;
        apply_patch(&self.original, &applied.diff, true)
    }

    /// Run an explicit test or verification command inside the isolated checkout.
    pub fn run_test(&self, command_str: &str) -> Result<CodeTestResult> {
        let trimmed = command_str.trim();
        if trimmed.is_empty() {
            bail!("Enter a test command to run in the isolated checkout");
        }
        let mut command = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", trimmed]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", trimmed]);
            c
        };
        let output = command
            .current_dir(&self.isolated)
            .output()
            .with_context(|| format!("Could not run test command `{trimmed}`"))?;
        let exit_code = output.status.code().unwrap_or(-1);
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            if !combined.is_empty() && !combined.ends_with('\n') {
                combined.push('\n');
            }
            combined.push_str(&stderr);
        }
        const MAX_TEST_OUT: usize = 64 * 1024;
        if combined.len() > MAX_TEST_OUT {
            let mut end = MAX_TEST_OUT;
            while !combined.is_char_boundary(end) {
                end -= 1;
            }
            combined.truncate(end);
            combined.push_str("\n...[truncated]");
        }
        Ok(CodeTestResult {
            command: trimmed.to_owned(),
            exit_code,
            output: combined.trim_end().to_owned(),
            passed: output.status.success(),
        })
    }

    /// Cleanly detach and remove the isolated worktree from the original repository.
    pub fn remove(&self) -> Result<()> {
        if self.isolated.exists() {
            let _ = run(
                &self.original,
                [
                    OsStr::new("worktree"),
                    OsStr::new("remove"),
                    OsStr::new("--force"),
                    OsStr::new("--"),
                    self.isolated.as_os_str(),
                ],
            );
            if self.isolated.exists() {
                let _ = std::fs::remove_dir_all(&self.isolated);
            }
        }
        Ok(())
    }

    fn verify_base(&self) -> Result<()> {
        if revision(&self.original)? != self.base {
            bail!("The original checkout moved to another revision. Start a new Code run from that revision.");
        }
        Ok(())
    }
}

fn normalize_git_path(path: PathBuf) -> PathBuf {
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

fn revision(folder: &Path) -> Result<String> {
    Ok(text(folder, ["rev-parse", "--verify", "HEAD^{commit}"])?
        .trim()
        .to_owned())
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
    checked(
        git(folder)
            .args(args)
            .output()
            .context("Could not start Git")?,
    )
}

fn checked(output: Output) -> Result<Output> {
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

fn apply_patch(folder: &Path, patch: &str, reverse: bool) -> Result<()> {
    if patch.is_empty() {
        return Ok(());
    }
    for check in [true, false] {
        let mut command = git(folder);
        command.args(["apply", "--whitespace=nowarn"]);
        if reverse {
            command.arg("--reverse");
        }
        if check {
            command.arg("--check");
        }
        let mut child = command
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Could not start Git apply")?;
        let written = child
            .stdin
            .take()
            .context("Cannot send patch to Git")?
            .write_all(patch.as_bytes());
        checked(child.wait_with_output()?)?;
        written?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_patch_preserves_other_edits_and_can_be_reversed() {
        let scratch = std::env::temp_dir().join(format!("keel-worktree-{}", uuid::Uuid::new_v4()));
        let original = scratch.join("original");
        std::fs::create_dir_all(&original).unwrap();
        run(&original, ["init", "-q"]).unwrap();
        run(&original, ["config", "core.autocrlf", "false"]).unwrap();
        std::fs::write(original.join("calc.txt"), "subtract\n").unwrap();
        std::fs::write(original.join("notes.txt"), "version one\n").unwrap();
        run(&original, ["add", "--all"]).unwrap();
        run(
            &original,
            [
                "-c",
                "user.name=Keel Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "Fixture",
            ],
        )
        .unwrap();
        std::fs::write(original.join("notes.txt"), "user changed notes\n").unwrap();

        let checkout = CodeCheckout::create(&original, &scratch.join("isolated")).unwrap();
        assert_eq!(
            std::fs::read_to_string(checkout.isolated.join("notes.txt")).unwrap(),
            "version one\n"
        );
        std::fs::write(checkout.isolated.join("calc.txt"), "add\n").unwrap();
        std::fs::write(checkout.isolated.join("test.txt"), "addition gives 12\n").unwrap();
        let reviewed = checkout.review().unwrap();
        assert_eq!(reviewed.paths, ["calc.txt", "test.txt"]);
        assert!(!original.join("test.txt").exists());
        std::fs::write(original.join("calc.txt"), "user edit\n").unwrap();
        assert!(checkout
            .apply(&reviewed)
            .unwrap_err()
            .to_string()
            .contains("already changed"));
        assert!(!original.join("test.txt").exists());
        std::fs::write(original.join("calc.txt"), "subtract\n").unwrap();
        checkout.apply(&reviewed).unwrap();
        assert_eq!(
            std::fs::read_to_string(original.join("calc.txt")).unwrap(),
            "add\n"
        );
        assert!(original.join("test.txt").exists());
        checkout.undo(&reviewed).unwrap();
        assert_eq!(
            std::fs::read_to_string(original.join("calc.txt")).unwrap(),
            "subtract\n"
        );
        assert_eq!(
            std::fs::read_to_string(original.join("notes.txt")).unwrap(),
            "user changed notes\n"
        );
        assert!(!original.join("test.txt").exists());
        std::fs::write(checkout.isolated.join("calc.txt"), "changed after review\n").unwrap();
        assert!(checkout
            .apply(&reviewed)
            .unwrap_err()
            .to_string()
            .contains("changed. Review"));
        checkout.remove().unwrap();
        std::fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn run_test_captures_failing_and_passing_retest_inside_isolated_checkout() {
        let scratch =
            std::env::temp_dir().join(format!("keel-worktree-test-{}", uuid::Uuid::new_v4()));
        let original = scratch.join("original");
        std::fs::create_dir_all(&original).unwrap();
        run(&original, ["init", "-q"]).unwrap();
        run(&original, ["config", "core.autocrlf", "false"]).unwrap();
        std::fs::write(original.join("value.txt"), "wrong\n").unwrap();
        run(&original, ["add", "--all"]).unwrap();
        run(
            &original,
            [
                "-c",
                "user.name=Keel Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "Initial",
            ],
        )
        .unwrap();

        let checkout = CodeCheckout::create(&original, &scratch.join("isolated")).unwrap();
        let check_cmd = "git grep --no-index -q fixed value.txt";
        let fail = checkout.run_test(check_cmd).unwrap();
        assert!(!fail.passed);
        assert_ne!(fail.exit_code, 0);

        std::fs::write(checkout.isolated.join("value.txt"), "fixed\n").unwrap();
        let pass = checkout.run_test(check_cmd).unwrap();
        assert!(pass.passed);
        assert_eq!(pass.exit_code, 0);

        let reviewed = checkout.review().unwrap();
        checkout.apply(&reviewed).unwrap();
        assert_eq!(
            std::fs::read_to_string(original.join("value.txt")).unwrap(),
            "fixed\n"
        );
        checkout.undo(&reviewed).unwrap();
        assert_eq!(
            std::fs::read_to_string(original.join("value.txt")).unwrap(),
            "wrong\n"
        );
        checkout.remove().unwrap();
        std::fs::remove_dir_all(scratch).unwrap();
    }
}
