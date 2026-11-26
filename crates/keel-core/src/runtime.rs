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
    #[serde(default)]
    pub active_context_size: Option<u32>,
    pub llama_server_bin: Option<String>,
    pub managed_pid: Option<u32>,
    pub error: Option<String>,
    #[serde(default)]
    pub error_details: Option<String>,
}

/// What was true the last time the managed runner was started.
/// A later message reuses the process when this still matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedLaunch {
    pub model_id: String,
    pub model_path: String,
    pub context_size: u32,
    pub bin: String,
    pub run_where: String,
    pub graphics_share: String,
    pub offload_args: Vec<String>,
    /// The engine path from Settings, not the build that was actually launched.
    pub settings_bin: Option<String>,
}

pub fn same_loaded_launch(live: bool, current: &LoadedLaunch, next: &LoadedLaunch) -> bool {
    live && current == next
}

/// True when the loaded launch is using graphics offload (not `--device none`).
pub fn launch_uses_graphics(loaded: &LoadedLaunch) -> bool {
    !loaded
        .offload_args
        .windows(2)
        .any(|pair| pair[0] == "--device" && pair[1] == "none")
        && loaded
            .offload_args
            .iter()
            .any(|arg| arg == "--device" || arg == "--gpu-layers")
}

/// Whether keep-alive should stop so a pack can claim free VRAM.
pub fn should_stop_keep_alive_for_pack(
    inventory: &crate::hardware::VramInventory,
    pack: &crate::packs::PackManifest,
    loaded: Option<&LoadedLaunch>,
) -> bool {
    let Some(loaded) = loaded else {
        return false;
    };
    crate::packs::should_unload_for_pack(inventory, pack, launch_uses_graphics(loaded))
}

/// Outcome of preparing VRAM for a capability pack (keep-alive may stop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackVramStatus {
    /// Keep-alive was (or should be) stopped so the pack can claim graphics memory.
    Unloaded,
    /// Enough free VRAM already; keep-alive can stay.
    AlreadyFree,
    /// Pack should run on the processor (no graphics fit).
    ProcessorFallback,
}

/// Decision returned by [`plan_pack_vram`] before the shell stops the runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackVramPreparation {
    pub status: PackVramStatus,
    pub reason: String,
    /// When true, the desktop shell must stop the keep-alive runtime.
    pub stop_keep_alive: bool,
}

/// Decide whether to unload keep-alive or fall back to the processor for this pack.
pub fn plan_pack_vram(
    inventory: &crate::hardware::VramInventory,
    pack: &crate::packs::PackManifest,
    loaded: Option<&LoadedLaunch>,
) -> PackVramPreparation {
    if should_stop_keep_alive_for_pack(inventory, pack, loaded) {
        return PackVramPreparation {
            status: PackVramStatus::Unloaded,
            reason: format!(
                "Stopping the keep-alive model so pack '{}' can use graphics memory (needs {} MiB; {} MiB free).",
                pack.capability_id, pack.min_vram_mib, inventory.usable_free_mib
            ),
            stop_keep_alive: true,
        };
    }
    match crate::packs::place_pack(inventory, pack) {
        crate::packs::PackPlacement::Graphics {
            usable_free_mib, ..
        } => PackVramPreparation {
            status: PackVramStatus::AlreadyFree,
            reason: format!(
                "Pack '{}' can use graphics memory ({} MiB free; needs {} MiB).",
                pack.capability_id, usable_free_mib, pack.min_vram_mib
            ),
            stop_keep_alive: false,
        },
        crate::packs::PackPlacement::Processor { reason } => PackVramPreparation {
            status: PackVramStatus::ProcessorFallback,
            reason,
            stop_keep_alive: false,
        },
    }
}

pub struct ModelLaunch<'a> {
    pub model_id: &'a str,
    pub model_path: &'a Path,
    pub context_size: u32,
    pub custom_bin: Option<&'a str>,
    pub offload_args: &'a [String],
    pub run_where: &'a str,
    pub graphics_share: &'a str,
    pub settings_bin: Option<&'a str>,
}

pub struct RuntimeManager {
    child: Option<Child>,
    pub base_url: String,
    pub active_model_id: Option<String>,
    pub active_model_path: Option<String>,
    pub active_context_size: Option<u32>,
    pub llama_server_bin: Option<String>,
    pub port: u16,
    log_path: Option<PathBuf>,
    last_error: Option<String>,
    last_error_details: Option<String>,
    loaded: Option<LoadedLaunch>,
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
            active_context_size: None,
            llama_server_bin,
            port,
            log_path: None,
            last_error: None,
            last_error_details: None,
            loaded: None,
        }
    }

    pub fn set_external_url(&mut self, url: &str, model_id: Option<String>) {
        self.stop_sync();
        self.base_url = url.trim_end_matches('/').to_string();
        self.active_model_id = model_id.or_else(|| Some("default".to_string()));
        self.active_model_path = None;
        self.active_context_size = None;
        self.loaded = None;
    }

    pub fn is_managed(&self) -> bool {
        self.child.is_some()
    }

    /// The launch identity of the keep-alive / managed runner, when one is loaded.
    pub fn loaded_launch(&self) -> Option<&LoadedLaunch> {
        self.loaded.as_ref()
    }

    /// True when the managed runner is already serving this model, context,
    /// placement, and (when the person set one) engine path.
    pub fn still_loaded(
        &mut self,
        model_id: &str,
        model_path: &str,
        context_size: u32,
        run_where: &str,
        graphics_share: &str,
        settings_bin: Option<&str>,
    ) -> bool {
        if self.status().state != "running" {
            return false;
        }
        let Some(loaded) = &self.loaded else {
            return false;
        };
        let wanted_bin = settings_bin.filter(|bin| !bin.trim().is_empty());
        loaded.model_id == model_id
            && loaded.model_path == model_path
            && loaded.context_size == context_size
            && loaded.run_where == run_where
            && loaded.graphics_share == graphics_share
            && loaded.settings_bin.as_deref() == wanted_bin
    }

    pub async fn start_model(&mut self, launch: ModelLaunch<'_>) -> Result<RuntimeStatus> {
        if !launch.model_path.is_file() {
            bail!("This model isn't downloaded yet. Download it in Models first.");
        }
        let bin = find_llama_server(launch.custom_bin).context(
            "Keel's built-in AI engine isn't installed yet. Open Models and click Set it up for me.",
        )?;
        let next = LoadedLaunch {
            model_id: launch.model_id.to_string(),
            model_path: launch.model_path.to_string_lossy().into_owned(),
            context_size: launch.context_size,
            bin: bin.to_string_lossy().into_owned(),
            run_where: launch.run_where.to_string(),
            graphics_share: launch.graphics_share.to_string(),
            offload_args: launch.offload_args.to_vec(),
            settings_bin: launch
                .settings_bin
                .filter(|bin| !bin.trim().is_empty())
                .map(str::to_string),
        };
        let live = self.status().state == "running";
        if live {
            if let Some(current) = &self.loaded {
                if same_loaded_launch(true, current, &next) {
                    return Ok(self.status());
                }
            }
        }
        self.stop().await;
        self.llama_server_bin = Some(next.bin.clone());
        // A leftover runner (or a second Keel window) can still hold the usual
        // port; take a free one instead of failing with "exit status 1".
        let reserved_port =
            reserve_port(self.port).context("Could not prepare a local slot for the AI engine")?;
        self.port = reserved_port.local_addr()?.port();
        self.base_url = format!("http://127.0.0.1:{}/v1", self.port);

        let log_dir = dirs::cache_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("keel-desktop");
        std::fs::create_dir_all(&log_dir)?;
        let log_path = log_dir.join("model-runner.log");
        self.log_path = Some(log_path.clone());
        let log = std::fs::File::create(&log_path)?;
        let mut command = Command::new(&bin);
        apply_tokio_runner_env(&mut command, &bin);
        command
            .arg("-m")
            .arg(launch.model_path)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(self.port.to_string())
            .arg("-c")
            .arg(launch.context_size.to_string())
            .arg("--parallel")
            .arg("1")
            .arg("--alias")
            .arg(launch.model_id)
            .arg("--jinja");
        // Quantize KV cache to Q8_0 for 32K+ context windows so long sessions fit in
        // VRAM/RAM with half the memory footprint and negligible quality loss.
        if launch.context_size >= 32_768 {
            command.arg("-ctk").arg("q8_0").arg("-ctv").arg("q8_0");
        }
        // Graphics-or-processor flags from hardware::advise. They stay out of the UI.
        for arg in launch.offload_args {
            command.arg(arg);
        }
        command.stdout(Stdio::null()).stderr(Stdio::from(log));
        // llama-server binds its own socket. Keep the reservation until the
        // command and its log are ready, then release it immediately before spawn.
        drop(reserved_port);
        let child = command
            .spawn()
            .context("Keel's built-in AI engine couldn't start.")?;

        self.child = Some(child);
        self.active_model_id = Some(next.model_id.clone());
        self.active_model_path = Some(next.model_path.clone());
        self.active_context_size = Some(next.context_size);
        self.loaded = Some(next);

        let health_url = format!("http://127.0.0.1:{}/health", self.port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;

        // Allow up to 120 seconds for large 16 GB+ GGUF models to load on cold disk or GPU.
        for _ in 0..240 {
            if let Some(ref mut ch) = self.child {
                if let Ok(Some(_exit)) = ch.try_wait() {
                    self.child = None;
                    self.active_model_id = None;
                    self.active_model_path = None;
                    self.active_context_size = None;
                    self.loaded = None;
                    let details = std::fs::read_to_string(&log_path)
                        .unwrap_or_default()
                        .lines()
                        .rev()
                        .take(16)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n");
                    self.last_error = Some(friendly_runner_exit_message(&details, &os_release()));
                    self.last_error_details = (!details.trim().is_empty()).then_some(details);
                    bail!(
                        "{}",
                        self.last_error
                            .as_deref()
                            .unwrap_or("This model couldn't start on this computer.")
                    );
                }
            }
            if let Ok(resp) = client.get(&health_url).send().await {
                if resp.status().is_success() {
                    self.last_error = None;
                    self.last_error_details = None;
                    return Ok(self.status());
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        self.stop().await;
        bail!("Your AI took too long to wake up. Try a lighter model in Models.");
    }

    pub async fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        self.active_model_path = None;
        self.active_model_id = None;
        self.active_context_size = None;
        self.loaded = None;
        self.last_error = None;
        self.last_error_details = None;
    }

    pub fn stop_sync(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let _ = child.try_wait();
        }
        self.active_model_path = None;
        self.active_model_id = None;
        self.active_context_size = None;
        self.loaded = None;
        self.last_error = None;
        self.last_error_details = None;
    }

    pub fn status(&mut self) -> RuntimeStatus {
        if let Some(child) = &mut self.child {
            if let Ok(Some(_exit)) = child.try_wait() {
                self.child = None;
                self.active_model_id = None;
                self.active_model_path = None;
                self.active_context_size = None;
                self.loaded = None;
                let details = self
                    .log_path
                    .as_deref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .map(|raw| {
                        raw.lines()
                            .rev()
                            .take(16)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let msg = if details.trim().is_empty() {
                    "Your AI stopped unexpectedly. Send a message or click Start to wake it back up, or try a lighter model."
                        .to_string()
                } else {
                    friendly_runner_exit_message(&details, &os_release())
                };
                self.last_error = Some(msg);
                self.last_error_details = (!details.trim().is_empty()).then_some(details);
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
            active_context_size: self.active_context_size,
            llama_server_bin: self.llama_server_bin.clone(),
            managed_pid: self.child.as_ref().and_then(|c| c.id()),
            error: self.last_error.clone(),
            error_details: self.last_error_details.clone(),
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
    probe_openai_models_with_auth(base_url, None).await
}

/// Like [`probe_openai_models`], optionally sending `Authorization: Bearer <key>`.
pub async fn probe_openai_models_with_auth(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>> {
    let trimmed = base_url.trim().trim_end_matches('/');
    let parsed =
        reqwest::Url::parse(trimmed).context("Enter a valid http:// or https:// server URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        bail!("Server URL must use http:// or https://");
    }
    let url = format!("{trimmed}/models");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut request = client.get(&url);
    if let Some(auth) = crate::remote::bearer_authorization(api_key) {
        request = request.header(reqwest::header::AUTHORIZATION, auth);
    }
    let mut resp = request.send().await?.error_for_status()?;
    let mut body = Vec::new();
    const MAX_PROBE_BYTES: usize = 2 * 1024 * 1024;
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > MAX_PROBE_BYTES {
            bail!("Server returned an unexpectedly large models list");
        }
        body.extend_from_slice(&chunk);
    }
    let val: serde_json::Value = serde_json::from_slice(&body)?;
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

    #[test]
    fn the_same_launch_stays_loaded_and_a_new_placement_does_not() {
        let current = LoadedLaunch {
            model_id: "bonsai-4b".into(),
            model_path: "/models/bonsai.gguf".into(),
            context_size: 8192,
            bin: "/runtime/llama-server".into(),
            run_where: "auto".into(),
            graphics_share: "auto".into(),
            offload_args: vec![
                "--device".into(),
                "CUDA0".into(),
                "--gpu-layers".into(),
                "all".into(),
            ],
            settings_bin: None,
        };
        assert!(same_loaded_launch(true, &current, &current.clone()));
        assert!(!same_loaded_launch(false, &current, &current.clone()));
        assert!(launch_uses_graphics(&current));
        let mut processor = current.clone();
        processor.run_where = "processor".into();
        processor.offload_args = vec![
            "--device".into(),
            "none".into(),
            "--gpu-layers".into(),
            "0".into(),
        ];
        assert!(!same_loaded_launch(true, &current, &processor));
        assert!(!launch_uses_graphics(&processor));
    }

    fn sample_graphics_loaded() -> LoadedLaunch {
        LoadedLaunch {
            model_id: "bonsai-4b".into(),
            model_path: "/models/bonsai.gguf".into(),
            context_size: 8192,
            bin: "/runtime/llama-server".into(),
            run_where: "auto".into(),
            graphics_share: "auto".into(),
            offload_args: vec![
                "--device".into(),
                "CUDA0".into(),
                "--gpu-layers".into(),
                "all".into(),
            ],
            settings_bin: None,
        }
    }

    fn sample_inventory(free: u64) -> crate::hardware::VramInventory {
        use crate::hardware::{DeviceKind, GraphicsDevice, InventorySource, VramInventory};
        let device = GraphicsDevice {
            id: "CUDA0".into(),
            name: "NVIDIA".into(),
            total_mib: 8192,
            free_mib: free,
            kind: DeviceKind::Discrete,
        };
        VramInventory {
            usable_free_mib: free,
            total_mib: 8192,
            kind: "discrete".into(),
            source: InventorySource::ListDevices,
            preferred: Some(device.clone()),
            devices: vec![device],
        }
    }

    #[test]
    fn keep_alive_releases_graphics_when_a_pack_needs_more_free_vram() {
        use crate::packs::{PackManifest, RunnerKind};

        let inv = sample_inventory(600);
        let loaded = sample_graphics_loaded();
        let pack = PackManifest::new("audio.stt", 2048, RunnerKind::AudioStt);
        assert!(should_stop_keep_alive_for_pack(&inv, &pack, Some(&loaded)));
        assert!(!should_stop_keep_alive_for_pack(&inv, &pack, None));
    }

    #[test]
    fn plan_pack_vram_unloads_keep_alive_when_free_vram_is_short() {
        use crate::packs::{PackManifest, RunnerKind};

        let inv = sample_inventory(600);
        let loaded = sample_graphics_loaded();
        let pack = PackManifest::new("audio.stt", 2048, RunnerKind::AudioStt);
        let plan = plan_pack_vram(&inv, &pack, Some(&loaded));
        assert_eq!(plan.status, PackVramStatus::Unloaded);
        assert!(plan.stop_keep_alive);
        assert!(plan.reason.contains("audio.stt"));
        assert!(plan.reason.contains("2048"));
    }

    #[test]
    fn plan_pack_vram_reports_already_free_when_inventory_covers_the_pack() {
        use crate::packs::{PackManifest, RunnerKind};

        let inv = sample_inventory(4096);
        let loaded = sample_graphics_loaded();
        let pack = PackManifest::new("audio.stt", 2048, RunnerKind::AudioStt);
        let plan = plan_pack_vram(&inv, &pack, Some(&loaded));
        assert_eq!(plan.status, PackVramStatus::AlreadyFree);
        assert!(!plan.stop_keep_alive);
        assert!(plan.reason.contains("4096"));
    }

    #[test]
    fn plan_pack_vram_falls_back_to_processor_without_unloading() {
        use crate::packs::{PackManifest, RunnerKind};

        let inv = sample_inventory(400);
        let pack = PackManifest::new("audio.stt", 2048, RunnerKind::AudioStt);
        // No keep-alive loaded: cannot unload; pack must use the processor.
        let plan = plan_pack_vram(&inv, &pack, None);
        assert_eq!(plan.status, PackVramStatus::ProcessorFallback);
        assert!(!plan.stop_keep_alive);
        assert!(plan.reason.contains("2048"));
    }

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

/// Hold the preferred loopback port or an OS-assigned fallback until launch.
fn reserve_port(preferred: u16) -> std::io::Result<std::net::TcpListener> {
    match std::net::TcpListener::bind(("127.0.0.1", preferred)) {
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            std::net::TcpListener::bind(("127.0.0.1", 0))
        }
        result => result,
    }
}

#[cfg(target_os = "linux")]
fn runner_ld_library_path(bin: &Path) -> Option<std::ffi::OsString> {
    let dir = bin.parent()?;
    let mut paths = vec![dir.to_path_buf()];
    if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).ok()
}

pub(crate) fn apply_std_runner_env(cmd: &mut std::process::Command, bin: &Path) {
    #[cfg(target_os = "linux")]
    if let Some(val) = runner_ld_library_path(bin) {
        cmd.env("LD_LIBRARY_PATH", val);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (cmd, bin);
}

fn apply_tokio_runner_env(cmd: &mut Command, bin: &Path) {
    #[cfg(target_os = "linux")]
    if let Some(val) = runner_ld_library_path(bin) {
        cmd.env("LD_LIBRARY_PATH", val);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (cmd, bin);
}

/// Parses the `llama-server --version` build number (e.g. `(build 12772, ...)`).
pub fn runner_build_number(bin: &Path) -> Option<u64> {
    let mut cmd = std::process::Command::new(bin);
    apply_std_runner_env(&mut cmd, bin);
    let output = cmd.arg("--version").stdin(Stdio::null()).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let idx = text.find("(build ")?;
    let rest = &text[idx + 7..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// Starts the model runner once with `--version` to prove it can load on this
/// computer. A Linux build can fail here when the system lacks a shared
/// library it was linked against (current Fedora ships OpenSSL 4, while the
/// llama.cpp Linux build needs OpenSSL 3), so the person gets the fix before
/// a multi-gigabyte model download instead of a silent "model won't load".
pub fn check_runner_starts(bin: &Path) -> Result<()> {
    let mut cmd = std::process::Command::new(bin);
    apply_std_runner_env(&mut cmd, bin);
    let output = cmd
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

/// Returns true if a URL points to the local machine (127.x.x.x, localhost, or ::1).
pub fn is_loopback_url(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw.trim()) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Detects Keel's own default internal runner address (`127.0.0.1:18180`).
/// If saved as an "external" endpoint from an older session or config, it would
/// trap the app in a broken state because nothing listens there until Keel starts.
pub fn is_default_managed_loopback(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw.trim()) else {
        return false;
    };
    is_loopback_url(raw) && url.port() == Some(18180)
}

/// Converts raw runner stderr into a plain-language explanation without dumping
/// exit codes, ports, or internal tool names to the user.
pub fn friendly_runner_exit_message(details: &str, os_release: &str) -> String {
    if let Some(hint) = missing_library_hint(details, os_release) {
        return hint;
    }
    let lower = details.to_ascii_lowercase();
    if [
        "out of memory",
        "failed to allocate",
        "bad_alloc",
        "cannot allocate memory",
        "mmap failed",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return "This computer doesn't have enough free memory to load this model. Try a lighter model in Models."
            .to_string();
    }
    if lower.contains("unknown model architecture") {
        return "This model uses an architecture that your current engine doesn't support yet. Update the engine or pick a different model."
            .to_string();
    }
    if [
        "failed to load model",
        "invalid gguf",
        "gguf_init_from_file",
        "not a gguf",
        "corrupt",
        "truncated",
        "unexpected eof",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return "This model file looks damaged and couldn't load. Repair it in Models, or pick a different model."
            .to_string();
    }
    "This model couldn't start on this computer. Try a lighter model in Models.".to_string()
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
    fn runner_exit_messages_stay_plain_language() {
        let oom =
            friendly_runner_exit_message("ggml_backend_alloc: failed to allocate buffer", FEDORA);
        assert!(
            oom.contains("enough free memory") && !oom.contains("ggml"),
            "{oom}"
        );

        let bad_file = friendly_runner_exit_message(
            "llama_model_load: failed to load model from x.gguf",
            FEDORA,
        );
        assert!(
            bad_file.contains("damaged") && !bad_file.contains("gguf"),
            "{bad_file}"
        );

        let generic = friendly_runner_exit_message("exit status 1", FEDORA);
        assert!(
            !generic.contains("exit status") && !generic.contains("llama-server"),
            "{generic}"
        );
    }

    #[test]
    fn detects_loopback_and_default_managed_port() {
        assert!(is_default_managed_loopback("http://127.0.0.1:18180/v1"));
        assert!(is_default_managed_loopback("http://localhost:18180/v1"));
        assert!(is_default_managed_loopback("http://[::1]:18180/v1"));
        assert!(!is_default_managed_loopback("http://127.0.0.1:11434/v1"));
        assert!(!is_default_managed_loopback("http://127.0.0.1:1234/v1"));
        assert!(!is_default_managed_loopback(
            "https://models.example.com:18180/v1"
        ));
        assert!(is_loopback_url("http://127.0.0.1:11434/v1"));
        assert!(!is_loopback_url("https://models.example.com/v1"));
    }

    #[tokio::test]
    async fn probe_sends_authorization_bearer_when_key_set() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = r#"{"data":[{"id":"remote-model"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            request
        });
        let ids =
            probe_openai_models_with_auth(&format!("http://{addr}/v1"), Some("sk-fixture-key"))
                .await
                .unwrap();
        assert_eq!(ids, vec!["remote-model".to_string()]);
        let request = server.await.unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer sk-fixture-key"),
            "missing auth header in: {request}"
        );
        assert!(
            request.starts_with("GET /v1/models"),
            "unexpected request: {request}"
        );
    }

    #[test]
    fn a_busy_port_is_swapped_for_a_reserved_one() {
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy = held.local_addr().unwrap().port();
        let reserved = reserve_port(busy).unwrap();
        let address = reserved.local_addr().unwrap();
        assert_ne!(address.port(), busy);
        assert!(address.ip().is_loopback());
        let client = std::net::TcpStream::connect(address).unwrap();
        assert_eq!(reserved.accept().unwrap().1, client.local_addr().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_working_program_passes_the_check() {
        check_runner_starts(Path::new("/usr/bin/true")).unwrap();
        assert!(check_runner_starts(Path::new("/usr/bin/false")).is_err());
    }
}
