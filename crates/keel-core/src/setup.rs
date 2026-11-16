//! Curated, checksum-verified runtime installation. No shell installer is executed.
use crate::models::{CatalogModel, DownloadProgress};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAsset {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Deserialize)]
struct RuntimeCatalog {
    version: String,
    assets: HashMap<String, RuntimeAsset>,
}
#[derive(Debug, Clone, Serialize)]
pub struct SetupPlan {
    pub memory_bytes: u64,
    pub platform: String,
    pub runtime_asset: Option<RuntimeAsset>,
    pub runtime_path: Option<String>,
    pub recommended_model: CatalogModel,
    pub context_size: u32,
}

fn catalog() -> RuntimeCatalog {
    serde_json::from_str(include_str!("runtime-catalog.json")).expect("bundled runtime catalog")
}

pub fn plan(data: &Path, explicit: Option<&str>) -> SetupPlan {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    let memory = system.total_memory();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let catalog = catalog();
    let recommended = if memory >= 12 * 1024 * 1024 * 1024 {
        "bonsai-9b"
    } else {
        "bonsai-4b"
    };
    SetupPlan {
        memory_bytes: memory,
        platform: platform.clone(),
        runtime_asset: catalog.assets.get(&platform).cloned(),
        runtime_path: crate::runtime::find_llama_server(explicit)
            .or_else(|| installed_runtime(data))
            .map(|p| p.to_string_lossy().into_owned()),
        recommended_model: crate::models::default_catalog()
            .into_iter()
            .find(|m| m.id == recommended)
            .unwrap(),
        context_size: 8192,
    }
}

pub fn installed_runtime(data: &Path) -> Option<PathBuf> {
    locate_server(&data.join("runtime").join(catalog().version), 4)
}

fn locate_server(root: &Path, depth: usize) -> Option<PathBuf> {
    let executable = if cfg!(target_os = "windows") {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    if root.join(executable).is_file() {
        return Some(root.join(executable));
    }
    if depth == 0 {
        return None;
    }
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        if entry.file_type().ok()?.is_dir() {
            if let Some(path) = locate_server(&entry.path(), depth - 1) {
                return Some(path);
            }
        }
    }
    None
}

pub async fn install_runtime<F>(data: &Path, mut progress: F) -> Result<PathBuf>
where
    F: FnMut(DownloadProgress),
{
    if let Some(path) = installed_runtime(data) {
        return Ok(path);
    }
    let catalog = catalog();
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let asset = catalog.assets.get(&platform).context("Automatic setup is not available for this platform yet. Install llama.cpp, or connect an existing model server in Models.")?;
    let runtime_root = data.join("runtime");
    std::fs::create_dir_all(&runtime_root)?;
    let archive = runtime_root.join(&asset.name);
    crate::models::download_gguf(
        &asset.url,
        &archive,
        Some(&asset.sha256),
        "runtime",
        &mut progress,
    )
    .await?;
    let temporary = runtime_root.join(format!("install-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temporary)?;
    let unpack = unpack_runtime(&archive, &temporary);
    if let Err(error) = unpack {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(error);
    }
    locate_server(&temporary, 4).context("Runtime package did not contain llama-server")?;
    let destination = runtime_root.join(catalog.version);
    if destination.exists() {
        std::fs::remove_dir_all(&destination)?;
    }
    std::fs::rename(&temporary, &destination)?;
    let _ = std::fs::remove_file(archive);
    installed_runtime(data).context("Could not locate the installed model runner")
}

pub fn unpack_runtime(archive: &Path, destination: &Path) -> Result<()> {
    if archive.extension().and_then(|e| e.to_str()) == Some("zip") {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)?;
        let mut total = 0u64;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            let name = file
                .enclosed_name()
                .context("Invalid path in runtime package")?;
            if file.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                bail!("Unexpected symbolic link in ZIP runtime");
            }
            total += file.size();
            if total > 512 * 1024 * 1024 {
                bail!("Runtime package is too large");
            }
            let path = destination.join(name);
            if file.is_dir() {
                std::fs::create_dir_all(path)?;
            } else {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::io::copy(&mut file, &mut std::fs::File::create(path)?)?;
            }
        }
    } else {
        let decoder = flate2::read::GzDecoder::new(std::fs::File::open(archive)?);
        let mut tar = tar::Archive::new(decoder);
        let mut total = 0u64;
        for entry in tar.entries()? {
            let mut entry = entry?;
            total += entry.size();
            if total > 512 * 1024 * 1024 {
                bail!("Runtime package is too large");
            }
            if !entry.unpack_in(destination)? {
                bail!("Invalid path in runtime package");
            }
        }
    }
    Ok(())
}
