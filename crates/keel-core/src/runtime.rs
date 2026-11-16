use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub state: String,
    pub base_url: String,
    pub active_model_id: Option<String>,
    pub active_model_path: Option<String>,
    pub llama_server_bin: Option<String>,
    pub managed_pid: Option<u32>,
    pub error: Option<String>,
}

pub struct RuntimeManager {
    child: Option<Child>,
    pub base_url: String,
    pub active_model_id: Option<String>,
    pub active_model_path: Option<String>,
    pub llama_server_bin: Option<String>,
    pub port: u16,
    last_error: Option<String>,
}

impl Default for RuntimeManager {
    fn default() -> Self {
        Self::new(18180)
    }
}

impl RuntimeManager {
    pub fn new(port: u16) -> Self {
        let llama_server_bin = find_llama_server(None).map(|p| p.to_string_lossy().to_string());
        Self {
            child: None,
            base_url: format!("http://127.0.0.1:{port}/v1"),
            active_model_id: None,
            active_model_path: None,
            llama_server_bin,
            port,
            last_error: None,
        }
    }

    pub fn set_external_url(&mut self, url: &str, model_id: Option<String>) {
        self.stop_sync();
        self.base_url = url.trim_end_matches('/').to_string();
        self.active_model_id = model_id.or_else(|| Some("default".to_string()));
        self.active_model_path = None;
    }

    pub async fn start_model(
        &mut self,
        model_id: &str,
        model_path: &Path,
        context_size: u32,
        custom_bin: Option<&str>,
    ) -> Result<RuntimeStatus> {
        if !model_path.is_file() {
            bail!("Model file not found: {}", model_path.display());
        }
        let bin = find_llama_server(custom_bin).context(
            "Could not find `llama-server`. Install llama.cpp or set its path in Models.",
        )?;
        self.stop().await;
        self.llama_server_bin = Some(bin.to_string_lossy().to_string());
        self.base_url = format!("http://127.0.0.1:{}/v1", self.port);

        let log_dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("keel-desktop");
        std::fs::create_dir_all(&log_dir)?;
        let log_path = log_dir.join("model-runner.log");
        let log = std::fs::File::create(&log_path)?;
        let child = Command::new(&bin)
            .arg("-m")
            .arg(model_path)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(self.port.to_string())
            .arg("-c")
            .arg(context_size.to_string())
            .arg("--parallel")
            .arg("1")
            .arg("--alias")
            .arg(model_id)
            .arg("--jinja")
            .stdout(Stdio::null())
            .stderr(Stdio::from(log))
            .spawn()
            .with_context(|| format!("Failed to start {}", bin.display()))?;

        self.child = Some(child);
        self.active_model_id = Some(model_id.to_string());
        self.active_model_path = Some(model_path.to_string_lossy().to_string());

        let health_url = format!("http://127.0.0.1:{}/health", self.port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;

        for _ in 0..90 {
            if let Some(ref mut ch) = self.child {
                if let Ok(Some(exit)) = ch.try_wait() {
                    self.child = None;
                    self.active_model_id = None;
                    self.active_model_path = None;
                    let details = std::fs::read_to_string(&log_path)
                        .unwrap_or_default()
                        .lines()
                        .rev()
                        .take(8)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.last_error = Some(format!("Model runner exited ({exit}). {details}"));
                    bail!(
                        "{}",
                        self.last_error.as_deref().unwrap_or("Model runner failed")
                    );
                }
            }
            if let Ok(resp) = client.get(&health_url).send().await {
                if resp.status().is_success() {
                    return Ok(self.status());
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        self.stop().await;
        bail!(
            "Timed out waiting for llama-server to become healthy on port {}",
            self.port
        );
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        self.active_model_path = None;
        self.active_model_id = None;
        self.last_error = None;
    }

    pub fn stop_sync(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        self.active_model_path = None;
        self.active_model_id = None;
        self.last_error = None;
    }

    pub fn status(&mut self) -> RuntimeStatus {
        if let Some(child) = &mut self.child {
            if let Ok(Some(exit)) = child.try_wait() {
                self.child = None;
                self.active_model_id = None;
                self.active_model_path = None;
                self.last_error = Some(format!("The model runner stopped ({exit}). Reload a smaller model or check its runner log."));
            }
        }
        let state = if self.child.is_some() {
            "running"
        } else if self.active_model_id.is_some() {
            "external"
        } else if self.last_error.is_some() {
            "error"
        } else {
            "stopped"
        };
        RuntimeStatus {
            state: state.to_string(),
            base_url: self.base_url.clone(),
            active_model_id: self.active_model_id.clone(),
            active_model_path: self.active_model_path.clone(),
            llama_server_bin: self.llama_server_bin.clone(),
            managed_pid: self.child.as_ref().and_then(|c| c.id()),
            error: self.last_error.clone(),
        }
    }
}

impl Drop for RuntimeManager {
    fn drop(&mut self) {
        self.stop_sync();
    }
}

/// Locates `llama-server` from an explicit setting, PATH, or common local build folders.
pub fn find_llama_server(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit.filter(|s| !s.trim().is_empty()) {
        let pb = PathBuf::from(p.trim());
        if pb.is_file() {
            return Some(pb);
        }
    }
    if let Ok(path_var) = std::env::var("PATH") {
        let bin_name = if cfg!(target_os = "windows") {
            "llama-server.exe"
        } else {
            "llama-server"
        };
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(bin_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        let common = [
            home.join(".local/bin/llama-server"),
            home.join("Projects/llama.cpp/build/bin/llama-server"),
            home.join("Projects/llama.cpp-b12367/build/bin/llama-server"),
        ];
        for cand in common {
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Queries `/v1/models` on an OpenAI-compatible server to list available model IDs.
pub async fn probe_openai_models(base_url: &str) -> Result<Vec<String>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()?;
    let val: serde_json::Value = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let ids = val
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stopping_clears_identity_instead_of_turning_local_into_external() {
        let mut runtime = RuntimeManager::new(18180);
        runtime.active_model_id = Some("local-model".into());
        runtime.active_model_path = Some("model.gguf".into());
        runtime.stop().await;
        assert_eq!(runtime.status().state, "stopped");
        assert!(runtime.active_model_id.is_none());
        assert!(runtime.active_model_path.is_none());
        runtime.set_external_url("http://127.0.0.1:8080/v1/", Some("external-model".into()));
        assert_eq!(runtime.status().state, "external");
        assert_eq!(runtime.base_url, "http://127.0.0.1:8080/v1");
        runtime.stop_sync();
        assert_eq!(runtime.status().state, "stopped");
    }
}
