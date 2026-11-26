//! Offline Keel Arena compare viewer host.
//!
//! Desktop owns file pickers and panel chrome. Arena owns validate / report /
//! cards / view — this module shells out to `python3 -m keel_arena view` (or
//! `py -3` / `python` on Windows) and loads the written HTML bundle into the
//! UI, then deletes the temporary output directory. No private runs, model
//! calls, or verifier bodies are pulled into the repo.

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use uuid::Uuid;

const ARENA_VIEW_TIMEOUT: Duration = Duration::from_secs(30);

struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArenaCompareView {
    pub source_path: String,
    pub source_label: String,
    pub purpose: String,
    pub result_sha256: String,
    pub matched_blocks: u64,
    pub planned_blocks: u64,
    pub excluded_blocks: u64,
    pub independent_tasks: u64,
    pub contestants: Value,
    pub protocol: Value,
    pub index_html: String,
    pub dashboard_html: String,
    pub overview_svg: Option<String>,
}

#[tauri::command]
pub async fn pick_arena_results_file(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose an Arena results JSON")
            .add_filter("Arena results", &["json"])
            .blocking_pick_file()
            .map(|selection| {
                selection
                    .into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn open_arena_compare(results_path: String) -> Result<ArenaCompareView, String> {
    tokio::task::spawn_blocking(move || open_arena_compare_sync(&results_path))
        .await
        .map_err(|error| error.to_string())?
}

fn open_arena_compare_sync(results_path: &str) -> Result<ArenaCompareView, String> {
    let source = PathBuf::from(results_path);
    if !source.is_file() {
        return Err(format!(
            "Arena results file not found: {}",
            source.display()
        ));
    }

    let out_dir = std::env::temp_dir().join(format!("keel-desktop-arena-{}", Uuid::new_v4()));
    fs::create_dir_all(&out_dir).map_err(|e| format!("Could not create temp compare dir: {e}"))?;
    let _cleanup = TempDirGuard(out_dir.clone());

    run_arena_view(&source, &out_dir)?;
    load_compare_bundle(&source, &out_dir)
}

fn python_candidates() -> &'static [(&'static str, &'static [&'static str])] {
    if cfg!(target_os = "windows") {
        &[("py", &["-3"]), ("python", &[]), ("python3", &[])]
    } else {
        &[("python3", &[]), ("python", &[])]
    }
}

fn spawn_with_timeout(
    program: &str,
    prefix: &[&str],
    source: &Path,
    out_dir: &Path,
    timeout: Duration,
) -> std::io::Result<Result<(bool, String, String), String>> {
    let mut cmd = Command::new(program);
    cmd.args(prefix)
        .args(["-m", "keel_arena", "view"])
        .arg(source)
        .arg("--out")
        .arg(out_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut r) = stdout_pipe {
            let _ = r.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    });
    let err_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(ref mut r) = stderr_pipe {
            let _ = r.read_to_end(&mut buf);
        }
        String::from_utf8_lossy(&buf).into_owned()
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out_handle.join();
                let _ = err_handle.join();
                return Ok(Err(format!(
                    "Arena view timed out after {} seconds.",
                    timeout.as_secs()
                )));
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };

    let stdout = out_handle.join().unwrap_or_default();
    let stderr = err_handle.join().unwrap_or_default();
    Ok(Ok((status.success(), stdout, stderr)))
}

fn run_arena_view(source: &Path, out_dir: &Path) -> Result<(), String> {
    let mut last_spawn_err: Option<String> = None;
    let mut missing_module = false;

    for (program, prefix) in python_candidates() {
        match spawn_with_timeout(program, prefix, source, out_dir, ARENA_VIEW_TIMEOUT) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last_spawn_err = Some(format!("{program}: {e}"));
                continue;
            }
            Err(e) => {
                last_spawn_err = Some(format!("{program}: {e}"));
                continue;
            }
            Ok(Err(timeout_msg)) => return Err(timeout_msg),
            Ok(Ok((true, _, _))) => return Ok(()),
            Ok(Ok((false, stdout, stderr))) => {
                let detail = [stderr.trim(), stdout.trim()]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .unwrap_or("unknown error");

                if detail.contains("No module named keel_arena")
                    || detail.contains("No module named 'keel_arena'")
                {
                    missing_module = true;
                    continue;
                }
                return Err(format!("Arena view failed: {detail}"));
            }
        }
    }

    if missing_module {
        return Err(
            "Keel Arena is not installed for Python. Install from https://github.com/olivermikkola/keel-arena (`pip install -e .`), then reopen this panel.".into(),
        );
    }

    let detail = last_spawn_err.unwrap_or_else(|| "no python interpreter found".into());
    Err(format!(
        "Could not run Keel Arena via Python ({detail}). Install Python and Keel Arena locally: https://github.com/olivermikkola/keel-arena"
    ))
}

fn confine_bundle_file(out_dir: &Path, raw: &str) -> Option<PathBuf> {
    let canon_root = out_dir.canonicalize().ok()?;
    let raw_path = Path::new(raw);
    let candidate = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        out_dir.join(raw_path)
    };
    let canon = candidate.canonicalize().ok()?;
    (canon.starts_with(&canon_root) && canon.is_file()).then_some(canon)
}

fn load_compare_bundle(source: &Path, out_dir: &Path) -> Result<ArenaCompareView, String> {
    let manifest_path = out_dir.join("manifest.json");
    let manifest_raw = fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "Arena view did not write manifest.json under {}: {e}",
            out_dir.display()
        )
    })?;
    let manifest: Value = serde_json::from_str(&manifest_raw)
        .map_err(|e| format!("Could not parse Arena manifest.json: {e}"))?;

    let index_html = read_required_text(out_dir, "index.html")?;
    let dashboard_path = manifest
        .get("dashboard")
        .and_then(Value::as_str)
        .and_then(|p| confine_bundle_file(out_dir, p))
        .or_else(|| confine_bundle_file(out_dir, "cards/dashboard.html"))
        .ok_or_else(|| {
            format!(
                "Could not find Arena dashboard HTML inside {}",
                out_dir.display()
            )
        })?;
    let dashboard_html = fs::read_to_string(&dashboard_path).map_err(|e| {
        format!(
            "Could not read Arena dashboard HTML ({}): {e}",
            dashboard_path.display()
        )
    })?;

    let overview_svg = ["cards/overview.svg", "overview.svg"]
        .into_iter()
        .find_map(|rel| confine_bundle_file(out_dir, rel))
        .and_then(|p| fs::read_to_string(p).ok());

    let source_label = source
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| source.display().to_string());

    Ok(ArenaCompareView {
        source_path: source.to_string_lossy().into_owned(),
        source_label,
        purpose: manifest_string(&manifest, "purpose").unwrap_or_else(|| "unknown".into()),
        result_sha256: manifest_string(&manifest, "result_sha256").unwrap_or_default(),
        matched_blocks: manifest_u64(&manifest, "matched_blocks"),
        planned_blocks: manifest_u64(&manifest, "planned_blocks"),
        excluded_blocks: manifest_u64(&manifest, "excluded_blocks"),
        independent_tasks: manifest_u64(&manifest, "independent_tasks"),
        contestants: manifest
            .get("contestants")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default())),
        protocol: manifest
            .get("protocol")
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default())),
        index_html,
        dashboard_html,
        overview_svg,
    })
}

fn read_required_text(root: &Path, rel: &str) -> Result<String, String> {
    let path = confine_bundle_file(root, rel)
        .ok_or_else(|| format!("Missing Arena output {}", root.join(rel).display()))?;
    fs::read_to_string(&path).map_err(|e| format!("Could not read {}: {e}", path.display()))
}

fn manifest_string(manifest: &Value, key: &str) -> Option<String> {
    manifest.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn manifest_u64(manifest: &Value, key: &str) -> u64 {
    manifest
        .get(key)
        .and_then(Value::as_u64)
        .or_else(|| manifest.get(key).and_then(Value::as_f64).map(|n| n as u64))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn manifest_helpers_read_public_shape() {
        let manifest = json!({
            "purpose": "demonstration",
            "result_sha256": "abc",
            "matched_blocks": 7,
            "planned_blocks": 7.0,
            "excluded_blocks": 0,
            "independent_tasks": 7,
            "contestants": {"demo": {"grade": "A"}},
            "protocol": {"harness_revision": "aa"}
        });
        assert_eq!(
            manifest_string(&manifest, "purpose").as_deref(),
            Some("demonstration")
        );
        assert_eq!(manifest_u64(&manifest, "matched_blocks"), 7);
        assert_eq!(manifest_u64(&manifest, "planned_blocks"), 7);
        assert_eq!(manifest_u64(&manifest, "missing"), 0);
    }

    #[test]
    fn load_compare_bundle_reads_written_layout_and_confines_paths() {
        let root = std::env::temp_dir().join(format!("keel_arena_bundle_test_{}", Uuid::new_v4()));
        let outside =
            std::env::temp_dir().join(format!("keel_arena_outside_{}.html", Uuid::new_v4()));
        fs::write(&outside, "<html>SECRET_OUTSIDE</html>").unwrap();

        let cards = root.join("cards");
        fs::create_dir_all(&cards).unwrap();
        // A manifest pointing outside `out_dir` must be ignored in favor of the confined bundle.
        fs::write(
            root.join("manifest.json"),
            json!({
                "purpose": "demonstration",
                "result_sha256": "deadbeef",
                "matched_blocks": 2,
                "planned_blocks": 2,
                "excluded_blocks": 0,
                "independent_tasks": 2,
                "contestants": {},
                "protocol": {},
                "index": root.join("index.html").to_string_lossy(),
                "dashboard": outside.to_string_lossy()
            })
            .to_string(),
        )
        .unwrap();
        fs::write(root.join("index.html"), "<html>compare</html>").unwrap();
        fs::write(cards.join("dashboard.html"), "<html>dash</html>").unwrap();
        fs::write(cards.join("overview.svg"), "<svg></svg>").unwrap();

        let source = root.join("results.json");
        fs::write(&source, "{}").unwrap();
        let view = load_compare_bundle(&source, &root).unwrap();
        assert_eq!(view.purpose, "demonstration");
        assert_eq!(view.result_sha256, "deadbeef");
        assert_eq!(view.index_html, "<html>compare</html>");
        assert_eq!(view.dashboard_html, "<html>dash</html>");
        assert!(!view.dashboard_html.contains("SECRET_OUTSIDE"));
        assert_eq!(view.overview_svg.as_deref(), Some("<svg></svg>"));
        assert_eq!(view.source_label, "results.json");

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn temp_dir_guard_cleans_up_bundle_directory() {
        let dir = std::env::temp_dir().join(format!("keel_arena_guard_test_{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.html"), "ok").unwrap();
        {
            let _guard = TempDirGuard(dir.clone());
            assert!(dir.is_dir());
        }
        assert!(!dir.exists());
    }

    #[test]
    fn open_arena_compare_runs_fixture_when_configured_and_arena_installed() {
        let Ok(fixture_env) = std::env::var("KEEL_ARENA_FIXTURE") else {
            return;
        };
        let fixture = PathBuf::from(fixture_env);
        if !fixture.is_file() {
            return;
        }
        let view = open_arena_compare_sync(fixture.to_str().unwrap()).unwrap();
        assert_eq!(view.purpose, "demonstration");
        assert!(!view.result_sha256.is_empty());
        assert!(
            view.index_html.contains("Keel Arena Compare") || view.index_html.contains("compare")
        );
        assert!(view.dashboard_html.to_lowercase().contains("html"));
        assert!(view.matched_blocks > 0);
    }
}
