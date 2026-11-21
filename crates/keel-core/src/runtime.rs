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
        // A leftover runner (or a second Keel window) can still hold the usual
        // port; take a free one instead of failing with "exit status 1".
        self.port = free_port(self.port);
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
                    self.last_error = Some(
                        missing_library_hint(&details, &os_release())
                            .unwrap_or_else(|| format!("Model runner exited ({exit}). {details}")),
                    );
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

/// Returns `preferred` if nothing is listening there, otherwise any free loopback port.
pub fn free_port(preferred: u16) -> u16 {
    if std::net::TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return preferred;
    }
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(preferred)
}

/// Starts the model runner once with `--version` to prove it can load on this
/// computer. A Linux build can fail here when the system lacks a shared
/// library it was linked against (current Fedora ships OpenSSL 4, while the
/// llama.cpp Linux build needs OpenSSL 3), so the person gets the fix before
/// a multi-gigabyte model download instead of a silent "model won't load".
pub fn check_runner_starts(bin: &Path) -> Result<()> {
    let output = std::process::Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("Keel's engine could not be started ({})", bin.display()))?;
    if output.status.success() {
        return Ok(());
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    if let Some(hint) = missing_library_hint(&text, &os_release()) {
        bail!("{hint}");
    }
    bail!(
        "Keel's engine could not start on this computer: {}",
        text.trim()
    );
}

/// Turns a dynamic-loader failure into a plain sentence with the exact command
/// that installs the missing library on this Linux distribution.
pub fn missing_library_hint(output: &str, os_release: &str) -> Option<String> {
    let marker = "error while loading shared libraries: ";
    let start = output.find(marker)? + marker.len();
    let library = output[start..].split(':').next()?.trim().to_string();
    let field = |key: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{key}=")))
            .unwrap_or("")
            .trim_matches('"')
            .to_lowercase()
    };
    let family = format!("{} {}", field("ID"), field("ID_LIKE"));
    let is = |name: &str| family.split_whitespace().any(|w| w == name);
    let package = |fedora: &str, debian: &str, arch: &str| {
        if is("fedora") || is("rhel") {
            Some(format!("sudo dnf install {fedora}"))
        } else if is("debian") || is("ubuntu") {
            Some(format!("sudo apt install {debian}"))
        } else if is("arch") {
            Some(format!("sudo pacman -S {arch}"))
        } else {
            None
        }
    };
    let command = match library.as_str() {
        "libssl.so.3" | "libcrypto.so.3" => package("openssl3-libs", "libssl3", "openssl"),
        "libgomp.so.1" => package("libgomp", "libgomp1", "gcc-libs"),
        "libstdc++.so.6" => package("libstdc++", "libstdc++6", "gcc-libs"),
        "libzstd.so.1" => package("libzstd", "libzstd1", "zstd"),
        "libz.so.1" => package("zlib-ng-compat", "zlib1g", "zlib"),
        _ => None,
    };
    Some(match command {
        Some(cmd) => format!(
            "Keel's engine needs a system library this computer doesn't have yet ({library}). Open a terminal, run: {cmd} — then press Try again."
        ),
        None => format!(
            "Keel's engine needs a system library this computer doesn't have yet ({library}). Install it with your system's package manager, then press Try again."
        ),
    })
}

fn os_release() -> String {
    std::fs::read_to_string("/etc/os-release").unwrap_or_default()
}

#[cfg(test)]
mod runner_check_tests {
    use super::*;

    const FEDORA: &str = "NAME=\"Fedora Linux\"\nID=fedora\nVERSION_ID=45\n";
    const UBUNTU: &str = "ID=ubuntu\nID_LIKE=debian\n";

    #[test]
    fn fedora_openssl3_gap_names_the_compat_package() {
        let out = "/r/llama-server: error while loading shared libraries: libssl.so.3: cannot open shared object file: No such file or directory";
        let hint = missing_library_hint(out, FEDORA).unwrap();
        assert!(hint.contains("libssl.so.3"), "{hint}");
        assert!(hint.contains("sudo dnf install openssl3-libs"), "{hint}");
    }

    #[test]
    fn debian_family_gets_apt() {
        let out = "x: error while loading shared libraries: libgomp.so.1: cannot open";
        let hint = missing_library_hint(out, UBUNTU).unwrap();
        assert!(hint.contains("sudo apt install libgomp1"), "{hint}");
    }

    #[test]
    fn unknown_library_still_explains_without_a_guess() {
        let out = "x: error while loading shared libraries: libweird.so.9: cannot open";
        let hint = missing_library_hint(out, FEDORA).unwrap();
        assert!(
            hint.contains("libweird.so.9") && !hint.contains("sudo"),
            "{hint}"
        );
    }

    #[test]
    fn other_failures_are_not_called_missing_libraries() {
        assert!(missing_library_hint("Segmentation fault", FEDORA).is_none());
    }

    #[test]
    fn a_busy_port_is_swapped_for_a_free_one() {
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy = held.local_addr().unwrap().port();
        let chosen = free_port(busy);
        assert_ne!(chosen, busy);
        assert!(std::net::TcpListener::bind(("127.0.0.1", chosen)).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_working_program_passes_the_check() {
        check_runner_starts(Path::new("/usr/bin/true")).unwrap();
        assert!(check_runner_starts(Path::new("/usr/bin/false")).is_err());
    }
}
