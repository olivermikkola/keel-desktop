//! Curated, checksum-verified runtime installation. No shell installer is executed.
use crate::models::{CatalogModel, DownloadProgress};
use crate::runtime::check_runner_starts;
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
    /// A smaller, quicker download offered when the recommendation is the larger model.
    pub fast_model: Option<CatalogModel>,
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
    let runtime_asset = catalog.assets.get(&platform).cloned();
    // Guided setup owns its runner on supported systems. An existing server is
    // an explicit choice and an arbitrary PATH binary cannot block setup.
    let runtime_path = installed_runtime(data).or_else(|| {
        if runtime_asset.is_none() {
            explicit.map(PathBuf::from).filter(|path| path.is_file())
        } else {
            None
        }
    });
    let recommended = if memory >= 12 * 1024 * 1024 * 1024 {
        "bonsai-9b"
    } else {
        "bonsai-4b"
    };
    let find = |id: &str| {
        crate::models::default_catalog()
            .into_iter()
            .find(|m| m.id == id)
    };
    SetupPlan {
        memory_bytes: memory,
        platform: platform.clone(),
        runtime_asset,
        runtime_path: runtime_path.map(|p| p.to_string_lossy().into_owned()),
        recommended_model: find(recommended).unwrap(),
        fast_model: if recommended == "bonsai-4b" {
            None
        } else {
            find("bonsai-4b")
        },
        context_size: 8192,
    }
}

/// Picks the model guided setup downloads: the recommendation, or the smaller
/// option when the person asked for it. Anything else is rejected so the
/// first-run screen can only install curated, checksum-verified models.
pub fn resolve_model(plan: &SetupPlan, requested: Option<&str>) -> Result<CatalogModel> {
    match requested {
        None => Ok(plan.recommended_model.clone()),
        Some(id) if id == plan.recommended_model.id => Ok(plan.recommended_model.clone()),
        Some(id) => match &plan.fast_model {
            Some(fast) if fast.id == id => Ok(fast.clone()),
            _ => bail!("`{id}` is not offered by guided setup on this computer"),
        },
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
    if let Some(path) = installed_runtime(data).filter(|path| check_runner_starts(path).is_ok()) {
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
    let prepare = unpack_runtime(&archive, &temporary).and_then(|()| {
        let runner =
            locate_server(&temporary, 4).context("Runtime package did not contain llama-server")?;
        check_runner_starts(&runner)
    });
    if let Err(error) = prepare {
        let _ = std::fs::remove_dir_all(&temporary);
        return Err(error);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_for(recommended: &str) -> SetupPlan {
        let find = |id: &str| {
            crate::models::default_catalog()
                .into_iter()
                .find(|m| m.id == id)
                .unwrap()
        };
        SetupPlan {
            memory_bytes: 16 << 30,
            platform: "test".into(),
            runtime_asset: None,
            runtime_path: None,
            recommended_model: find(recommended),
            fast_model: (recommended != "bonsai-4b").then(|| find("bonsai-4b")),
            context_size: 8192,
        }
    }

    #[test]
    fn guided_setup_defaults_to_the_recommendation() {
        let plan = plan_for("bonsai-9b");
        assert_eq!(resolve_model(&plan, None).unwrap().id, "bonsai-9b");
        assert_eq!(
            resolve_model(&plan, Some("bonsai-9b")).unwrap().id,
            "bonsai-9b"
        );
    }

    #[test]
    fn guided_setup_offers_the_smaller_download_on_big_machines_only() {
        let big = plan_for("bonsai-9b");
        let fast = resolve_model(&big, Some("bonsai-4b")).unwrap();
        assert!(fast.bytes < big.recommended_model.bytes);
        assert!(plan_for("bonsai-4b").fast_model.is_none());
    }

    #[test]
    fn guided_setup_rejects_models_it_did_not_offer() {
        let plan = plan_for("bonsai-9b");
        assert!(resolve_model(&plan, Some("bonsai-27b")).is_err());
        assert!(resolve_model(&plan, Some("../../etc/passwd")).is_err());
        assert!(resolve_model(&plan_for("bonsai-4b"), Some("lfm3-1.7b")).is_err());
    }

    #[test]
    fn real_plan_matches_this_computers_memory() {
        let dir = std::env::temp_dir().join(format!("keel-plan-{}", uuid::Uuid::new_v4()));
        let plan = plan(&dir, None);
        let offered_fast = plan.fast_model.is_some();
        assert_eq!(offered_fast, plan.recommended_model.id != "bonsai-4b");
    }

    #[test]
    fn guided_setup_prefers_the_managed_runner_over_an_explicit_path() {
        let dir = std::env::temp_dir().join(format!("keel-plan-{}", uuid::Uuid::new_v4()));
        let explicit = std::env::current_exe().unwrap();
        let setup = plan(&dir, explicit.to_str());
        if setup.runtime_asset.is_some() {
            assert!(setup.runtime_path.is_none());
        } else {
            assert_eq!(setup.runtime_path.as_deref(), explicit.to_str());
        }
    }
}
