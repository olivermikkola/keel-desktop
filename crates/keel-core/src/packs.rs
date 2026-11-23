//! Capability pack manifests, VRAM placement, and checksummed install/remove.
//!
//! Packs live under `{data_dir}/packs/<capability_id>/`. Downloads reuse the
//! shared SHA-256 verifier from [`crate::models::download_gguf`].

use crate::hardware::VramInventory;
use crate::models::{download_gguf, DownloadProgress};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Which runner a pack expects. Text reuses llama-server; audio packs will
/// bring their own adapters later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerKind {
    Text,
    AudioStt,
    AudioTts,
}

/// Skeleton capability pack used for VRAM placement decisions.
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

/// Catalog entry with optional checksummed download metadata for Settings UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPack {
    pub capability_id: String,
    pub display_name: String,
    pub detail: String,
    pub min_vram_mib: u64,
    pub runner_kind: RunnerKind,
    pub download_url: Option<String>,
    pub sha256: Option<String>,
    pub file_name: String,
    pub bytes: u64,
    pub installed: bool,
}

impl CapabilityPack {
    pub fn to_manifest(&self) -> PackManifest {
        PackManifest::new(
            self.capability_id.clone(),
            self.min_vram_mib,
            self.runner_kind,
        )
    }
}

/// Demo pack artifact checked into `fixtures/packs/demo-capability.pack`.
pub const DEMO_PACK_ID: &str = "demo.capability";
pub const DEMO_PACK_FILE: &str = "demo-capability.pack";
pub const DEMO_PACK_SHA256: &str =
    "8d687b16b151eaca49c555232b5fecb541f24da00fa195ab3377c11246f0b5b2";
pub const DEMO_PACK_BYTES: u64 = 67;

/// Raw GitHub URL for the demo fixture on this pillar branch (checksum-pinned).
pub fn demo_pack_download_url() -> String {
    format!(
        "https://raw.githubusercontent.com/olivermikkola/keel-desktop/cursor/v06-vram-inventory-adf8/crates/keel-core/fixtures/packs/{DEMO_PACK_FILE}"
    )
}

/// Built-in capability pack catalog (downloadable demo + future audio stubs).
pub fn default_pack_catalog() -> Vec<CapabilityPack> {
    vec![
        CapabilityPack {
            capability_id: DEMO_PACK_ID.into(),
            display_name: "Demo capability pack".into(),
            detail: "Tiny checksummed fixture used to exercise download and remove.".into(),
            min_vram_mib: 0,
            runner_kind: RunnerKind::Text,
            download_url: Some(demo_pack_download_url()),
            sha256: Some(DEMO_PACK_SHA256.into()),
            file_name: DEMO_PACK_FILE.into(),
            bytes: DEMO_PACK_BYTES,
            installed: false,
        },
        CapabilityPack {
            capability_id: "audio.stt".into(),
            display_name: "Speech to text (stub)".into(),
            detail: "Whisper-class STT pack will download here once a runner is pinned.".into(),
            min_vram_mib: 1024,
            runner_kind: RunnerKind::AudioStt,
            download_url: None,
            sha256: None,
            file_name: "audio-stt.pack".into(),
            bytes: 0,
            installed: false,
        },
        CapabilityPack {
            capability_id: "audio.tts".into(),
            display_name: "Text to speech (stub)".into(),
            detail: "Local TTS pack will download here once a runner is pinned.".into(),
            min_vram_mib: 512,
            runner_kind: RunnerKind::AudioTts,
            download_url: None,
            sha256: None,
            file_name: "audio-tts.pack".into(),
            bytes: 0,
            installed: false,
        },
    ]
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

/// Root directory for installed capability packs.
pub fn packs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("packs")
}

fn validate_pack_id(capability_id: &str) -> Result<()> {
    let id = capability_id.trim();
    if id.is_empty() {
        bail!("Pack id is empty");
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") || id.contains('\0') {
        bail!("Invalid pack id `{id}`");
    }
    Ok(())
}

fn validate_file_name(file_name: &str) -> Result<()> {
    if file_name.is_empty()
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains("..")
        || file_name.contains('\0')
    {
        bail!("Invalid pack file name `{file_name}`");
    }
    Ok(())
}

/// Directory for one installed pack.
pub fn pack_install_dir(data_dir: &Path, capability_id: &str) -> Result<PathBuf> {
    validate_pack_id(capability_id)?;
    Ok(packs_dir(data_dir).join(capability_id.trim()))
}

fn installed_marker(install_dir: &Path) -> PathBuf {
    install_dir.join(".installed")
}

/// True when the pack directory has an install marker (and optional artifact).
pub fn is_pack_installed(data_dir: &Path, capability_id: &str) -> bool {
    match pack_install_dir(data_dir, capability_id) {
        Ok(dir) => installed_marker(&dir).is_file(),
        Err(_) => false,
    }
}

pub fn find_catalog_pack(capability_id: &str) -> Option<CapabilityPack> {
    default_pack_catalog()
        .into_iter()
        .find(|pack| pack.capability_id == capability_id)
}

/// Catalog with `installed` reflecting on-disk markers under `data_dir`.
pub fn list_capability_packs(data_dir: &Path) -> Vec<CapabilityPack> {
    default_pack_catalog()
        .into_iter()
        .map(|mut pack| {
            pack.installed = is_pack_installed(data_dir, &pack.capability_id);
            pack
        })
        .collect()
}

/// Write bytes into the pack directory after verifying SHA-256. Used by tests
/// and by the download path after the network transfer finishes.
pub fn place_pack_bytes(data_dir: &Path, pack: &CapabilityPack, bytes: &[u8]) -> Result<PathBuf> {
    validate_pack_id(&pack.capability_id)?;
    validate_file_name(&pack.file_name)?;
    if let Some(expected) = pack.sha256.as_deref() {
        let actual = hex::encode(Sha256::digest(bytes));
        if !actual.eq_ignore_ascii_case(expected) {
            bail!(
                "SHA-256 mismatch for {}: expected {}, got {}",
                pack.capability_id,
                expected,
                actual
            );
        }
    }
    let dir = pack_install_dir(data_dir, &pack.capability_id)?;
    fs::create_dir_all(&dir).with_context(|| format!("create pack dir {}", dir.display()))?;
    let dest = dir.join(&pack.file_name);
    let partial = dest.with_extension(format!(
        "{}.part",
        dest.extension().and_then(|e| e.to_str()).unwrap_or("bin")
    ));
    fs::write(&partial, bytes).with_context(|| format!("write {}", partial.display()))?;
    fs::rename(&partial, &dest).with_context(|| format!("rename into {}", dest.display()))?;
    let manifest = serde_json::json!({
        "capability_id": pack.capability_id,
        "display_name": pack.display_name,
        "min_vram_mib": pack.min_vram_mib,
        "runner_kind": pack.runner_kind,
        "file_name": pack.file_name,
        "sha256": pack.sha256,
        "bytes": bytes.len() as u64,
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    fs::write(installed_marker(&dir), b"ok\n")?;
    Ok(dest)
}

/// Remove an installed pack directory (artifact, manifest, marker).
pub fn remove_capability_pack(data_dir: &Path, capability_id: &str) -> Result<()> {
    let dir = pack_install_dir(data_dir, capability_id)?;
    if !dir.exists() {
        bail!("No installed pack found for `{capability_id}`");
    }
    fs::remove_dir_all(&dir).with_context(|| format!("remove {}", dir.display()))?;
    Ok(())
}

/// Download a catalog pack to `{data_dir}/packs/<id>/` with checksum verification.
pub async fn download_capability_pack<F>(
    data_dir: &Path,
    capability_id: &str,
    on_progress: F,
) -> Result<CapabilityPack>
where
    F: FnMut(DownloadProgress),
{
    let pack = find_catalog_pack(capability_id)
        .with_context(|| format!("Unknown capability pack: {capability_id}"))?;
    download_capability_pack_entry(data_dir, &pack, on_progress).await
}

/// Download using an explicit catalog entry (tests may override `download_url`).
pub async fn download_capability_pack_entry<F>(
    data_dir: &Path,
    pack: &CapabilityPack,
    on_progress: F,
) -> Result<CapabilityPack>
where
    F: FnMut(DownloadProgress),
{
    let url = pack
        .download_url
        .as_deref()
        .context("This pack has no download URL yet")?;
    validate_pack_id(&pack.capability_id)?;
    validate_file_name(&pack.file_name)?;
    let dir = pack_install_dir(data_dir, &pack.capability_id)?;
    fs::create_dir_all(&dir)?;
    let dest = dir.join(&pack.file_name);
    if dest.is_file() {
        let _ = fs::remove_file(&dest);
    }

    download_gguf(
        url,
        &dest,
        pack.sha256.as_deref(),
        &pack.capability_id,
        on_progress,
    )
    .await?;

    let bytes = fs::read(&dest).with_context(|| format!("read {}", dest.display()))?;
    // Re-place so manifest + .installed marker are written consistently.
    let _ = fs::remove_file(&dest);
    place_pack_bytes(data_dir, pack, &bytes)?;
    let mut installed = pack.clone();
    installed.installed = true;
    Ok(installed)
}

/// Demo fixture bytes (matches [`DEMO_PACK_SHA256`]).
pub fn demo_pack_bytes() -> &'static [u8] {
    b"{\"keel_pack\":\"demo\",\"capability_id\":\"demo.capability\",\"version\":1}\n"
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

    fn temp_data(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "keel-packs-{}-{}-{}",
            label,
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
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

    #[test]
    fn demo_fixture_matches_pinned_checksum() {
        let digest = hex::encode(Sha256::digest(demo_pack_bytes()));
        assert_eq!(digest, DEMO_PACK_SHA256);
        assert_eq!(demo_pack_bytes().len() as u64, DEMO_PACK_BYTES);
        let on_disk = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/packs")
            .join(DEMO_PACK_FILE);
        let mut bytes = fs::read(&on_disk).unwrap();
        // Without binary .gitattributes, Windows checkouts may use CRLF.
        if bytes.ends_with(b"\r\n") {
            bytes.pop();
            bytes.pop();
            bytes.push(b'\n');
        }
        assert_eq!(bytes.as_slice(), demo_pack_bytes());
    }

    #[test]
    fn place_and_remove_demo_pack_under_packs_dir() {
        let data = temp_data("place-remove");
        let pack = find_catalog_pack(DEMO_PACK_ID).expect("demo pack");
        assert!(!is_pack_installed(&data, DEMO_PACK_ID));

        let dest = place_pack_bytes(&data, &pack, demo_pack_bytes()).unwrap();
        assert!(dest.is_file());
        assert!(is_pack_installed(&data, DEMO_PACK_ID));
        let listed = list_capability_packs(&data);
        let demo = listed
            .iter()
            .find(|p| p.capability_id == DEMO_PACK_ID)
            .unwrap();
        assert!(demo.installed);

        remove_capability_pack(&data, DEMO_PACK_ID).unwrap();
        assert!(!is_pack_installed(&data, DEMO_PACK_ID));
        assert!(!pack_install_dir(&data, DEMO_PACK_ID).unwrap().exists());
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn place_rejects_checksum_mismatch() {
        let data = temp_data("bad-checksum");
        let pack = find_catalog_pack(DEMO_PACK_ID).unwrap();
        let err = place_pack_bytes(&data, &pack, b"not-the-demo-bytes")
            .unwrap_err()
            .to_string();
        assert!(err.contains("SHA-256 mismatch"), "{err}");
        assert!(!is_pack_installed(&data, DEMO_PACK_ID));
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn remove_errors_when_pack_missing() {
        let data = temp_data("missing-remove");
        let err = remove_capability_pack(&data, DEMO_PACK_ID)
            .unwrap_err()
            .to_string();
        assert!(err.contains("No installed pack"), "{err}");
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn rejects_path_traversal_pack_ids() {
        let data = temp_data("traversal");
        assert!(pack_install_dir(&data, "../escape").is_err());
        assert!(pack_install_dir(&data, "a/b").is_err());
        let _ = fs::remove_dir_all(&data);
    }

    #[tokio::test]
    async fn download_capability_pack_verifies_checksum_over_http() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let data = temp_data("download-http");
        let body = demo_pack_bytes();
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

        let mut pack = find_catalog_pack(DEMO_PACK_ID).unwrap();
        pack.download_url = Some(format!("http://{addr}/demo-capability.pack"));

        let installed = download_capability_pack_entry(&data, &pack, |_| {})
            .await
            .unwrap();
        server.await.unwrap();
        assert!(installed.installed);
        assert!(is_pack_installed(&data, DEMO_PACK_ID));
        remove_capability_pack(&data, DEMO_PACK_ID).unwrap();
        let _ = fs::remove_dir_all(&data);
    }
}
