//! Minimal model-pack manifests for future audio/text capability packs.
//!
//! Full download/remove UI and measured hardware profiles come later; this
//! module only defines the shared shape and VRAM-aware placement helpers.

use crate::hardware::VramInventory;
use serde::{Deserialize, Serialize};

/// Which runner a pack expects. Text reuses llama-server; audio packs will
/// bring their own adapters later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerKind {
    Text,
    AudioStt,
    AudioTts,
}

/// Skeleton capability pack: id, minimum free VRAM, and runner kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackManifest {
    pub capability_id: String,
    pub min_vram_mib: u64,
    pub runner_kind: RunnerKind,
}

impl PackManifest {
    pub fn new(
        capability_id: impl Into<String>,
        min_vram_mib: u64,
        runner_kind: RunnerKind,
    ) -> Self {
        Self {
            capability_id: capability_id.into(),
            min_vram_mib,
            runner_kind,
        }
    }
}

/// Parse a runner kind from UI / command input (`text`, `audio_stt`, `audio_tts`).
pub fn parse_runner_kind(value: &str) -> Result<RunnerKind, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "text" => Ok(RunnerKind::Text),
        "audio_stt" | "stt" => Ok(RunnerKind::AudioStt),
        "audio_tts" | "tts" => Ok(RunnerKind::AudioTts),
        other => Err(format!("Unknown runner kind: {other}")),
    }
}

/// Where a pack should run given the current VRAM inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackPlacement {
    Graphics {
        device_id: String,
        usable_free_mib: u64,
    },
    Processor {
        reason: String,
    },
}

impl PackPlacement {
    pub fn uses_graphics(&self) -> bool {
        matches!(self, Self::Graphics { .. })
    }
}

/// Prefer a graphics device with enough free VRAM; otherwise processor with a reason.
pub fn place_pack(inventory: &VramInventory, pack: &PackManifest) -> PackPlacement {
    let need = pack.min_vram_mib;
    if need == 0 {
        return PackPlacement::Processor {
            reason: format!(
                "Pack '{}' does not require graphics memory.",
                pack.capability_id
            ),
        };
    }
    match &inventory.preferred {
        Some(device) if inventory.has_free(need) => PackPlacement::Graphics {
            device_id: device.id.clone(),
            usable_free_mib: inventory.usable_free_mib,
        },
        Some(_) => PackPlacement::Processor {
            reason: format!(
                "Pack '{}' needs at least {need} MiB free graphics memory; {} MiB is free.",
                pack.capability_id, inventory.usable_free_mib
            ),
        },
        None => PackPlacement::Processor {
            reason: format!(
                "Pack '{}' needs graphics memory, but no device inventory is available.",
                pack.capability_id
            ),
        },
    }
}

/// True when a keep-alive text model on graphics should unload so this pack can fit.
pub fn should_unload_for_pack(
    inventory: &VramInventory,
    pack: &PackManifest,
    text_using_graphics: bool,
) -> bool {
    if !text_using_graphics || pack.min_vram_mib == 0 {
        return false;
    }
    !inventory.has_free(pack.min_vram_mib)
}

/// Parse a JSON pack manifest (one object). Used by tests and future installers.
pub fn parse_pack_manifest(json: &str) -> Result<PackManifest, String> {
    serde_json::from_str(json).map_err(|err| format!("Invalid pack manifest: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::{DeviceKind, GraphicsDevice, InventorySource};

    fn inventory_with(free: u64, total: u64) -> VramInventory {
        let device = GraphicsDevice {
            id: "CUDA0".into(),
            name: "NVIDIA".into(),
            total_mib: total,
            free_mib: free,
            kind: DeviceKind::Discrete,
        };
        VramInventory {
            usable_free_mib: device.usable_mib(),
            total_mib: total,
            kind: "discrete".into(),
            source: InventorySource::ListDevices,
            preferred: Some(device.clone()),
            devices: vec![device],
        }
    }

    #[test]
    fn parses_a_minimal_text_pack_manifest() {
        let pack = parse_pack_manifest(
            r#"{"capability_id":"text.default","min_vram_mib":2048,"runner_kind":"text"}"#,
        )
        .expect("manifest");
        assert_eq!(pack.capability_id, "text.default");
        assert_eq!(pack.min_vram_mib, 2048);
        assert_eq!(pack.runner_kind, RunnerKind::Text);
    }

    #[test]
    fn parses_audio_runner_kinds() {
        let stt = parse_pack_manifest(
            r#"{"capability_id":"audio.stt","min_vram_mib":1024,"runner_kind":"audio_stt"}"#,
        )
        .unwrap();
        assert_eq!(stt.runner_kind, RunnerKind::AudioStt);
        let tts = parse_pack_manifest(
            r#"{"capability_id":"audio.tts","min_vram_mib":512,"runner_kind":"audio_tts"}"#,
        )
        .unwrap();
        assert_eq!(tts.runner_kind, RunnerKind::AudioTts);
    }

    #[test]
    fn places_on_graphics_when_free_vram_covers_the_pack() {
        let inv = inventory_with(6000, 8192);
        let pack = PackManifest::new("audio.stt", 1024, RunnerKind::AudioStt);
        match place_pack(&inv, &pack) {
            PackPlacement::Graphics {
                device_id,
                usable_free_mib,
            } => {
                assert_eq!(device_id, "CUDA0");
                assert_eq!(usable_free_mib, 6000);
            }
            other => panic!("expected graphics, got {other:?}"),
        }
    }

    #[test]
    fn falls_back_to_processor_with_an_explicit_reason_when_vram_is_short() {
        let inv = inventory_with(400, 8192);
        let pack = PackManifest::new("audio.stt", 2048, RunnerKind::AudioStt);
        match place_pack(&inv, &pack) {
            PackPlacement::Processor { reason } => {
                assert!(reason.contains("2048"));
                assert!(reason.contains("400"));
                assert!(reason.contains("audio.stt"));
            }
            other => panic!("expected processor, got {other:?}"),
        }
    }

    #[test]
    fn unload_only_when_keep_alive_blocks_a_vram_hungry_pack() {
        let tight = inventory_with(800, 8192);
        let pack = PackManifest::new("audio.stt", 2048, RunnerKind::AudioStt);
        assert!(should_unload_for_pack(&tight, &pack, true));
        assert!(!should_unload_for_pack(&tight, &pack, false));
        let roomy = inventory_with(4096, 8192);
        assert!(!should_unload_for_pack(&roomy, &pack, true));
        let cpu_pack = PackManifest::new("text.cpu", 0, RunnerKind::Text);
        assert!(!should_unload_for_pack(&tight, &cpu_pack, true));
    }

    #[test]
    fn parses_runner_kind_aliases() {
        assert_eq!(parse_runner_kind("text").unwrap(), RunnerKind::Text);
        assert_eq!(
            parse_runner_kind("audio_stt").unwrap(),
            RunnerKind::AudioStt
        );
        assert_eq!(parse_runner_kind("stt").unwrap(), RunnerKind::AudioStt);
        assert_eq!(parse_runner_kind("TTS").unwrap(), RunnerKind::AudioTts);
        assert!(parse_runner_kind("bogus").is_err());
    }
}
