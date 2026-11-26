//! Offline Keel Arena compare viewer host.
//!
//! Desktop owns file pickers and panel chrome. Arena owns validate / report /
//! cards / view — this module shells out to `python3 -m keel_arena view` and
//! loads the written HTML bundle into the UI. No private runs, model calls, or
//! verifier bodies are pulled into the repo.

use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::AppHandle;
use uuid::Uuid;

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
    pub out_dir: String,
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

    run_arena_view(&source, &out_dir)?;
    load_compare_bundle(&source, &out_dir)
}

fn run_arena_view(source: &Path, out_dir: &Path) -> Result<(), String> {
    let output = Command::new("python3")
        .args([
            "-m",
            "keel_arena",
            "view",
            &source.to_string_lossy(),
            "--out",
            &out_dir.to_string_lossy(),
        ])
        .output()
        .map_err(|e| {
            format!(
                "Could not run python3 -m keel_arena view ({e}). Install Keel Arena locally: https://github.com/olivermikkola/keel-arena"
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("unknown error");

    if detail.contains("No module named keel_arena")
        || detail.contains("No module named 'keel_arena'")
    {
        return Err(
            "Keel Arena is not installed for python3. Install from https://github.com/olivermikkola/keel-arena (`pip install -e .`), then reopen this panel.".into(),
        );
    }

    Err(format!("Arena view failed: {detail}"))
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
    let dashboard_rel = manifest
        .get("dashboard")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .unwrap_or_else(|| out_dir.join("cards/dashboard.html"));
    let dashboard_html = fs::read_to_string(&dashboard_rel).map_err(|e| {
        format!(
            "Could not read Arena dashboard HTML ({}): {e}",
            dashboard_rel.display()
        )
    })?;

    let overview_svg = ["cards/overview.svg", "overview.svg"]
        .into_iter()
        .map(|rel| out_dir.join(rel))
        .find(|p| p.is_file())
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
        out_dir: out_dir.to_string_lossy().into_owned(),
    })
}

fn read_required_text(root: &Path, rel: &str) -> Result<String, String> {
    let path = root.join(rel);
    fs::read_to_string(&path).map_err(|e| format!("Missing Arena output {} ({e})", path.display()))
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
    fn load_compare_bundle_reads_written_layout() {
        let root = std::env::temp_dir().join(format!("keel_arena_bundle_test_{}", Uuid::new_v4()));
        let cards = root.join("cards");
        fs::create_dir_all(&cards).unwrap();
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
                "dashboard": cards.join("dashboard.html").to_string_lossy()
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
        assert_eq!(view.overview_svg.as_deref(), Some("<svg></svg>"));
        assert_eq!(view.source_label, "results.json");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn open_arena_compare_runs_public_fixture_when_arena_installed() {
        let fixture = PathBuf::from("/tmp/keel-arena/examples/synthetic-gauntlet-results.json");
        if !fixture.is_file() {
            // Cloud/dev machines without the Arena checkout skip the live CLI proof.
            return;
        }
        let probe = Command::new("python3")
            .args(["-c", "import keel_arena"])
            .status();
        if !probe.map(|s| s.success()).unwrap_or(false) {
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
        let _ = fs::remove_dir_all(PathBuf::from(&view.out_dir));
    }
}
