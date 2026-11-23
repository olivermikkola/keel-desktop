//! Skeleton speech-to-text and text-to-speech capability packs.
//!
//! Catalog entries are stubs: nothing is downloaded or runnable yet. Full
//! Whisper/TTS adapters, resumable downloads, and audio artifacts land later.
//! Remote text-provider auth is a separate pillar (not wired here).

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Capability ids exposed by the audio pack catalog.
pub const CAPABILITY_STT: &str = "stt";
pub const CAPABILITY_TTS: &str = "tts";

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
/// `download_url` is a placeholder (`None` until a real pack is pinned).
/// Stub catalog entries report `installed: false`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioPackManifest {
    pub id: String,
    pub capability: AudioCapability,
    pub display_name: String,
    pub min_vram_mib: u64,
    pub runner_kind: AudioRunnerKind,
    pub download_url: Option<String>,
    pub installed: bool,
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
        },
        AudioPackManifest {
            id: "tts-local-en".into(),
            capability: AudioCapability::Tts,
            display_name: "Text to speech · English (stub)".into(),
            min_vram_mib: 512,
            runner_kind: AudioRunnerKind::AudioTts,
            download_url: None,
            installed: false,
        },
    ]
}

pub fn find_audio_pack(pack_id: &str) -> Option<AudioPackManifest> {
    list_audio_packs()
        .into_iter()
        .find(|pack| pack.id == pack_id)
}

fn audio_packs_dir(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("audio-packs")
}

/// True only when an install marker exists under the data directory.
/// Stub catalog entries never create this marker.
pub fn is_audio_pack_installed(data_dir: &Path, pack_id: &str) -> bool {
    audio_packs_dir(data_dir)
        .join(pack_id)
        .join(".installed")
        .is_file()
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
            return Ok(AudioPackManifest {
                installed: true,
                ..pack
            });
        }
    }
    bail!(
        "No {} pack is installed. Download an audio pack from Settings before using {}.",
        capability.label(),
        capability.label()
    );
}

/// Speech-to-text entry point. Fails clearly until a pack is installed (and a
/// real runner lands later).
pub fn run_stt(data_dir: &Path, _audio_path: &Path) -> Result<String> {
    let pack = require_installed_audio_pack(data_dir, AudioCapability::Stt)?;
    bail!(
        "Speech-to-text pack '{}' is marked installed, but no STT runner is wired yet.",
        pack.id
    );
}

/// Text-to-speech entry point. Fails clearly until a pack is installed (and a
/// real runner lands later).
pub fn run_tts(data_dir: &Path, _text: &str, _output_path: &Path) -> Result<()> {
    let pack = require_installed_audio_pack(data_dir, AudioCapability::Tts)?;
    bail!(
        "Text-to-speech pack '{}' is marked installed, but no TTS runner is wired yet.",
        pack.id
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

        let pack_dir = audio_packs_dir(&dir).join("stt-whisper-base");
        fs::create_dir_all(&pack_dir).unwrap();
        fs::write(pack_dir.join(".installed"), b"stub").unwrap();
        let pack = require_installed_audio_pack(&dir, AudioCapability::Stt).unwrap();
        assert_eq!(pack.id, "stt-whisper-base");
        assert!(pack.installed);
    }

    fn tempfile_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("keel-audio-{}-{}", label, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
