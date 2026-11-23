//! Speech-to-text and text-to-speech capability packs.
//!
//! Catalog entries ship as stubs. Users can mark a pack installed, register a
//! local / Whisper.cpp / Piper runner, or download a **checksum-pinned** artifact
//! when the catalog lists `download_url` + `sha256`. Whisper base stays on an
//! unpinned placeholder until a real ggml URL is pinned. Remote text-provider
//! auth is a separate pillar (not wired here).
//!
//! ## Local runner contract (`adapter`: omitted / `"local"`)
//!
//! Pack directory under `<data_dir>/audio-packs/<pack_id>/`:
//! - `.installed` — install marker
//! - `runner.json` — `{ "command": "<path-or-name>", "args": ["…"] }`
//!
//! STT invokes: `command …args… <audio_path>` and reads the transcript from stdout.
//! TTS invokes: `command …args… --text <text> --output <path>` ([`AudioTtsAdapterKind::Local`]).
//!
//! ## Whisper.cpp runner contract (`adapter`: `"whisper_cpp"`)
//!
//! STT only. `runner.json`:
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
//! ## Piper TTS runner contract (`adapter`: `"piper"`)
//!
//! TTS only. Official [Piper](https://github.com/rhasspy/piper) CLI shape:
//! ```json
//! {
//!   "adapter": "piper",
//!   "command": "piper",
//!   "args": [],
//!   "model": "/path/to/voice.onnx"
//! }
//! ```
//!
//! TTS argv: `command …args… --model <model> -f <path>` with synthesis
//! text on **stdin** (Piper's documented contract). Piper also expects
//! `<model>.onnx.json` beside the ONNX file; the checksummed
//! [`PIPER_EN_KUSAL_Q8_PACK_ID`] download fetches that sidecar after the weights.
//! A `whisper_cpp` adapter on a TTS pack (or `piper` on STT) is rejected with a
//! clear error. Local argv contract remains
//! `command …args… --text <text> --output <path>`.
//!
//! Env (optional overrides):
//! - `KEEL_AUDIO_STT_CMD` — PATH command used when no `runner.json` exists (local adapter)
//! - `KEEL_AUDIO_TTS_CMD` — PATH command used when no `runner.json` exists (local TTS)
//! - `KEEL_WHISPER_MODEL` — ggml model path when `runner.json` omits `model`
//! - `KEEL_PIPER_MODEL` — Piper ONNX path when `runner.json` omits `model`
//!
//! Weights are **not** bundled. Missing model paths fail with a clear error; the
//! catalog `download_url` for Whisper base is an unpinned `keel-pack:unpinned/…`
//! placeholder until a checksummed artifact is pinned.
//!
//! ## Audio artifacts
//!
//! Successful [`run_stt`] / [`run_tts`] calls also persist a thin local record
//! under `<data_dir>/audio-artifacts/`:
//! - STT → `{id}.txt` transcript + `{id}.meta.json`
//! - TTS → copy of the output wav as `{id}.wav` + `{id}.meta.json`
//!
//! [`list_audio_artifacts`] reads those meta files. No cloud upload, no fake
//! players — just paths on disk for Settings to show.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// Capability ids exposed by the audio pack catalog.
pub const CAPABILITY_STT: &str = "stt";
pub const CAPABILITY_TTS: &str = "tts";

const RUNNER_FILE: &str = "runner.json";
const INSTALLED_MARKER: &str = ".installed";
const INSTALL_META: &str = "install.json";

/// Catalog placeholder for a larger Whisper base pin (not fetchable yet).
///
/// [`install_audio_pack_download_stub`] and [`download_audio_pack`] refuse it.
pub const WHISPER_BASE_DOWNLOAD_PLACEHOLDER: &str = "keel-pack:unpinned/stt-whisper-base";

/// Prefix for honest "not pinned yet" catalog download placeholders.
pub const UNPINNED_PACK_URL_PREFIX: &str = "keel-pack:unpinned/";

/// Checksum-pinned Whisper.cpp tiny q5_1 weights (Hugging Face `ggerganov/whisper.cpp`).
pub const WHISPER_TINY_Q5_PACK_ID: &str = "stt-whisper-tiny-q5";
pub const WHISPER_TINY_Q5_FILE: &str = "ggml-tiny-q5_1.bin";
pub const WHISPER_TINY_Q5_SHA256: &str =
    "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7";
pub const WHISPER_TINY_Q5_BYTES: u64 = 32_152_673;
pub const WHISPER_TINY_Q5_DOWNLOAD_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin";

/// Checksum-pinned Piper English voice (Hugging Face `okasi/piper-english-kusal-onnx-q8`).
///
/// Quantized ONNX (~16.8 MiB) derived from `rhasspy/piper-voices` `en_US-kusal-medium`.
/// HF LFS oid == SHA-256 of the ONNX bytes. Companion `.onnx.json` is fetched as a
/// checksummed sidecar on download (Piper requires it beside the model).
pub const PIPER_EN_KUSAL_Q8_PACK_ID: &str = "tts-piper-en-kusal-q8";
pub const PIPER_EN_KUSAL_Q8_FILE: &str = "en_US-kusal-medium-q8.onnx";
pub const PIPER_EN_KUSAL_Q8_SHA256: &str =
    "e7efc0d0015643e28b7533cdb4d432a91aa6ea7887b99f54a8a3c10f803730ef";
pub const PIPER_EN_KUSAL_Q8_BYTES: u64 = 16_762_569;
pub const PIPER_EN_KUSAL_Q8_DOWNLOAD_URL: &str =
    "https://huggingface.co/okasi/piper-english-kusal-onnx-q8/resolve/main/en_US-kusal-medium-q8.onnx";
pub const PIPER_EN_KUSAL_Q8_JSON_FILE: &str = "en_US-kusal-medium-q8.onnx.json";
pub const PIPER_EN_KUSAL_Q8_JSON_SHA256: &str =
    "ddd3c4dfd8b4f568150c934fb94912dd788d44db87f4f0a328c469d7a6761f41";
pub const PIPER_EN_KUSAL_Q8_JSON_BYTES: u64 = 4_884;
pub const PIPER_EN_KUSAL_Q8_JSON_DOWNLOAD_URL: &str =
    "https://huggingface.co/okasi/piper-english-kusal-onnx-q8/resolve/main/en_US-kusal-medium-q8.onnx.json";

/// Tiny checksummed STT fixture used to exercise download / remove (not Whisper weights).
pub const DEMO_STT_PACK_ID: &str = "stt-demo-fixture";
pub const DEMO_STT_FILE: &str = "demo-stt.weights";
pub const DEMO_STT_SHA256: &str =
    "b3bbdb816ceaf3dccff5669d4e6fe361cdd92ccac7fc027e1015e204262d883e";
pub const DEMO_STT_BYTES: u64 = 43;

/// Raw GitHub URL for the demo STT fixture on this pillar branch (checksum-pinned).
pub fn demo_stt_download_url() -> String {
    format!(
        "https://raw.githubusercontent.com/olivermikkola/keel-desktop/590d132bbf29fca2db9ab59d88d15c01ec1ba865/crates/keel-core/fixtures/audio/{DEMO_STT_FILE}"
    )
}

/// Demo STT fixture bytes (matches [`DEMO_STT_SHA256`]).
pub fn demo_stt_bytes() -> &'static [u8] {
    b"{\"keel_audio_pack\":\"demo-stt\",\"version\":1}\n"
}

/// Env override for the Whisper.cpp ggml model path when `runner.json` omits `model`.
pub const KEEL_WHISPER_MODEL_ENV: &str = "KEEL_WHISPER_MODEL";

/// Env override for the Piper ONNX model path when `runner.json` omits `model`.
pub const KEEL_PIPER_MODEL_ENV: &str = "KEEL_PIPER_MODEL";

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

/// How [`run_stt`] / shared `runner.json` interprets the `adapter` field.
///
/// Piper is stored here so TTS packs can reuse the same JSON shape; STT rejects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioSttAdapterKind {
    /// Generic: `command …args… <audio_path>` → transcript on stdout.
    #[default]
    Local,
    /// Whisper.cpp CLI: `command …args… -m <model> -f <audio_path>` → stdout.
    WhisperCpp,
    /// Piper TTS CLI (TTS packs only): stdin text + `--model` / `-f`.
    Piper,
}

impl AudioSttAdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::WhisperCpp => "whisper_cpp",
            Self::Piper => "piper",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "local" | "command" | "generic" => Ok(Self::Local),
            "whisper_cpp" | "whisper-cpp" | "whisper" => Ok(Self::WhisperCpp),
            "piper" => Ok(Self::Piper),
            other => {
                bail!("Unknown runner adapter '{other}'. Use 'local', 'whisper_cpp', or 'piper'.")
            }
        }
    }
}

/// How [`run_tts`] interprets `runner.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioTtsAdapterKind {
    /// Generic: `command …args… --text <text> --output <path>` (must create the file).
    #[default]
    Local,
    /// Piper CLI: text on stdin, `command …args… --model <model> -f <path>`.
    Piper,
}

impl AudioTtsAdapterKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Piper => "piper",
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "local" | "command" | "generic" => Ok(Self::Local),
            "piper" => Ok(Self::Piper),
            other => bail!(
                "Unknown TTS adapter '{other}'. Use 'local' (command … --text <text> --output <path>) or 'piper' (stdin text; --model / -f)."
            ),
        }
    }

    /// Map the shared `runner.json` `adapter` field into a TTS kind.
    ///
    /// `runner.json` reuses the same `adapter` key as STT ([`AudioSttAdapterKind`]).
    /// TTS accepts `local` and `piper`; STT-only kinds such as `whisper_cpp` fail honestly.
    pub fn from_runner_adapter(kind: AudioSttAdapterKind) -> Result<Self> {
        match kind {
            AudioSttAdapterKind::Local => Ok(Self::Local),
            AudioSttAdapterKind::Piper => Ok(Self::Piper),
            AudioSttAdapterKind::WhisperCpp => bail!(
                "TTS runner adapter 'whisper_cpp' is not supported. Use 'local' (command … --text <text> --output <path>) or 'piper' (stdin text; --model / -f)."
            ),
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
    /// Expected SHA-256 of the download artifact when pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// On-disk file name under the pack dir after a checksummed download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    /// Declared artifact size in bytes (0 when unknown / unpinned).
    #[serde(default)]
    pub bytes: u64,
    /// Optional companion artifact (e.g. Piper `<voice>.onnx.json`) fetched after weights.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar_file_name: Option<String>,
    /// Declared sidecar size in bytes (0 when absent / unknown).
    #[serde(default)]
    pub sidecar_bytes: u64,
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
    /// Shared `runner.json` adapter; omitted / default is local.
    ///
    /// STT: [`AudioSttAdapterKind`]. TTS: [`AudioTtsAdapterKind`] via
    /// [`AudioTtsAdapterKind::from_runner_adapter`].
    #[serde(default, skip_serializing_if = "adapter_is_local")]
    pub adapter: AudioSttAdapterKind,
    /// Model path for [`AudioSttAdapterKind::WhisperCpp`] / [`AudioSttAdapterKind::Piper`]
    /// (or set [`KEEL_WHISPER_MODEL_ENV`] / [`KEEL_PIPER_MODEL_ENV`]).
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
    /// Checksum-verified artifact downloaded into the pack directory.
    ChecksummedDownload {
        url: String,
        sha256: String,
        file_name: String,
    },
}

/// Catalog: demo fixture + pinned Whisper tiny q5_1 + unpinned Whisper base stub
/// + local TTS stub + pinned Piper English q8 voice.
pub fn list_audio_packs() -> Vec<AudioPackManifest> {
    vec![
        AudioPackManifest {
            id: DEMO_STT_PACK_ID.into(),
            capability: AudioCapability::Stt,
            display_name: "Speech to text · Demo fixture (checksummed)".into(),
            min_vram_mib: 0,
            runner_kind: AudioRunnerKind::AudioStt,
            download_url: Some(demo_stt_download_url()),
            sha256: Some(DEMO_STT_SHA256.into()),
            file_name: Some(DEMO_STT_FILE.into()),
            bytes: DEMO_STT_BYTES,
            sidecar_url: None,
            sidecar_sha256: None,
            sidecar_file_name: None,
            sidecar_bytes: 0,
            installed: false,
            runner_path: None,
        },
        AudioPackManifest {
            id: WHISPER_TINY_Q5_PACK_ID.into(),
            capability: AudioCapability::Stt,
            display_name: "Speech to text · Whisper tiny q5_1".into(),
            min_vram_mib: 512,
            runner_kind: AudioRunnerKind::AudioStt,
            download_url: Some(WHISPER_TINY_Q5_DOWNLOAD_URL.into()),
            sha256: Some(WHISPER_TINY_Q5_SHA256.into()),
            file_name: Some(WHISPER_TINY_Q5_FILE.into()),
            bytes: WHISPER_TINY_Q5_BYTES,
            sidecar_url: None,
            sidecar_sha256: None,
            sidecar_file_name: None,
            sidecar_bytes: 0,
            installed: false,
            runner_path: None,
        },
        AudioPackManifest {
            id: "stt-whisper-base".into(),
            capability: AudioCapability::Stt,
            display_name: "Speech to text · Whisper base (unpinned)".into(),
            min_vram_mib: 1024,
            runner_kind: AudioRunnerKind::AudioStt,
            download_url: Some(WHISPER_BASE_DOWNLOAD_PLACEHOLDER.into()),
            sha256: None,
            file_name: None,
            bytes: 0,
            sidecar_url: None,
            sidecar_sha256: None,
            sidecar_file_name: None,
            sidecar_bytes: 0,
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
            sha256: None,
            file_name: None,
            bytes: 0,
            sidecar_url: None,
            sidecar_sha256: None,
            sidecar_file_name: None,
            sidecar_bytes: 0,
            installed: false,
            runner_path: None,
        },
        AudioPackManifest {
            id: PIPER_EN_KUSAL_Q8_PACK_ID.into(),
            capability: AudioCapability::Tts,
            display_name: "Text to speech · Piper en_US kusal q8".into(),
            min_vram_mib: 256,
            runner_kind: AudioRunnerKind::AudioTts,
            download_url: Some(PIPER_EN_KUSAL_Q8_DOWNLOAD_URL.into()),
            sha256: Some(PIPER_EN_KUSAL_Q8_SHA256.into()),
            file_name: Some(PIPER_EN_KUSAL_Q8_FILE.into()),
            bytes: PIPER_EN_KUSAL_Q8_BYTES,
            sidecar_url: Some(PIPER_EN_KUSAL_Q8_JSON_DOWNLOAD_URL.into()),
            sidecar_sha256: Some(PIPER_EN_KUSAL_Q8_JSON_SHA256.into()),
            sidecar_file_name: Some(PIPER_EN_KUSAL_Q8_JSON_FILE.into()),
            sidecar_bytes: PIPER_EN_KUSAL_Q8_JSON_BYTES,
            installed: false,
            runner_path: None,
        },
    ]
}

/// True when the catalog entry has a fetchable URL + SHA-256 (not an unpinned placeholder).
pub fn audio_pack_download_is_pinned(pack: &AudioPackManifest) -> bool {
    let Some(url) = pack.download_url.as_deref() else {
        return false;
    };
    let Some(sha) = pack.sha256.as_deref() else {
        return false;
    };
    !url.trim().is_empty()
        && !sha.trim().is_empty()
        && !is_unpinned_audio_download_url(url)
        && pack
            .file_name
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty())
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

/// On-disk directory for STT transcripts and TTS wav copies.
pub fn audio_artifacts_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("audio-artifacts")
}

/// Kind of persisted audio artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioArtifactKind {
    SttTranscript,
    TtsOutput,
}

impl AudioArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SttTranscript => "stt_transcript",
            Self::TtsOutput => "tts_output",
        }
    }
}

/// One persisted STT transcript or TTS wav under [`audio_artifacts_dir`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioArtifact {
    pub id: String,
    pub kind: AudioArtifactKind,
    /// Absolute path to the primary artifact file (`.txt` or `.wav`).
    pub path: String,
    /// Unix seconds when the artifact was written.
    pub created_at: u64,
    /// Input audio path for STT, or the caller's original TTS output path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    /// Short text preview (STT transcript or TTS input), truncated for UI lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_preview: Option<String>,
}

const ARTIFACT_META_SUFFIX: &str = ".meta.json";
const ARTIFACT_PREVIEW_CHARS: usize = 120;

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn truncate_preview(text: &str) -> String {
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let preview: String = chars.by_ref().take(ARTIFACT_PREVIEW_CHARS).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn write_artifact_meta(dir: &Path, artifact: &AudioArtifact) -> Result<()> {
    let meta_path = dir.join(format!("{}{ARTIFACT_META_SUFFIX}", artifact.id));
    let raw = serde_json::to_string_pretty(artifact).context("serialize audio artifact meta")?;
    fs::write(&meta_path, raw)
        .with_context(|| format!("write audio artifact meta {}", meta_path.display()))?;
    Ok(())
}

/// Persist an STT transcript under [`audio_artifacts_dir`].
///
/// Best-effort for callers: returns the artifact record on success.
pub fn persist_stt_artifact(
    data_dir: &Path,
    transcript: &str,
    source_audio: &Path,
) -> Result<AudioArtifact> {
    let dir = audio_artifacts_dir(data_dir);
    fs::create_dir_all(&dir)
        .with_context(|| format!("create audio artifacts dir {}", dir.display()))?;
    let id = uuid::Uuid::new_v4().to_string();
    let txt_path = dir.join(format!("{id}.txt"));
    fs::write(&txt_path, transcript)
        .with_context(|| format!("write STT transcript {}", txt_path.display()))?;
    let artifact = AudioArtifact {
        id,
        kind: AudioArtifactKind::SttTranscript,
        path: txt_path.to_string_lossy().into_owned(),
        created_at: now_unix_secs(),
        source_path: Some(source_audio.to_string_lossy().into_owned()),
        text_preview: Some(truncate_preview(transcript)),
    };
    write_artifact_meta(&dir, &artifact)?;
    Ok(artifact)
}

/// Persist a TTS output by copying the wav into [`audio_artifacts_dir`].
pub fn persist_tts_artifact(
    data_dir: &Path,
    text: &str,
    output_path: &Path,
) -> Result<AudioArtifact> {
    let dir = audio_artifacts_dir(data_dir);
    fs::create_dir_all(&dir)
        .with_context(|| format!("create audio artifacts dir {}", dir.display()))?;
    let id = uuid::Uuid::new_v4().to_string();
    let wav_path = dir.join(format!("{id}.wav"));
    fs::copy(output_path, &wav_path).with_context(|| {
        format!(
            "copy TTS output {} → {}",
            output_path.display(),
            wav_path.display()
        )
    })?;
    let artifact = AudioArtifact {
        id,
        kind: AudioArtifactKind::TtsOutput,
        path: wav_path.to_string_lossy().into_owned(),
        created_at: now_unix_secs(),
        source_path: Some(output_path.to_string_lossy().into_owned()),
        text_preview: Some(truncate_preview(text)),
    };
    write_artifact_meta(&dir, &artifact)?;
    Ok(artifact)
}

/// List persisted audio artifacts (newest first). Skips unreadable meta files.
pub fn list_audio_artifacts(data_dir: &Path) -> Vec<AudioArtifact> {
    let dir = audio_artifacts_dir(data_dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut artifacts = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(ARTIFACT_META_SUFFIX) => n,
            _ => continue,
        };
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(mut artifact) = serde_json::from_str::<AudioArtifact>(&raw) else {
            continue;
        };
        // Prefer the id from the filename when meta is incomplete.
        if artifact.id.is_empty() {
            artifact.id = name.trim_end_matches(ARTIFACT_META_SUFFIX).to_string();
        }
        artifacts.push(artifact);
    }
    artifacts.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    artifacts
}

/// Remove all files under [`audio_artifacts_dir`]. No-op if the dir is missing.
pub fn clear_audio_artifacts(data_dir: &Path) -> Result<()> {
    let dir = audio_artifacts_dir(data_dir);
    if !dir.exists() {
        return Ok(());
    }
    fs::remove_dir_all(&dir).with_context(|| format!("clear audio artifacts {}", dir.display()))?;
    Ok(())
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

/// Prefer a registered runner over packs that only have downloaded weights.
pub fn resolve_audio_runner(
    data_dir: &Path,
    capability: AudioCapability,
) -> Result<(AudioPackManifest, AudioRunnerConfig)> {
    let packs = list_audio_packs();
    for pack in packs
        .into_iter()
        .filter(|pack| pack.capability == capability)
    {
        if is_audio_pack_installed(data_dir, &pack.id) {
            if let Some(runner) = read_audio_runner(data_dir, &pack.id) {
                return Ok((pack, runner));
            }
        }
    }
    let pack = require_installed_audio_pack(data_dir, capability)?;
    let runner = require_runner(data_dir, &pack)?;
    Ok((pack, runner))
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
    let catalog_model = pack
        .file_name
        .as_ref()
        .map(|name| pack_dir(data_dir, pack_id).join(name));
    let model_path = if model.trim().is_empty() {
        catalog_model
            .as_deref()
            .filter(|path| path.is_file())
            .context("Download this pack's model or provide the path to a local model file.")?
    } else {
        Path::new(model.trim())
    };
    if !model_path.is_file() {
        bail!(
            "Whisper.cpp model '{}' does not exist. Pin checksummed weights before registering, or point at a ggml file already on this machine.",
            model_path.display()
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
            model: Some(model_path.to_string_lossy().into_owned()),
        },
        "whisper_cpp_runner\n",
        AudioInstallMeta::LocalRunner {
            command: command.to_string(),
        },
    )
}

/// Register a Piper TTS runner (`adapter: piper`) and mark installed.
///
/// `command` is the Piper binary (absolute or PATH name). `model` must be an
/// existing `.onnx` voice file — Desktop does not bundle Piper; the catalog can
/// download checksummed ONNX (+ sidecar JSON) for [`PIPER_EN_KUSAL_Q8_PACK_ID`].
pub fn register_piper_runner(
    data_dir: &Path,
    pack_id: &str,
    command: &str,
    model: &str,
    args: &[String],
) -> Result<AudioPackManifest> {
    let pack = ensure_catalog_pack(pack_id)?;
    if pack.capability != AudioCapability::Tts {
        bail!(
            "Piper runner is only valid for text-to-speech packs; '{}' is {}.",
            pack_id,
            pack.capability.label()
        );
    }
    let command = command.trim();
    if command.is_empty() {
        bail!("Provide a Piper binary path or PATH name (for example piper).");
    }
    let catalog_model = pack
        .file_name
        .as_ref()
        .map(|name| pack_dir(data_dir, pack_id).join(name));
    let model_path = if model.trim().is_empty() {
        catalog_model
            .as_deref()
            .filter(|path| path.is_file())
            .context("Download this pack's model or provide the path to a local model file.")?
    } else {
        Path::new(model.trim())
    };
    if !model_path.is_file() {
        bail!(
            "Piper model '{}' does not exist. Download the checksummed catalog voice or point at an ONNX file already on this machine.",
            model_path.display()
        );
    }
    let json_sidecar = PathBuf::from(format!("{}.json", model_path.display()));
    if !json_sidecar.is_file() {
        bail!(
            "Piper config '{}' is missing beside the ONNX model. Piper needs `<model>.onnx.json` next to the voice file.",
            json_sidecar.display()
        );
    }
    let cmd_path = Path::new(command);
    let looks_like_path = cmd_path.is_absolute() || command.contains('/') || command.contains('\\');
    if looks_like_path && !cmd_path.exists() {
        bail!(
            "Piper binary '{}' does not exist. Pick a binary on this machine.",
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
            adapter: AudioSttAdapterKind::Piper,
            model: Some(model_path.to_string_lossy().into_owned()),
        },
        "piper_runner\n",
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

/// Remove an installed audio pack directory (weights, runner.json, markers).
pub fn remove_audio_pack(data_dir: &Path, pack_id: &str) -> Result<()> {
    let _ = ensure_catalog_pack(pack_id)?;
    let dir = pack_dir(data_dir, pack_id);
    if !dir.exists() {
        bail!("No installed audio pack found for `{pack_id}`");
    }
    fs::remove_dir_all(&dir).with_context(|| format!("remove {}", dir.display()))?;
    Ok(())
}

/// Download a checksum-pinned catalog pack into `{data_dir}/audio-packs/<id>/`.
///
/// Requires catalog `download_url` + `sha256` + `file_name`. Unpinned Whisper
/// placeholders are refused. Does not register a runner — Settings still needs
/// a local / Whisper.cpp binary to run STT/TTS.
pub async fn download_audio_pack<F>(
    data_dir: &Path,
    pack_id: &str,
    on_progress: F,
) -> Result<AudioPackManifest>
where
    F: FnMut(crate::models::DownloadProgress),
{
    let pack = ensure_catalog_pack(pack_id)?;
    download_audio_pack_entry(data_dir, &pack, on_progress).await
}

/// Download using an explicit catalog entry (tests may override `download_url`).
pub async fn download_audio_pack_entry<F>(
    data_dir: &Path,
    pack: &AudioPackManifest,
    on_progress: F,
) -> Result<AudioPackManifest>
where
    F: FnMut(crate::models::DownloadProgress),
{
    if !audio_pack_download_is_pinned(pack) {
        bail!(
            "Pack '{}' download is not pinned yet (need http(s) URL + sha256 + file_name). Register a local or Whisper.cpp runner, or wait for a checksummed catalog pin.",
            pack.id
        );
    }
    let url = pack.download_url.as_deref().expect("pinned");
    let sha256 = pack.sha256.as_deref().expect("pinned");
    let file_name = pack.file_name.as_deref().expect("pinned");
    if file_name.contains('/') || file_name.contains('\\') || file_name == ".." || file_name == "."
    {
        bail!("Invalid audio pack file_name '{file_name}'");
    }

    let dir = pack_dir(data_dir, &pack.id);
    fs::create_dir_all(&dir).with_context(|| format!("create pack dir {}", dir.display()))?;
    let dest = dir.join(file_name);
    if dest.is_file() {
        let _ = fs::remove_file(&dest);
    }

    crate::models::download_gguf(url, &dest, Some(sha256), &pack.id, on_progress).await?;

    // Optional companion artifact (Piper needs `<voice>.onnx.json` beside ONNX weights).
    if let (Some(sidecar_url), Some(sidecar_sha), Some(sidecar_name)) = (
        pack.sidecar_url.as_deref(),
        pack.sidecar_sha256.as_deref(),
        pack.sidecar_file_name.as_deref(),
    ) {
        if sidecar_name.contains('/')
            || sidecar_name.contains('\\')
            || sidecar_name == ".."
            || sidecar_name == "."
        {
            bail!("Invalid audio pack sidecar_file_name '{sidecar_name}'");
        }
        let sidecar_dest = dir.join(sidecar_name);
        if sidecar_dest.is_file() {
            let _ = fs::remove_file(&sidecar_dest);
        }
        crate::models::download_gguf(
            sidecar_url,
            &sidecar_dest,
            Some(sidecar_sha),
            &format!("{}-sidecar", pack.id),
            |_| {},
        )
        .await
        .context("download audio pack sidecar")?;
        if pack.sidecar_bytes > 0 {
            let got = fs::metadata(&sidecar_dest).map(|m| m.len()).unwrap_or(0);
            if got != pack.sidecar_bytes {
                bail!(
                    "Audio pack sidecar '{}' has unexpected size {got} (expected {})",
                    sidecar_dest.display(),
                    pack.sidecar_bytes
                );
            }
        }
    }

    fs::write(dir.join(INSTALLED_MARKER), b"checksummed\n").context("write install marker")?;
    write_install_meta(
        &dir,
        &AudioInstallMeta::ChecksummedDownload {
            url: url.to_string(),
            sha256: sha256.to_string(),
            file_name: file_name.to_string(),
        },
    )?;
    Ok(resolve_audio_pack(data_dir, pack.clone()))
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

/// Run `command …args…` with `stdin_text` on stdin; return trimmed stdout.
fn run_local_command_with_stdin(
    config: &AudioRunnerConfig,
    argv: &[String],
    stdin_text: &str,
) -> Result<String> {
    let mut child = Command::new(&config.command)
        .args(argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start audio runner '{}'", config.command))?;
    {
        let mut stdin = child
            .stdin
            .take()
            .context("audio runner stdin unavailable")?;
        stdin
            .write_all(stdin_text.as_bytes())
            .context("write text to audio runner stdin")?;
        // Piper reads until EOF; drop closes the pipe.
    }
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed waiting for audio runner '{}'", config.command))?;
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

/// Resolve the ONNX model path for a Piper runner.
pub fn resolve_piper_model(config: &AudioRunnerConfig) -> Result<String> {
    if let Some(model) = config
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(model.to_string());
    }
    if let Ok(from_env) = std::env::var(KEEL_PIPER_MODEL_ENV) {
        let from_env = from_env.trim().to_string();
        if !from_env.is_empty() {
            return Ok(from_env);
        }
    }
    bail!(
        "Piper adapter needs an ONNX model path in runner.json (\"model\") or {KEEL_PIPER_MODEL_ENV}. No Piper voices are bundled."
    );
}

/// Build Piper CLI args after `command` (does not include the binary name).
///
/// Contract: `…args… --model <model> -f <path>` with text on stdin.
pub fn piper_argv(config: &AudioRunnerConfig, model: &str, output_path: &Path) -> Vec<String> {
    let mut argv = config.args.clone();
    argv.push("--model".into());
    argv.push(model.to_string());
    argv.push("-f".into());
    argv.push(output_path.to_string_lossy().into_owned());
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
pub fn stt_adapter_from_runner(config: AudioRunnerConfig) -> Result<Box<dyn SttAdapter>> {
    match config.adapter {
        AudioSttAdapterKind::Local => Ok(Box::new(LocalCommandSttAdapter { config })),
        AudioSttAdapterKind::WhisperCpp => Ok(Box::new(WhisperCppSttAdapter { config })),
        AudioSttAdapterKind::Piper => bail!(
            "STT runner adapter 'piper' is not supported. Use 'local' or 'whisper_cpp'. Piper is a TTS engine."
        ),
    }
}

/// Speech-to-text entry point.
///
/// Requires an installed pack and a registered (or env-overridden) runner.
/// Dispatches to [`LocalCommandSttAdapter`] or [`WhisperCppSttAdapter`] from
/// `runner.json`'s `adapter` field.
pub fn run_stt(data_dir: &Path, audio_path: &Path) -> Result<String> {
    let (pack, runner) = resolve_audio_runner(data_dir, AudioCapability::Stt)?;
    run_stt_with_runner(data_dir, audio_path, &pack, runner)
}

pub fn run_stt_with_runner(
    data_dir: &Path,
    audio_path: &Path,
    pack: &AudioPackManifest,
    mut runner: AudioRunnerConfig,
) -> Result<String> {
    if !audio_path.is_file() {
        bail!("Audio file not found: {}", audio_path.display());
    }
    // Prefer checksummed pack weights when Whisper.cpp has no explicit model path.
    if runner.adapter == AudioSttAdapterKind::WhisperCpp && runner.model.is_none() {
        if let Some(name) = pack.file_name.as_deref() {
            let candidate = pack_dir(data_dir, &pack.id).join(name);
            if candidate.is_file() {
                runner.model = Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    let command = runner.command.clone();
    let transcript = stt_adapter_from_runner(runner)?.transcribe(audio_path)?;
    if transcript.is_empty() {
        bail!(
            "Speech-to-text runner '{}' produced no transcript for {}.",
            command,
            audio_path.display()
        );
    }
    // Best-effort local record; STT success does not depend on artifact I/O.
    let _ = persist_stt_artifact(data_dir, &transcript, audio_path);
    Ok(transcript)
}

/// Build local TTS CLI args after `command` (does not include the binary name).
///
/// Contract: `…args… --text <text> --output <path>`.
pub fn local_tts_argv(config: &AudioRunnerConfig, text: &str, output_path: &Path) -> Vec<String> {
    let mut argv = config.args.clone();
    argv.push("--text".into());
    argv.push(text.to_string());
    argv.push("--output".into());
    argv.push(output_path.to_string_lossy().into_owned());
    argv
}

/// TTS adapter invoked by [`run_tts`] once a runner is resolved.
pub trait TtsAdapter {
    fn synthesize(&self, text: &str, output_path: &Path) -> Result<()>;
}

/// Default TTS adapter: `command …args… --text <text> --output <path>`.
pub struct LocalCommandTtsAdapter {
    pub config: AudioRunnerConfig,
}

impl TtsAdapter for LocalCommandTtsAdapter {
    fn synthesize(&self, text: &str, output_path: &Path) -> Result<()> {
        let argv = local_tts_argv(&self.config, text, output_path);
        // Fold args into a temporary config so `run_local_command` does not
        // double-apply `config.args` (already present in argv).
        let launch = AudioRunnerConfig {
            command: self.config.command.clone(),
            args: argv,
            adapter: AudioSttAdapterKind::Local,
            model: self.config.model.clone(),
        };
        let _ = run_local_command(&launch, &[])?;
        Ok(())
    }
}

/// Piper TTS adapter: text on stdin, `command …args… --model <model> -f <path>`.
///
/// Fails honestly when the ONNX model (or sibling `.onnx.json`) is missing.
pub struct PiperTtsAdapter {
    pub config: AudioRunnerConfig,
}

impl TtsAdapter for PiperTtsAdapter {
    fn synthesize(&self, text: &str, output_path: &Path) -> Result<()> {
        let model = resolve_piper_model(&self.config)?;
        let model_path = Path::new(&model);
        if !model_path.is_file() {
            bail!(
                "Piper model '{}' is not a file. Download the checksummed catalog voice or update runner.json / {KEEL_PIPER_MODEL_ENV}.",
                model
            );
        }
        let json_sidecar = PathBuf::from(format!("{}.json", model_path.display()));
        if !json_sidecar.is_file() {
            bail!(
                "Piper config '{}' is missing. Piper needs `<model>.onnx.json` beside the ONNX voice.",
                json_sidecar.display()
            );
        }
        let argv = piper_argv(&self.config, &model, output_path);
        let _ = run_local_command_with_stdin(&self.config, &argv, text)?;
        Ok(())
    }
}

/// Pick a [`TtsAdapter`] from `runner.json`, rejecting STT-only adapter kinds.
pub fn tts_adapter_from_runner(config: AudioRunnerConfig) -> Result<Box<dyn TtsAdapter>> {
    match AudioTtsAdapterKind::from_runner_adapter(config.adapter)? {
        AudioTtsAdapterKind::Local => Ok(Box::new(LocalCommandTtsAdapter { config })),
        AudioTtsAdapterKind::Piper => Ok(Box::new(PiperTtsAdapter { config })),
    }
}

/// Text-to-speech entry point.
///
/// Requires an installed pack and a registered (or env-overridden) runner.
/// Dispatches to [`LocalCommandTtsAdapter`] or [`PiperTtsAdapter`] from
/// `runner.json`'s `adapter` field.
pub fn run_tts(data_dir: &Path, text: &str, output_path: &Path) -> Result<()> {
    let (pack, runner) = resolve_audio_runner(data_dir, AudioCapability::Tts)?;
    run_tts_with_runner(data_dir, text, output_path, &pack, runner)
}

pub fn run_tts_with_runner(
    data_dir: &Path,
    text: &str,
    output_path: &Path,
    pack: &AudioPackManifest,
    mut runner: AudioRunnerConfig,
) -> Result<()> {
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
    // Prefer checksummed pack weights when Piper has no explicit model path.
    if runner.adapter == AudioSttAdapterKind::Piper && runner.model.is_none() {
        if let Some(name) = pack.file_name.as_deref() {
            let candidate = pack_dir(data_dir, &pack.id).join(name);
            if candidate.is_file() {
                runner.model = Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    let command = runner.command.clone();
    tts_adapter_from_runner(runner)?.synthesize(text, output_path)?;
    if !output_path.exists() {
        bail!(
            "Text-to-speech runner '{}' finished but did not create {}.",
            command,
            output_path.display()
        );
    }
    // Best-effort local record; TTS success does not depend on artifact I/O.
    let _ = persist_tts_artifact(data_dir, text, output_path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_stt_and_tts_stubs_not_downloaded() {
        let packs = list_audio_packs();
        assert_eq!(packs.len(), 5);
        let demo = packs
            .iter()
            .find(|p| p.id == DEMO_STT_PACK_ID)
            .expect("demo stt fixture");
        let tiny = packs
            .iter()
            .find(|p| p.id == WHISPER_TINY_Q5_PACK_ID)
            .expect("whisper tiny q5");
        let whisper_base = packs
            .iter()
            .find(|p| p.id == "stt-whisper-base")
            .expect("whisper base stub");
        let tts = packs
            .iter()
            .find(|p| p.id == "tts-local-en")
            .expect("tts stub");
        let piper = packs
            .iter()
            .find(|p| p.id == PIPER_EN_KUSAL_Q8_PACK_ID)
            .expect("piper kusal q8");
        assert!(audio_pack_download_is_pinned(demo));
        assert_eq!(demo.sha256.as_deref(), Some(DEMO_STT_SHA256));
        assert_eq!(demo.bytes, DEMO_STT_BYTES);
        assert_eq!(demo_stt_bytes().len() as u64, DEMO_STT_BYTES);
        assert!(audio_pack_download_is_pinned(tiny));
        assert_eq!(tiny.sha256.as_deref(), Some(WHISPER_TINY_Q5_SHA256));
        assert_eq!(tiny.bytes, WHISPER_TINY_Q5_BYTES);
        assert_eq!(
            tiny.download_url.as_deref(),
            Some(WHISPER_TINY_Q5_DOWNLOAD_URL)
        );
        assert!(!audio_pack_download_is_pinned(whisper_base));
        assert_eq!(
            whisper_base.download_url.as_deref(),
            Some(WHISPER_BASE_DOWNLOAD_PLACEHOLDER)
        );
        assert!(is_unpinned_audio_download_url(
            whisper_base.download_url.as_deref().unwrap()
        ));
        assert!(tts.download_url.is_none());
        assert!(audio_pack_download_is_pinned(piper));
        assert_eq!(piper.sha256.as_deref(), Some(PIPER_EN_KUSAL_Q8_SHA256));
        assert_eq!(piper.bytes, PIPER_EN_KUSAL_Q8_BYTES);
        assert_eq!(piper.file_name.as_deref(), Some(PIPER_EN_KUSAL_Q8_FILE));
        assert_eq!(
            piper.download_url.as_deref(),
            Some(PIPER_EN_KUSAL_Q8_DOWNLOAD_URL)
        );
        assert_eq!(
            piper.sidecar_url.as_deref(),
            Some(PIPER_EN_KUSAL_Q8_JSON_DOWNLOAD_URL)
        );
        assert_eq!(
            piper.sidecar_sha256.as_deref(),
            Some(PIPER_EN_KUSAL_Q8_JSON_SHA256)
        );
        assert_eq!(
            piper.sidecar_file_name.as_deref(),
            Some(PIPER_EN_KUSAL_Q8_JSON_FILE)
        );
        assert_eq!(piper.sidecar_bytes, PIPER_EN_KUSAL_Q8_JSON_BYTES);
        assert_eq!(tiny.runner_kind, AudioRunnerKind::AudioStt);
        assert_eq!(tts.runner_kind, AudioRunnerKind::AudioTts);
        assert_eq!(piper.runner_kind, AudioRunnerKind::AudioTts);
        assert!(tiny.min_vram_mib > 0);
        assert!(tts.min_vram_mib > 0);
        assert!(piper.min_vram_mib > 0);
    }

    #[cfg(unix)]
    #[test]
    fn whisper_cpp_uses_downloaded_pack_weights_when_model_omitted() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile_dir("whisper-pack-weights");
        let weights = pack_dir(&dir, WHISPER_TINY_Q5_PACK_ID).join(WHISPER_TINY_Q5_FILE);
        fs::create_dir_all(weights.parent().unwrap()).unwrap();
        fs::write(&weights, b"fake-ggml").unwrap();
        fs::write(
            pack_dir(&dir, WHISPER_TINY_Q5_PACK_ID).join(INSTALLED_MARKER),
            b"checksummed\n",
        )
        .unwrap();

        let script = dir.join("fake-whisper.sh");
        // argv: command -m <model> -f <audio> → $2 is model
        fs::write(&script, "#!/bin/sh\nprintf '%s\\n' \"$2\"\n").unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        // Register without model — run_stt should pick pack weights.
        let config = AudioRunnerConfig {
            command: script.to_str().unwrap().into(),
            args: Vec::new(),
            adapter: AudioSttAdapterKind::WhisperCpp,
            model: None,
        };
        let pack_path = pack_dir(&dir, WHISPER_TINY_Q5_PACK_ID);
        fs::write(
            pack_path.join(RUNNER_FILE),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();

        let wav = dir.join("clip.wav");
        fs::write(&wav, b"RIFF").unwrap();
        let out = run_stt(&dir, &wav).unwrap();
        assert!(
            out.contains(WHISPER_TINY_Q5_FILE),
            "expected downloaded weights path in argv/transcript, got {out}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_audio_pack_resolves_stub_ids() {
        assert_eq!(
            find_audio_pack("stt-whisper-base").map(|p| p.id),
            Some("stt-whisper-base".into())
        );
        assert_eq!(
            find_audio_pack(DEMO_STT_PACK_ID).map(|p| p.id),
            Some(DEMO_STT_PACK_ID.into())
        );
        assert!(find_audio_pack("missing").is_none());
    }

    #[test]
    fn demo_stt_fixture_matches_pinned_checksum() {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(demo_stt_bytes());
        assert_eq!(format!("{digest:x}"), DEMO_STT_SHA256);
        let on_disk = include_bytes!("../fixtures/audio/demo-stt.weights");
        assert_eq!(on_disk, demo_stt_bytes());
    }

    #[tokio::test]
    async fn download_audio_pack_refuses_unpinned_whisper() {
        let data = tempfile_dir("audio-dl-unpinned");
        let err = download_audio_pack(&data, "stt-whisper-base", |_| {})
            .await
            .expect_err("unpinned whisper must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("not pinned yet") && msg.contains("sha256"),
            "unexpected: {msg}"
        );
        let _ = fs::remove_dir_all(&data);
    }

    #[tokio::test]
    async fn download_audio_pack_verifies_checksum_over_http() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let data = tempfile_dir("audio-dl-http");
        let body = demo_stt_bytes();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
        });

        let mut pack = find_audio_pack(DEMO_STT_PACK_ID).unwrap();
        pack.download_url = Some(format!("http://{addr}/demo-stt.weights"));

        let installed = download_audio_pack_entry(&data, &pack, |_| {})
            .await
            .unwrap();
        server.await.unwrap();
        assert!(installed.installed);
        assert!(is_audio_pack_installed(&data, DEMO_STT_PACK_ID));
        let dest = pack_dir(&data, DEMO_STT_PACK_ID).join(DEMO_STT_FILE);
        assert_eq!(fs::read(&dest).unwrap(), demo_stt_bytes());
        remove_audio_pack(&data, DEMO_STT_PACK_ID).unwrap();
        assert!(!is_audio_pack_installed(&data, DEMO_STT_PACK_ID));
        let _ = fs::remove_dir_all(&data);
    }

    /// Piper download must fetch both ONNX weights and the checksummed `.onnx.json` sidecar.
    #[tokio::test]
    async fn download_piper_pack_fetches_onnx_and_json_sidecar() {
        use sha2::{Digest, Sha256};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let onnx_body = b"tiny-piper-onnx-fixture";
        let json_body = br#"{"tiny":true}"#;
        let onnx_sha = format!("{:x}", Sha256::digest(onnx_body));
        let json_sha = format!("{:x}", Sha256::digest(json_body));
        assert_eq!(
            onnx_sha,
            "eb2bdebed0bf49c5a3c9f45cccccdbee1d9d0211067ef81cef117cd529cf713b"
        );
        assert_eq!(
            json_sha,
            "bbf4bb70bc84d7bd89fe56c409ec95ebfb251dfbbf918517278361a17c0ca207"
        );

        let data = tempfile_dir("audio-dl-piper-sidecar");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let n = socket.read(&mut request).await.unwrap();
                let req = String::from_utf8_lossy(&request[..n]);
                let body: &[u8] = if req.contains(PIPER_EN_KUSAL_Q8_JSON_FILE) {
                    json_body
                } else if req.contains(PIPER_EN_KUSAL_Q8_FILE) {
                    onnx_body
                } else {
                    panic!("unexpected request path: {req}");
                };
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                socket.write_all(body).await.unwrap();
            }
        });

        let mut pack = find_audio_pack(PIPER_EN_KUSAL_Q8_PACK_ID).unwrap();
        pack.download_url = Some(format!("http://{addr}/{}", PIPER_EN_KUSAL_Q8_FILE));
        pack.sha256 = Some(onnx_sha);
        pack.bytes = onnx_body.len() as u64;
        pack.sidecar_url = Some(format!("http://{addr}/{}", PIPER_EN_KUSAL_Q8_JSON_FILE));
        pack.sidecar_sha256 = Some(json_sha);
        pack.sidecar_bytes = json_body.len() as u64;

        let installed = download_audio_pack_entry(&data, &pack, |_| {})
            .await
            .unwrap();
        server.await.unwrap();
        assert!(installed.installed);
        assert!(is_audio_pack_installed(&data, PIPER_EN_KUSAL_Q8_PACK_ID));
        let pack_path = pack_dir(&data, PIPER_EN_KUSAL_Q8_PACK_ID);
        assert_eq!(
            fs::read(pack_path.join(PIPER_EN_KUSAL_Q8_FILE)).unwrap(),
            onnx_body
        );
        assert_eq!(
            fs::read(pack_path.join(PIPER_EN_KUSAL_Q8_JSON_FILE)).unwrap(),
            json_body
        );
        remove_audio_pack(&data, PIPER_EN_KUSAL_Q8_PACK_ID).unwrap();
        let _ = fs::remove_dir_all(&data);
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
        mark_audio_pack_installed(&dir, DEMO_STT_PACK_ID).unwrap();
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
        let artifacts = list_audio_artifacts(&dir);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, AudioArtifactKind::SttTranscript);
        assert_eq!(
            fs::read_to_string(&artifacts[0].path).unwrap().trim(),
            "hello from stt"
        );
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

        let pack = register_audio_pack_runner(&dir, "tts-local-en", script.to_str().unwrap(), &[])
            .unwrap();
        assert!(pack.installed);
        assert_eq!(pack.runner_path.as_deref(), script.to_str());
        let config = read_audio_runner(&dir, "tts-local-en").expect("runner.json");
        assert_eq!(config.adapter, AudioSttAdapterKind::Local);
        assert_eq!(
            AudioTtsAdapterKind::from_runner_adapter(config.adapter).unwrap(),
            AudioTtsAdapterKind::Local
        );

        let out = dir.join("out.wav");
        run_tts(&dir, "hello world", &out).unwrap();
        assert_eq!(fs::read_to_string(&out).unwrap(), "wav");
        let artifacts = list_audio_artifacts(&dir);
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].kind, AudioArtifactKind::TtsOutput);
        assert_eq!(fs::read_to_string(&artifacts[0].path).unwrap(), "wav");
        assert_eq!(artifacts[0].text_preview.as_deref(), Some("hello world"));
    }

    #[test]
    fn installed_tts_without_runner_gives_clear_error() {
        let dir = tempfile_dir("tts-no-runner");
        mark_audio_pack_installed(&dir, "tts-local-en").unwrap();
        let out = dir.join("out.wav");
        let err = run_tts(&dir, "hello", &out).unwrap_err().to_string();
        assert!(
            err.contains("no local runner is registered"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn local_tts_argv_follows_documented_contract() {
        let config = AudioRunnerConfig {
            command: "tts.sh".into(),
            args: vec!["--voice".into(), "en".into()],
            adapter: AudioSttAdapterKind::Local,
            model: None,
        };
        let argv = local_tts_argv(&config, "hello world", Path::new("/tmp/out.wav"));
        assert_eq!(
            argv,
            vec![
                "--voice".to_string(),
                "en".into(),
                "--text".into(),
                "hello world".into(),
                "--output".into(),
                "/tmp/out.wav".into(),
            ]
        );
    }

    #[test]
    fn tts_adapter_rejects_whisper_cpp_kind() {
        let config = AudioRunnerConfig {
            command: "tts.sh".into(),
            args: Vec::new(),
            adapter: AudioSttAdapterKind::WhisperCpp,
            model: None,
        };
        let err = match tts_adapter_from_runner(config) {
            Ok(_) => panic!("expected whisper_cpp TTS adapter to be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("whisper_cpp") && err.contains("not supported"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn run_tts_rejects_whisper_cpp_runner_json() {
        let dir = tempfile_dir("tts-whisper-adapter");
        mark_audio_pack_installed(&dir, "tts-local-en").unwrap();
        let pack_path = pack_dir(&dir, "tts-local-en");
        let config = AudioRunnerConfig {
            command: "tts.sh".into(),
            args: Vec::new(),
            adapter: AudioSttAdapterKind::WhisperCpp,
            model: None,
        };
        fs::write(
            pack_path.join(RUNNER_FILE),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
        let err = run_tts(&dir, "hello", &dir.join("out.wav"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("whisper_cpp") && err.contains("not supported"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn tts_adapter_kind_parse_accepts_local_and_piper() {
        assert_eq!(
            AudioTtsAdapterKind::parse("local").unwrap(),
            AudioTtsAdapterKind::Local
        );
        assert_eq!(
            AudioTtsAdapterKind::parse("").unwrap(),
            AudioTtsAdapterKind::Local
        );
        assert_eq!(
            AudioTtsAdapterKind::parse("piper").unwrap(),
            AudioTtsAdapterKind::Piper
        );
        let err = AudioTtsAdapterKind::parse("whisper_cpp")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Unknown TTS adapter"), "unexpected: {err}");
    }

    #[test]
    fn piper_argv_follows_documented_contract() {
        let config = AudioRunnerConfig {
            command: "piper".into(),
            args: vec!["--quiet".into()],
            adapter: AudioSttAdapterKind::Piper,
            model: Some("/voices/en.onnx".into()),
        };
        let argv = piper_argv(&config, "/voices/en.onnx", Path::new("/tmp/out.wav"));
        assert_eq!(
            argv,
            vec![
                "--quiet".to_string(),
                "--model".into(),
                "/voices/en.onnx".into(),
                "-f".into(),
                "/tmp/out.wav".into(),
            ]
        );
    }

    #[test]
    fn register_piper_rejects_missing_model_or_json() {
        let dir = tempfile_dir("piper-reg");
        let err = register_piper_runner(
            &dir,
            PIPER_EN_KUSAL_Q8_PACK_ID,
            "piper",
            dir.join("no-such.onnx").to_str().unwrap(),
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("does not exist"), "unexpected: {err}");

        let model = dir.join("voice.onnx");
        fs::write(&model, b"fake-onnx").unwrap();
        let err = register_piper_runner(
            &dir,
            PIPER_EN_KUSAL_Q8_PACK_ID,
            "piper",
            model.to_str().unwrap(),
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("missing") && err.contains(".json"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn register_piper_rejects_stt_pack() {
        let dir = tempfile_dir("piper-stt");
        let model = dir.join("voice.onnx");
        fs::write(&model, b"fake").unwrap();
        fs::write(format!("{}.json", model.display()), b"{}").unwrap();
        let err = register_piper_runner(
            &dir,
            "stt-whisper-base",
            "piper",
            model.to_str().unwrap(),
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("text-to-speech") || err.contains("only valid"),
            "unexpected: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn register_piper_runner_and_run_tts() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile_dir("piper-runner");
        let model = dir.join("voice.onnx");
        fs::write(&model, b"not-real-onnx").unwrap();
        fs::write(format!("{}.json", model.display()), b"{\"audio\":{}}").unwrap();
        let script = dir.join("fake-piper.sh");
        // Read stdin, require --model / -f, write a tiny wav to output.
        fs::write(
            &script,
            "#!/bin/sh\nmodel=\"\"\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"--model\" ]; then model=\"$2\"; shift 2; continue; fi\n  if [ \"$1\" = \"-f\" ]; then out=\"$2\"; shift 2; continue; fi\n  shift\ndone\n[ -n \"$model\" ] && [ -n \"$out\" ] || exit 2\ncat >/dev/null\nprintf 'RIFFPIPE' > \"$out\"\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let pack = register_piper_runner(
            &dir,
            PIPER_EN_KUSAL_Q8_PACK_ID,
            script.to_str().unwrap(),
            model.to_str().unwrap(),
            &[],
        )
        .unwrap();
        assert!(pack.installed);
        let raw = fs::read_to_string(pack_dir(&dir, PIPER_EN_KUSAL_Q8_PACK_ID).join(RUNNER_FILE))
            .unwrap();
        assert!(raw.contains("piper"));
        assert!(raw.contains("voice.onnx"));

        let out = dir.join("spoken.wav");
        run_tts(&dir, "hello piper", &out).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"RIFFPIPE");
    }

    #[cfg(unix)]
    #[test]
    fn piper_uses_downloaded_pack_weights_when_model_omitted() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile_dir("piper-pack-weights");
        let pack_path = pack_dir(&dir, PIPER_EN_KUSAL_Q8_PACK_ID);
        let weights = pack_path.join(PIPER_EN_KUSAL_Q8_FILE);
        fs::create_dir_all(&pack_path).unwrap();
        fs::write(&weights, b"fake-onnx").unwrap();
        fs::write(
            pack_path.join(PIPER_EN_KUSAL_Q8_JSON_FILE),
            b"{\"audio\":{}}",
        )
        .unwrap();
        fs::write(pack_path.join(INSTALLED_MARKER), b"checksummed\n").unwrap();

        let script = dir.join("fake-piper.sh");
        fs::write(
            &script,
            "#!/bin/sh\n# print model path then write output\nmodel=\"\"\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"--model\" ]; then model=\"$2\"; shift 2; continue; fi\n  if [ \"$1\" = \"-f\" ]; then out=\"$2\"; shift 2; continue; fi\n  shift\ndone\ncat >/dev/null\nprintf '%s' \"$model\" > \"$out\"\n",
        )
        .unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let config = AudioRunnerConfig {
            command: script.to_str().unwrap().into(),
            args: Vec::new(),
            adapter: AudioSttAdapterKind::Piper,
            model: None,
        };
        fs::write(
            pack_path.join(RUNNER_FILE),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();

        let out = dir.join("out.wav");
        run_tts(&dir, "hi", &out).unwrap();
        let written = fs::read_to_string(&out).unwrap();
        assert!(
            written.contains(PIPER_EN_KUSAL_Q8_FILE),
            "expected pack weights path, got {written}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stt_adapter_rejects_piper_kind() {
        let config = AudioRunnerConfig {
            command: "piper".into(),
            args: Vec::new(),
            adapter: AudioSttAdapterKind::Piper,
            model: None,
        };
        let err = match stt_adapter_from_runner(config) {
            Ok(_) => panic!("expected piper STT adapter to be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("piper") && err.contains("not supported"),
            "unexpected: {err}"
        );
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

    #[test]
    fn persist_and_list_stt_and_tts_artifacts() {
        let dir = tempfile_dir("artifacts-list");
        let source = dir.join("in.wav");
        fs::write(&source, b"RIFF").unwrap();
        let stt = persist_stt_artifact(&dir, "spoken words", &source).unwrap();
        assert_eq!(stt.kind, AudioArtifactKind::SttTranscript);
        assert!(Path::new(&stt.path).is_file());
        assert_eq!(fs::read_to_string(&stt.path).unwrap(), "spoken words");
        assert_eq!(stt.text_preview.as_deref(), Some("spoken words"));

        let out = dir.join("spoken.wav");
        fs::write(&out, b"RIFFDATA").unwrap();
        let tts = persist_tts_artifact(&dir, "say this", &out).unwrap();
        assert_eq!(tts.kind, AudioArtifactKind::TtsOutput);
        assert!(Path::new(&tts.path).is_file());
        assert_eq!(fs::read(&tts.path).unwrap(), b"RIFFDATA");
        assert_ne!(tts.path, out.to_string_lossy());

        let listed = list_audio_artifacts(&dir);
        assert_eq!(listed.len(), 2);
        assert!(listed
            .iter()
            .any(|a| a.id == stt.id && a.kind == AudioArtifactKind::SttTranscript));
        assert!(listed
            .iter()
            .any(|a| a.id == tts.id && a.kind == AudioArtifactKind::TtsOutput));

        // Newest-first ordering when timestamps differ.
        let older = AudioArtifact {
            created_at: 100,
            ..stt.clone()
        };
        let newer = AudioArtifact {
            created_at: 200,
            ..tts.clone()
        };
        write_artifact_meta(&audio_artifacts_dir(&dir), &older).unwrap();
        write_artifact_meta(&audio_artifacts_dir(&dir), &newer).unwrap();
        let ordered = list_audio_artifacts(&dir);
        assert_eq!(ordered[0].id, newer.id);
        assert_eq!(ordered[1].id, older.id);
    }

    #[test]
    fn list_audio_artifacts_empty_when_dir_missing() {
        let dir = tempfile_dir("artifacts-missing");
        let _ = fs::remove_dir_all(dir.join("audio-artifacts"));
        assert!(list_audio_artifacts(&dir).is_empty());
    }

    #[test]
    fn clear_audio_artifacts_removes_dir() {
        let dir = tempfile_dir("artifacts-clear");
        let source = dir.join("clip.wav");
        fs::write(&source, b"x").unwrap();
        persist_stt_artifact(&dir, "bye", &source).unwrap();
        assert_eq!(list_audio_artifacts(&dir).len(), 1);
        clear_audio_artifacts(&dir).unwrap();
        assert!(!audio_artifacts_dir(&dir).exists());
        assert!(list_audio_artifacts(&dir).is_empty());
        // Clearing again is a no-op.
        clear_audio_artifacts(&dir).unwrap();
    }

    #[test]
    fn artifact_text_preview_truncates() {
        let dir = tempfile_dir("artifacts-preview");
        let long = "a".repeat(ARTIFACT_PREVIEW_CHARS + 40);
        let source = dir.join("a.wav");
        fs::write(&source, b"x").unwrap();
        let art = persist_stt_artifact(&dir, &long, &source).unwrap();
        let preview = art.text_preview.expect("preview");
        assert!(preview.ends_with('…'));
        assert_eq!(preview.chars().count(), ARTIFACT_PREVIEW_CHARS + 1);
    }

    fn tempfile_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "keel-audio-{}-{}-{}",
            label,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
