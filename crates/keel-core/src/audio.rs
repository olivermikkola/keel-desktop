//! Speech-to-text and text-to-speech capability packs.
//!
//! Catalog entries ship as stubs. Users can mark a pack installed or register a
//! local runner binary/script from Settings. Checksummed Whisper weight downloads
//! and real TTS adapters land later. Remote text-provider auth is a separate
//! pillar (not wired here).
//!
//! ## Local runner contract (`adapter`: omitted / `"local"`)
//!
//! Pack directory under `<data_dir>/audio-packs/<pack_id>/`:
//! - `.installed` — install marker
//! - `runner.json` — `{ "command": "<path-or-name>", "args": ["…"] }`
//!
//! STT invokes: `command …args… <audio_path>` and reads the transcript from stdout.
//! TTS invokes: `command …args… --text <text> --output <path>`.
//!
//! ## Whisper.cpp runner contract (`adapter`: `"whisper_cpp"`)
//!
//! `runner.json`:
//! ```json
//! {
//!   "adapter": "whisper_cpp",
//!   "command": "whisper-cli",
//!   "args": [],
//!   "model": "/path/to/ggml-base.bin"
//! }
//! ```
//!
//! STT argv: `command …args… -m <model> -f <audio_path>`
//! Transcript is read from stdout (trim). No timestamps flags are injected; pass
//! extras via `args` (for example `["-nt"]`) if the binary needs them.
//!
//! Env (optional overrides):
//! - `KEEL_AUDIO_STT_CMD` — PATH command used when no `runner.json` exists (local adapter)
//! - `KEEL_WHISPER_MODEL` — ggml model path when `runner.json` omits `model`
//!
//! Weights are **not** bundled. Missing model paths fail with a clear error; the
//! catalog `download_url` for Whisper is an unpinned `keel-pack:unpinned/…`
//! placeholder until a checksummed artifact is pinned.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Capability ids exposed by the audio pack catalog.
pub const CAPABILITY_STT: &str = "stt";
pub const CAPABILITY_TTS: &str = "tts";

const RUNNER_FILE: &str = "runner.json";
const INSTALLED_MARKER: &str = ".installed";
const INSTALL_META: &str = "install.json";

/// Catalog placeholder until a checksummed Whisper artifact URL + SHA-256 are pinned.
///
/// Not fetchable. [`install_audio_pack_download_stub`] refuses it so Settings
/// cannot pretend a download succeeded.
pub const WHISPER_BASE_DOWNLOAD_PLACEHOLDER: &str = "keel-pack:unpinned/stt-whisper-base";

/// Prefix for honest "not pinned yet" catalog download placeholders.
pub const UNPINNED_PACK_URL_PREFIX: &str = "keel-pack:unpinned/";

/// Env override for the Whisper.cpp ggml model path when `runner.json` omits `model`.
pub const KEEL_WHISPER_MODEL_ENV: &str = "KEEL_WHISPER_MODEL";

/// Which audio capability a pack provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioCapability {
    Stt,
    Tts,
}

impl AudioCapability {
    pub fn id(self) -> &'static str {
        match self {
            Self::Stt => CAPABILITY_STT,
            Self::Tts => CAPABILITY_TTS,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Stt => "speech-to-text",
            Self::Tts => "text-to-speech",
        }
    }
}

/// Runner kind a pack expects once a real adapter exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioRunnerKind {
    AudioStt,
    AudioTts,
}

/// How [`run_stt`] interprets `runner.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioSttAdapterKind {
    /// Generic: `command …args… <audio_path>` → transcript on stdout.
    #[default]
    Local,
    /// Whisper.cpp CLI: `command …args… -m <model> -f <audio_path>` → stdout.
    WhisperCpp,
}

impl AudioSttAdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::WhisperCpp => "whisper_cpp",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "local" | "command" | "generic" => Ok(Self::Local),
            "whisper_cpp" | "whisper-cpp" | "whisper" => Ok(Self::WhisperCpp),
            other => bail!("Unknown STT adapter '{other}'. Use 'local' or 'whisper_cpp'."),
        }
    }
}

/// Pack-manifest shape for optional STT/TTS downloads.
///
/// Whisper's catalog `download_url` is an unpinned placeholder until a
/// checksummed artifact is pinned. Users can still register a local / Whisper.cpp
/// runner without downloading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioPackManifest {
    pub id: String,
    pub capability: AudioCapability,
    pub display_name: String,
    pub min_vram_mib: u64,
    pub runner_kind: AudioRunnerKind,
    pub download_url: Option<String>,
    pub installed: bool,
    /// Absolute or PATH-resolvable command registered for this pack, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner_path: Option<String>,
}

/// On-disk runner configuration for an installed pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRunnerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// STT adapter; omitted / default is [`AudioSttAdapterKind::Local`].
    #[serde(default, skip_serializing_if = "adapter_is_local")]
    pub adapter: AudioSttAdapterKind,
    /// ggml model path for [`AudioSttAdapterKind::WhisperCpp`] (or set [`KEEL_WHISPER_MODEL_ENV`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

fn adapter_is_local(kind: &AudioSttAdapterKind) -> bool {
    matches!(kind, AudioSttAdapterKind::Local)
}

/// True when `url` is a catalog placeholder, not a fetchable artifact.
pub fn is_unpinned_audio_download_url(url: &str) -> bool {
    let url = url.trim();
    url.starts_with(UNPINNED_PACK_URL_PREFIX) || url.contains("#unpinned")
}

/// True when `url` can be recorded as a download stub (http(s) or existing path),
/// and is not an unpinned placeholder.
pub fn is_recordable_audio_download_url(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() || is_unpinned_audio_download_url(url) {
        return false;
    }
    url.starts_with("https://") || url.starts_with("http://") || Path::new(url).exists()
}

/// How a pack was (or will be) installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum AudioInstallMeta {
    /// User marked the pack installed without a runner yet.
    Marked,
    /// Local binary/script path registered as the pack runner.
    LocalRunner { command: String },
    /// Download URL recorded; bytes are not fetched yet (stub).
    DownloadStub { url: String },
}

/// Stub catalog. Whisper lists an unpinned download placeholder (not fetchable).
pub fn list_audio_packs() -> Vec<AudioPackManifest> {
    vec![
        AudioPackManifest {
            id: "stt-whisper-base".into(),
            capability: AudioCapability::Stt,
            display_name: "Speech to text · Whisper base (stub)".into(),
            min_vram_mib: 1024,
            runner_kind: AudioRunnerKind::AudioStt,
            download_url: Some(WHISPER_BASE_DOWNLOAD_PLACEHOLDER.into()),
            installed: false,
            runner_path: None,
        },
        AudioPackManifest {
            id: "tts-local-en".into(),
            capability: AudioCapability::Tts,
            display_name: "Text to speech · English (stub)".into(),
            min_vram_mib: 512,
            runner_kind: AudioRunnerKind::AudioTts,
            download_url: None,
            installed: false,
            runner_path: None,
        },
    ]
}

pub fn find_audio_pack(pack_id: &str) -> Option<AudioPackManifest> {
    list_audio_packs()
        .into_iter()
        .find(|pack| pack.id == pack_id)
}

pub fn audio_packs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("audio-packs")
}

fn pack_dir(data_dir: &Path, pack_id: &str) -> PathBuf {
    audio_packs_dir(data_dir).join(pack_id)
}

/// True only when an install marker exists under the data directory.
pub fn is_audio_pack_installed(data_dir: &Path, pack_id: &str) -> bool {
    pack_dir(data_dir, pack_id).join(INSTALLED_MARKER).is_file()
}

/// Read the registered runner command for a pack, if any.
pub fn read_audio_runner(data_dir: &Path, pack_id: &str) -> Option<AudioRunnerConfig> {
    let path = pack_dir(data_dir, pack_id).join(RUNNER_FILE);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Overlay install + runner state onto a catalog entry.
pub fn resolve_audio_pack(data_dir: &Path, pack: AudioPackManifest) -> AudioPackManifest {
    let installed = is_audio_pack_installed(data_dir, &pack.id);
    let runner_path = read_audio_runner(data_dir, &pack.id).map(|r| r.command);
    AudioPackManifest {
        installed,
        runner_path,
        ..pack
    }
}

/// Catalog packs with on-disk install/runner overlays.
pub fn list_audio_packs_resolved(data_dir: &Path) -> Vec<AudioPackManifest> {
    list_audio_packs()
        .into_iter()
        .map(|pack| resolve_audio_pack(data_dir, pack))
        .collect()
}

/// Resolve a catalog pack that is actually installed for the given capability.
pub fn require_installed_audio_pack(
    data_dir: &Path,
    capability: AudioCapability,
) -> Result<AudioPackManifest> {
    let candidates: Vec<_> = list_audio_packs()
        .into_iter()
        .filter(|pack| pack.capability == capability)
        .collect();
    if candidates.is_empty() {
        bail!(
            "No {} pack is listed in the catalog yet.",
            capability.label()
        );
    }
    for pack in candidates {
        if is_audio_pack_installed(data_dir, &pack.id) {
            return Ok(resolve_audio_pack(data_dir, pack));
        }
    }
    bail!(
        "No {} pack is installed. Download or register an audio pack from Settings before using {}.",
        capability.label(),
        capability.label()
    );
}

fn write_install_meta(dir: &Path, meta: &AudioInstallMeta) -> Result<()> {
    let raw = serde_json::to_string_pretty(meta).context("serialize install meta")?;
    fs::write(dir.join(INSTALL_META), raw).context("write install meta")?;
    Ok(())
}

fn ensure_catalog_pack(pack_id: &str) -> Result<AudioPackManifest> {
    find_audio_pack(pack_id).ok_or_else(|| anyhow::anyhow!("Unknown audio pack id '{pack_id}'."))
}

/// Mark a catalog pack installed without registering a runner yet.
pub fn mark_audio_pack_installed(data_dir: &Path, pack_id: &str) -> Result<AudioPackManifest> {
    let pack = ensure_catalog_pack(pack_id)?;
    let dir = pack_dir(data_dir, pack_id);
    fs::create_dir_all(&dir).with_context(|| format!("create pack dir {}", dir.display()))?;
    fs::write(dir.join(INSTALLED_MARKER), b"marked\n").context("write install marker")?;
    write_install_meta(&dir, &AudioInstallMeta::Marked)?;
    Ok(resolve_audio_pack(data_dir, pack))
}

/// Register a local binary/script as the pack runner and mark the pack installed.
///
/// `command` may be an absolute path or a PATH-resolvable name. Absolute paths
/// must exist; bare names are accepted (resolved at run time).
pub fn register_audio_pack_runner(
    data_dir: &Path,
    pack_id: &str,
    command: &str,
    args: &[String],
) -> Result<AudioPackManifest> {
    let pack = ensure_catalog_pack(pack_id)?;
    let command = command.trim();
    if command.is_empty() {
        bail!("Provide a local binary or script path for the audio pack runner.");
    }
    let cmd_path = Path::new(command);
    // On Windows, Unix-style "/foo" is not `is_absolute()`. Treat any path with a
    // separator as a concrete filesystem path that must exist before register.
    let looks_like_path = cmd_path.is_absolute() || command.contains('/') || command.contains('\\');
    if looks_like_path && !cmd_path.exists() {
        bail!(
            "Audio pack runner '{}' does not exist. Pick a binary or script on this machine.",
            command
        );
    }

    write_runner_and_mark(
        data_dir,
        pack_id,
        &pack,
        AudioRunnerConfig {
            command: command.to_string(),
            args: args.to_vec(),
            adapter: AudioSttAdapterKind::Local,
            model: None,
        },
        "local_runner\n",
        AudioInstallMeta::LocalRunner {
            command: command.to_string(),
        },
    )
}

/// Register a Whisper.cpp-oriented runner (`adapter: whisper_cpp`) and mark installed.
///
/// `command` is the whisper.cpp binary (absolute or PATH name). `model` must be an
/// existing ggml weights file — Desktop does not bundle or download weights yet.
pub fn register_whisper_cpp_runner(
    data_dir: &Path,
    pack_id: &str,
    command: &str,
    model: &str,
    args: &[String],
) -> Result<AudioPackManifest> {
    let pack = ensure_catalog_pack(pack_id)?;
    if pack.capability != AudioCapability::Stt {
        bail!(
            "Whisper.cpp runner is only valid for speech-to-text packs; '{}' is {}.",
            pack_id,
            pack.capability.label()
        );
    }
    let command = command.trim();
    if command.is_empty() {
        bail!("Provide a whisper.cpp binary path or PATH name (for example whisper-cli).");
    }
    let model = model.trim();
    if model.is_empty() {
        bail!(
            "Provide a ggml model path for the Whisper.cpp adapter (or set {KEEL_WHISPER_MODEL_ENV} later). No Whisper weights are bundled yet."
        );
    }
    let model_path = Path::new(model);
    if !model_path.is_file() {
        bail!(
            "Whisper.cpp model '{}' does not exist. Pin checksummed weights before registering, or point at a ggml file already on this machine.",
            model
        );
    }
    let cmd_path = Path::new(command);
    let looks_like_path = cmd_path.is_absolute() || command.contains('/') || command.contains('\\');
    if looks_like_path && !cmd_path.exists() {
        bail!(
            "Whisper.cpp binary '{}' does not exist. Pick a binary on this machine.",
            command
        );
    }

    write_runner_and_mark(
        data_dir,
        pack_id,
        &pack,
        AudioRunnerConfig {
            command: command.to_string(),
            args: args.to_vec(),
            adapter: AudioSttAdapterKind::WhisperCpp,
            model: Some(model.to_string()),
        },
        "whisper_cpp_runner\n",
        AudioInstallMeta::LocalRunner {
            command: command.to_string(),
        },
    )
}

fn write_runner_and_mark(
    data_dir: &Path,
    pack_id: &str,
    pack: &AudioPackManifest,
    config: AudioRunnerConfig,
    marker_body: &str,
    meta: AudioInstallMeta,
) -> Result<AudioPackManifest> {
    let dir = pack_dir(data_dir, pack_id);
    fs::create_dir_all(&dir).with_context(|| format!("create pack dir {}", dir.display()))?;
    let raw = serde_json::to_string_pretty(&config).context("serialize runner config")?;
    fs::write(dir.join(RUNNER_FILE), raw).context("write runner.json")?;
    fs::write(dir.join(INSTALLED_MARKER), marker_body.as_bytes())
        .context("write install marker")?;
    write_install_meta(&dir, &meta)?;
    Ok(resolve_audio_pack(data_dir, pack.clone()))
}

/// Record a download-URL stub install. Does **not** fetch bytes yet.
///
/// Uses the catalog `download_url` when `url` is empty. Unpinned catalog
/// placeholders (`keel-pack:unpinned/…`) are refused — register a local /
/// Whisper.cpp runner, or pass a real http(s) URL / existing archive path.
/// Marks the pack installed so Settings shows Installed; `run_stt` / `run_tts`
/// still need a registered runner (clear error until then).
pub fn install_audio_pack_download_stub(
    data_dir: &Path,
    pack_id: &str,
    url: Option<&str>,
) -> Result<AudioPackManifest> {
    let pack = ensure_catalog_pack(pack_id)?;
    let resolved = url
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| pack.download_url.clone());
    let Some(resolved) = resolved else {
        bail!(
            "Pack '{}' has no download URL yet. Register a local runner path instead, or provide a URL.",
            pack_id
        );
    };
    if is_unpinned_audio_download_url(&resolved) {
        bail!(
            "Pack '{}' download is not pinned yet (no checksummed artifact URL). Register a local or Whisper.cpp runner, or provide a real http(s) URL / archive path.",
            pack_id
        );
    }
    if !is_recordable_audio_download_url(&resolved) {
        bail!(
            "Download stub needs an http(s) URL or an existing local archive path; got '{resolved}'."
        );
    }

    let dir = pack_dir(data_dir, pack_id);
    fs::create_dir_all(&dir).with_context(|| format!("create pack dir {}", dir.display()))?;
    fs::write(dir.join(INSTALLED_MARKER), b"download_stub\n").context("write install marker")?;
    write_install_meta(
        &dir,
        &AudioInstallMeta::DownloadStub {
            url: resolved.clone(),
        },
    )?;
    // Keep any previously registered runner; download stub alone is not enough to run.
    Ok(resolve_audio_pack(data_dir, pack))
}

impl AudioPackManifest {
    /// Convert to the shared pack-scheduling manifest used by [`crate::runtime::plan_pack_vram`].
    pub fn to_pack_manifest(&self) -> crate::packs::PackManifest {
        let kind = match self.runner_kind {
            AudioRunnerKind::AudioStt => crate::packs::RunnerKind::AudioStt,
            AudioRunnerKind::AudioTts => crate::packs::RunnerKind::AudioTts,
        };
        crate::packs::PackManifest::new(self.id.clone(), self.min_vram_mib, kind)
    }
}

fn require_runner(data_dir: &Path, pack: &AudioPackManifest) -> Result<AudioRunnerConfig> {
    if let Some(config) = read_audio_runner(data_dir, &pack.id) {
        return Ok(config);
    }
    // Env overrides for local testing without going through Settings.
    let env_key = match pack.capability {
        AudioCapability::Stt => "KEEL_AUDIO_STT_CMD",
        AudioCapability::Tts => "KEEL_AUDIO_TTS_CMD",
    };
    if let Ok(command) = std::env::var(env_key) {
        let command = command.trim().to_string();
        if !command.is_empty() {
            return Ok(AudioRunnerConfig {
                command,
                args: Vec::new(),
                adapter: AudioSttAdapterKind::Local,
                model: None,
            });
        }
    }
    bail!(
        "{} pack '{}' is marked installed, but no local runner is registered. In Settings, register a binary/script path for this pack (or set {env_key}).",
        match pack.capability {
            AudioCapability::Stt => "Speech-to-text",
            AudioCapability::Tts => "Text-to-speech",
        },
        pack.id
    );
}

fn run_local_command(config: &AudioRunnerConfig, extra_args: &[String]) -> Result<String> {
    let mut cmd = Command::new(&config.command);
    cmd.args(&config.args).args(extra_args);
    let output = cmd
        .output()
        .with_context(|| format!("failed to start audio runner '{}'", config.command))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        bail!(
            "Audio runner '{}' exited with {}. stderr: {} stdout: {}",
            config.command,
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into()),
            stderr.trim(),
            stdout.trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Resolve the ggml model path for a Whisper.cpp runner.
pub fn resolve_whisper_model(config: &AudioRunnerConfig) -> Result<String> {
    if let Some(model) = config
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(model.to_string());
    }
    if let Ok(from_env) = std::env::var(KEEL_WHISPER_MODEL_ENV) {
        let from_env = from_env.trim().to_string();
        if !from_env.is_empty() {
            return Ok(from_env);
        }
    }
    bail!(
        "Whisper.cpp adapter needs a ggml model path in runner.json (\"model\") or {KEEL_WHISPER_MODEL_ENV}. No Whisper weights are bundled yet."
    );
}

/// Build whisper.cpp CLI args after `command` (does not include the binary name).
///
/// Contract: `…args… -m <model> -f <audio_path>`.
pub fn whisper_cpp_argv(config: &AudioRunnerConfig, audio_path: &Path, model: &str) -> Vec<String> {
    let mut argv = config.args.clone();
    argv.push("-m".into());
    argv.push(model.to_string());
    argv.push("-f".into());
    argv.push(audio_path.to_string_lossy().into_owned());
    argv
}

/// STT adapter invoked by [`run_stt`] once a runner is resolved.
pub trait SttAdapter {
    fn transcribe(&self, audio_path: &Path) -> Result<String>;
}

/// Default adapter: `command …args… <audio_path>` → stdout transcript.
pub struct LocalCommandSttAdapter {
    pub config: AudioRunnerConfig,
}

impl SttAdapter for LocalCommandSttAdapter {
    fn transcribe(&self, audio_path: &Path) -> Result<String> {
        run_local_command(&self.config, &[audio_path.to_string_lossy().into_owned()])
    }
}

/// Whisper.cpp-oriented adapter: `command …args… -m <model> -f <audio_path>`.
///
/// Fails honestly when the model path is missing or not a file. Does not download weights.
pub struct WhisperCppSttAdapter {
    pub config: AudioRunnerConfig,
}

impl SttAdapter for WhisperCppSttAdapter {
    fn transcribe(&self, audio_path: &Path) -> Result<String> {
        let model = resolve_whisper_model(&self.config)?;
        let model_path = Path::new(&model);
        if !model_path.is_file() {
            bail!(
                "Whisper.cpp model '{}' is not a file. Place ggml weights on disk (checksummed download not pinned yet) or update runner.json / {KEEL_WHISPER_MODEL_ENV}.",
                model
            );
        }
        let argv = whisper_cpp_argv(&self.config, audio_path, &model);
        // Pass full argv via a temporary config so run_local_command does not
        // double-apply config.args (already folded into argv).
        let launch = AudioRunnerConfig {
            command: self.config.command.clone(),
            args: argv,
            adapter: AudioSttAdapterKind::WhisperCpp,
            model: Some(model),
        };
        run_local_command(&launch, &[])
    }
}

/// Pick an [`SttAdapter`] implementation from `runner.json`.
pub fn stt_adapter_from_runner(config: AudioRunnerConfig) -> Box<dyn SttAdapter> {
    match config.adapter {
        AudioSttAdapterKind::Local => Box::new(LocalCommandSttAdapter { config }),
        AudioSttAdapterKind::WhisperCpp => Box::new(WhisperCppSttAdapter { config }),
    }
}

/// Speech-to-text entry point.
///
/// Requires an installed pack and a registered (or env-overridden) runner.
/// Dispatches to [`LocalCommandSttAdapter`] or [`WhisperCppSttAdapter`] from
/// `runner.json`'s `adapter` field.
pub fn run_stt(data_dir: &Path, audio_path: &Path) -> Result<String> {
    let pack = require_installed_audio_pack(data_dir, AudioCapability::Stt)?;
    // VRAM prepare / keep-alive unload runs in the Desktop shell via
    // `prepare_pack_vram` before this entry is called.
    if !audio_path.exists() {
        bail!("Audio file not found: {}", audio_path.display());
    }
    let runner = require_runner(data_dir, &pack)?;
    let command = runner.command.clone();
    let transcript = stt_adapter_from_runner(runner).transcribe(audio_path)?;
    if transcript.is_empty() {
        bail!(
            "Speech-to-text runner '{}' produced no transcript for {}.",
            command,
            audio_path.display()
        );
    }
    Ok(transcript)
}

/// Text-to-speech entry point.
///
/// Requires an installed pack and a registered (or env-overridden) local runner.
pub fn run_tts(data_dir: &Path, text: &str, output_path: &Path) -> Result<()> {
    let pack = require_installed_audio_pack(data_dir, AudioCapability::Tts)?;
    // VRAM prepare / keep-alive unload runs in the Desktop shell via
    // `prepare_pack_vram` before this entry is called.
    let text = text.trim();
    if text.is_empty() {
        bail!("Provide text to speak.");
    }
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create TTS output dir {}", parent.display()))?;
        }
    }
    let runner = require_runner(data_dir, &pack)?;
    let _ = run_local_command(
        &runner,
        &[
            "--text".into(),
            text.to_string(),
            "--output".into(),
            output_path.to_string_lossy().into_owned(),
        ],
    )?;
    if !output_path.exists() {
        bail!(
            "Text-to-speech runner '{}' finished but did not create {}.",
            runner.command,
            output_path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_stt_and_tts_stubs_not_downloaded() {
        let packs = list_audio_packs();
        assert_eq!(packs.len(), 2);
        let stt = packs
            .iter()
            .find(|p| p.capability == AudioCapability::Stt)
            .expect("stt stub");
        let tts = packs
            .iter()
            .find(|p| p.capability == AudioCapability::Tts)
            .expect("tts stub");
        assert_eq!(stt.capability.id(), CAPABILITY_STT);
        assert_eq!(tts.capability.id(), CAPABILITY_TTS);
        assert!(!stt.installed);
        assert!(!tts.installed);
        assert_eq!(
            stt.download_url.as_deref(),
            Some(WHISPER_BASE_DOWNLOAD_PLACEHOLDER)
        );
        assert!(is_unpinned_audio_download_url(
            stt.download_url.as_deref().unwrap()
        ));
        assert!(!is_recordable_audio_download_url(
            stt.download_url.as_deref().unwrap()
        ));
        assert!(tts.download_url.is_none());
        assert_eq!(stt.runner_kind, AudioRunnerKind::AudioStt);
        assert_eq!(tts.runner_kind, AudioRunnerKind::AudioTts);
        assert!(stt.min_vram_mib > 0);
        assert!(tts.min_vram_mib > 0);
    }

    #[test]
    fn find_audio_pack_resolves_stub_ids() {
        assert_eq!(
            find_audio_pack("stt-whisper-base").map(|p| p.id),
            Some("stt-whisper-base".into())
        );
        assert!(find_audio_pack("missing").is_none());
    }

    #[test]
    fn run_stt_errors_when_no_pack_is_installed() {
        let dir = tempfile_dir("stt-missing");
        let err = run_stt(&dir, Path::new("clip.wav"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("No speech-to-text pack is installed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn run_tts_errors_when_no_pack_is_installed() {
        let dir = tempfile_dir("tts-missing");
        let err = run_tts(&dir, "hello", Path::new("out.wav"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("No text-to-speech pack is installed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn require_installed_succeeds_only_with_marker_file() {
        let dir = tempfile_dir("stt-marker");
        let err = require_installed_audio_pack(&dir, AudioCapability::Stt)
            .unwrap_err()
            .to_string();
        assert!(err.contains("No speech-to-text pack is installed"));

        let pack = mark_audio_pack_installed(&dir, "stt-whisper-base").unwrap();
        assert_eq!(pack.id, "stt-whisper-base");
        assert!(pack.installed);
    }

    #[test]
    fn installed_without_runner_gives_clear_error() {
        let dir = tempfile_dir("stt-no-runner");
        mark_audio_pack_installed(&dir, "stt-whisper-base").unwrap();
        let wav = dir.join("clip.wav");
        fs::write(&wav, b"RIFF").unwrap();
        let err = run_stt(&dir, &wav).unwrap_err().to_string();
        assert!(
            err.contains("no local runner is registered"),
            "unexpected: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn register_local_runner_and_run_stt() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile_dir("stt-runner");
        let script = dir.join("fake-stt.sh");
        fs::write(&script, "#!/bin/sh\nprintf 'hello from stt\\n'\n").unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let pack =
            register_audio_pack_runner(&dir, "stt-whisper-base", script.to_str().unwrap(), &[])
                .unwrap();
        assert!(pack.installed);
        assert_eq!(pack.runner_path.as_deref(), script.to_str());

        let wav = dir.join("clip.wav");
        fs::write(&wav, b"RIFF").unwrap();
        let text = run_stt(&dir, &wav).unwrap();
        assert_eq!(text, "hello from stt");
    }

    #[cfg(unix)]
    #[test]
    fn register_local_runner_and_run_tts() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile_dir("tts-runner");
        let script = dir.join("fake-tts.sh");
        // Write output file for --output path (args: --text X --output Y).
        fs::write(
            &script,
            "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"--output\" ]; then out=\"$2\"; shift 2; continue; fi\n  shift\ndone\n: \"${out:?}\"\nprintf 'wav' > \"$out\"\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        register_audio_pack_runner(&dir, "tts-local-en", script.to_str().unwrap(), &[]).unwrap();
        let out = dir.join("out.wav");
        run_tts(&dir, "hello world", &out).unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "wav");
    }

    #[test]
    fn register_rejects_missing_absolute_runner() {
        let dir = tempfile_dir("missing-bin");
        let err = register_audio_pack_runner(
            &dir,
            "stt-whisper-base",
            "/definitely/missing/keel-audio-runner-xyz",
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("does not exist"), "unexpected: {err}");
    }

    #[test]
    fn download_stub_refuses_unpinned_catalog_placeholder() {
        let dir = tempfile_dir("dl-stub");
        let err = install_audio_pack_download_stub(&dir, "stt-whisper-base", None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not pinned yet") && err.contains("checksummed"),
            "unexpected: {err}"
        );

        let err = install_audio_pack_download_stub(
            &dir,
            "stt-whisper-base",
            Some(WHISPER_BASE_DOWNLOAD_PLACEHOLDER),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("not pinned yet"), "unexpected: {err}");
    }

    #[test]
    fn download_stub_accepts_real_url_and_marks_installed() {
        let dir = tempfile_dir("dl-stub-url");
        let pack = install_audio_pack_download_stub(
            &dir,
            "stt-whisper-base",
            Some("https://example.invalid/whisper-base.tar.gz"),
        )
        .unwrap();
        assert!(pack.installed);
        assert!(pack.runner_path.is_none());
        let meta =
            fs::read_to_string(pack_dir(&dir, "stt-whisper-base").join(INSTALL_META)).unwrap();
        assert!(meta.contains("download_stub"));
    }

    #[test]
    fn whisper_cpp_argv_follows_documented_contract() {
        let config = AudioRunnerConfig {
            command: "whisper-cli".into(),
            args: vec!["-nt".into()],
            adapter: AudioSttAdapterKind::WhisperCpp,
            model: Some("/models/ggml-base.bin".into()),
        };
        let argv = whisper_cpp_argv(&config, Path::new("/tmp/clip.wav"), "/models/ggml-base.bin");
        assert_eq!(
            argv,
            vec![
                "-nt".to_string(),
                "-m".into(),
                "/models/ggml-base.bin".into(),
                "-f".into(),
                "/tmp/clip.wav".into(),
            ]
        );
    }

    #[test]
    fn whisper_cpp_adapter_errors_when_model_file_absent() {
        let dir = tempfile_dir("whisper-no-model");
        let wav = dir.join("clip.wav");
        fs::write(&wav, b"RIFF").unwrap();
        let missing = dir.join("missing-ggml.bin");
        let config = AudioRunnerConfig {
            command: "whisper-cli".into(),
            args: Vec::new(),
            adapter: AudioSttAdapterKind::WhisperCpp,
            model: Some(missing.to_string_lossy().into_owned()),
        };
        let err = WhisperCppSttAdapter { config }
            .transcribe(&wav)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not a file") && err.contains("checksummed download not pinned"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn resolve_whisper_model_prefers_runner_json() {
        let config = AudioRunnerConfig {
            command: "whisper-cli".into(),
            args: Vec::new(),
            adapter: AudioSttAdapterKind::WhisperCpp,
            model: Some("/models/from-runner.bin".into()),
        };
        assert_eq!(
            resolve_whisper_model(&config).unwrap(),
            "/models/from-runner.bin"
        );
    }

    #[test]
    fn register_whisper_cpp_rejects_missing_model_file() {
        let dir = tempfile_dir("whisper-reg");
        let err = register_whisper_cpp_runner(
            &dir,
            "stt-whisper-base",
            "whisper-cli",
            dir.join("no-such-ggml.bin").to_str().unwrap(),
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("does not exist"), "unexpected: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn register_whisper_cpp_runner_and_run_stt() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile_dir("whisper-runner");
        let model = dir.join("ggml-base.bin");
        fs::write(&model, b"not-real-weights").unwrap();
        let script = dir.join("fake-whisper.sh");
        // Echo argv so the test can prove -m / -f contract, then print transcript.
        fs::write(
            &script,
            "#!/bin/sh\n# fake whisper-cli: require -m and -f then print transcript\nmodel=\"\"\nfile=\"\"\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"-m\" ]; then model=\"$2\"; shift 2; continue; fi\n  if [ \"$1\" = \"-f\" ]; then file=\"$2\"; shift 2; continue; fi\n  shift\ndone\n[ -n \"$model\" ] && [ -n \"$file\" ] || exit 2\nprintf 'whisper ok\\n'\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let pack = register_whisper_cpp_runner(
            &dir,
            "stt-whisper-base",
            script.to_str().unwrap(),
            model.to_str().unwrap(),
            &[],
        )
        .unwrap();
        assert!(pack.installed);
        let raw = fs::read_to_string(pack_dir(&dir, "stt-whisper-base").join(RUNNER_FILE)).unwrap();
        assert!(raw.contains("whisper_cpp"));
        assert!(raw.contains("ggml-base.bin"));

        let wav = dir.join("clip.wav");
        fs::write(&wav, b"RIFF").unwrap();
        let text = run_stt(&dir, &wav).unwrap();
        assert_eq!(text, "whisper ok");
    }

    #[test]
    fn runner_json_without_adapter_defaults_to_local() {
        let raw = r#"{"command":"stt.sh","args":[]}"#;
        let config: AudioRunnerConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(config.adapter, AudioSttAdapterKind::Local);
        assert!(config.model.is_none());
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

    fn sample_graphics_loaded() -> crate::runtime::LoadedLaunch {
        crate::runtime::LoadedLaunch {
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

    #[test]
    fn audio_pack_maps_to_shared_pack_manifest_for_vram_plan() {
        let stt = find_audio_pack("stt-whisper-base").expect("stt stub");
        let manifest = stt.to_pack_manifest();
        assert_eq!(manifest.capability_id, stt.id);
        assert_eq!(manifest.min_vram_mib, stt.min_vram_mib);
        assert_eq!(manifest.runner_kind, crate::packs::RunnerKind::AudioStt);

        let plan = crate::runtime::plan_pack_vram(&sample_inventory(400), &manifest, None);
        assert_eq!(
            plan.status,
            crate::runtime::PackVramStatus::ProcessorFallback
        );
        assert!(!plan.stop_keep_alive);
    }

    #[test]
    fn audio_pack_vram_plan_unloads_keep_alive_when_free_vram_is_short() {
        let pack = find_audio_pack("stt-whisper-base")
            .expect("stt stub")
            .to_pack_manifest();
        // STT stub needs 1024 MiB; 256 free with keep-alive on graphics → unload.
        let plan = crate::runtime::plan_pack_vram(
            &sample_inventory(256),
            &pack,
            Some(&sample_graphics_loaded()),
        );
        assert_eq!(plan.status, crate::runtime::PackVramStatus::Unloaded);
        assert!(plan.stop_keep_alive);
    }

    fn tempfile_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("keel-audio-{}-{}", label, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
