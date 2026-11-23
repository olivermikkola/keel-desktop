//! Speech-to-text and text-to-speech capability packs.
//!
//! Catalog entries ship as stubs. Users can mark a pack installed or register a
//! local runner binary/script from Settings. Real Whisper/TTS adapters and
//! resumable downloads land later. Remote text-provider auth is a separate
//! pillar (not wired here).
//!
//! ## Local runner contract
//!
//! Pack directory under `<data_dir>/audio-packs/<pack_id>/`:
//! - `.installed` — install marker
//! - `runner.json` — `{ "command": "<path-or-name>", "args": ["…"] }`
//!
//! STT invokes: `command …args… <audio_path>` and reads the transcript from stdout.
//! TTS invokes: `command …args… --text <text> --output <path>`.

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

/// Pack-manifest shape for optional STT/TTS downloads.
///
/// Catalog stubs leave `download_url` / `runner_path` empty until the user
/// registers a local binary or a future download pins a URL.
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

/// Stub catalog. Entries are documented as not downloaded yet.
pub fn list_audio_packs() -> Vec<AudioPackManifest> {
    vec![
        AudioPackManifest {
            id: "stt-whisper-base".into(),
            capability: AudioCapability::Stt,
            display_name: "Speech to text · Whisper base (stub)".into(),
            min_vram_mib: 1024,
            runner_kind: AudioRunnerKind::AudioStt,
            download_url: None,
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

    let dir = pack_dir(data_dir, pack_id);
    fs::create_dir_all(&dir).with_context(|| format!("create pack dir {}", dir.display()))?;
    let config = AudioRunnerConfig {
        command: command.to_string(),
        args: args.to_vec(),
    };
    let raw = serde_json::to_string_pretty(&config).context("serialize runner config")?;
    fs::write(dir.join(RUNNER_FILE), raw).context("write runner.json")?;
    fs::write(dir.join(INSTALLED_MARKER), b"local_runner\n").context("write install marker")?;
    write_install_meta(
        &dir,
        &AudioInstallMeta::LocalRunner {
            command: command.to_string(),
        },
    )?;
    Ok(resolve_audio_pack(data_dir, pack))
}

/// Record a download-URL stub install. Does **not** fetch bytes yet.
///
/// Uses the catalog `download_url` when `url` is empty. Marks the pack installed
/// so Settings shows Installed; `run_stt` / `run_tts` still need a registered
/// runner (clear error until then).
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
    if !(resolved.starts_with("https://")
        || resolved.starts_with("http://")
        || Path::new(&resolved).exists())
    {
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

/// Speech-to-text entry point.
///
/// Requires an installed pack and a registered (or env-overridden) local runner.
pub fn run_stt(data_dir: &Path, audio_path: &Path) -> Result<String> {
    let pack = require_installed_audio_pack(data_dir, AudioCapability::Stt)?;
    // VRAM prepare / keep-alive unload runs in the Desktop shell via
    // `prepare_pack_vram` before this entry is called.
    if !audio_path.exists() {
        bail!("Audio file not found: {}", audio_path.display());
    }
    let runner = require_runner(data_dir, &pack)?;
    let transcript = run_local_command(&runner, &[audio_path.to_string_lossy().into_owned()])?;
    if transcript.is_empty() {
        bail!(
            "Speech-to-text runner '{}' produced no transcript for {}.",
            runner.command,
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
        assert!(stt.download_url.is_none());
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
    fn download_stub_requires_url_and_marks_installed() {
        let dir = tempfile_dir("dl-stub");
        let err = install_audio_pack_download_stub(&dir, "stt-whisper-base", None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("no download URL"), "unexpected: {err}");

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
