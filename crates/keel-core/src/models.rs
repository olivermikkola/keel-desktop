use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::future::{pending, Future};
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::watch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub file_name: String,
    pub revision: String,
    pub bytes: u64,
    pub sha256: Option<String>,
    pub quantization: String,
    pub context_size: u32,
    pub tier: String,
    pub detail: String,
    pub local_path: Option<String>,
    pub installed: bool,
    #[serde(default)]
    pub damaged: bool,
    #[serde(default)]
    pub damage_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub done: bool,
    pub error: Option<String>,
}

/// Pinned model catalog carried over from Keel Mobile, plus Desktop-tier options.
pub fn default_catalog() -> Vec<CatalogModel> {
    vec![
        CatalogModel {
            id: "bonsai-4b".to_string(),
            name: "Bonsai 2 · 4B".to_string(),
            repository: "prism-ml/Ternary-Bonsai-2-4B-gguf".to_string(),
            file_name: "Ternary-Bonsai-2-4B-Q2_0.gguf".to_string(),
            revision: "50ebba61be85e5be07e8083db468ace0fc1bac43".to_string(),
            bytes: 1_212_103_520,
            sha256: Some(
                "64a5183b44921bdb5ac4a6f5f92007821d6ebe5c66997d3c147fd214ab9433d4".to_string(),
            ),
            quantization: "Q2_0".to_string(),
            context_size: 16_384,
            tier: "fast".to_string(),
            detail: "Small and quick. A good start for most laptops.".to_string(),
            local_path: None,
            installed: false,
            damaged: false,
            damage_reason: None,
        },
        CatalogModel {
            id: "bonsai-9b".to_string(),
            name: "Bonsai 2 · 9B".to_string(),
            repository: "prism-ml/Ternary-Bonsai-2-9B-gguf".to_string(),
            file_name: "Ternary-Bonsai-2-9B-Q2_0.gguf".to_string(),
            revision: "82265ccc78ddaac4700286a35e9badcc32376dd9".to_string(),
            bytes: 2_556_640_128,
            sha256: Some(
                "1f884aa56aacd2bbbb2e4b7abaead2b1102b74a9c75a23fc33dab916813a384d".to_string(),
            ),
            quantization: "Q2_0".to_string(),
            context_size: 32_768,
            tier: "balanced".to_string(),
            detail: "Recommended. Smarter everyday helper; best with 12 GB of memory or more."
                .to_string(),
            local_path: None,
            installed: false,
            damaged: false,
            damage_reason: None,
        },
        CatalogModel {
            id: "lfm3-1.7b".to_string(),
            name: "LFM3 · 1.7B".to_string(),
            repository: "LiquidAI/LFM3-1.7B-GGUF".to_string(),
            file_name: "LFM3-1.7B-Q4_K_M.gguf".to_string(),
            revision: "cd0c74c575b5c416f3011b5b4c298077f26ca8c2".to_string(),
            bytes: 1_099_627_808,
            sha256: Some(
                "3793e1e6e2266919d6e015287221b41100175a98eddaba92e111aed3b05c93c4".to_string(),
            ),
            quantization: "Q4_K_M".to_string(),
            context_size: 8_192,
            tier: "fast".to_string(),
            detail: "Very small. For older computers with little memory.".to_string(),
            local_path: None,
            installed: false,
            damaged: false,
            damage_reason: None,
        },
        CatalogModel {
            id: "minicpm5-2b".to_string(),
            name: "MiniCPM5 · 2B".to_string(),
            repository: "openbmb/MiniCPM5-2B-GGUF".to_string(),
            file_name: "MiniCPM5-2B-Q4_K_M.gguf".to_string(),
            revision: "8ad02b786ab81e33d465e1c80562640c55ab2877".to_string(),
            bytes: 1_561_318_368,
            sha256: Some(
                "ec2d5801640099e97d8d7e8003ad4d81f336e757811f03a26173dddf386602fd".to_string(),
            ),
            quantization: "Q4_K_M".to_string(),
            context_size: 8_192,
            tier: "fast".to_string(),
            detail: "Small and light. Good for thin laptops.".to_string(),
            local_path: None,
            installed: false,
            damaged: false,
            damage_reason: None,
        },
        CatalogModel {
            id: "bonsai-27b".to_string(),
            name: "Bonsai 2 · 27B".to_string(),
            repository: "prism-ml/Ternary-Bonsai-2-27B-gguf".to_string(),
            file_name: "Ternary-Bonsai-2-27B-Q2_0.gguf".to_string(),
            revision: "main".to_string(),
            bytes: 7_630_000_000,
            sha256: None,
            quantization: "Q2_0".to_string(),
            context_size: 32_768,
            tier: "desktop".to_string(),
            detail: "Largest and smartest. Needs a powerful computer with 16 GB of memory or more."
                .to_string(),
            local_path: None,
            installed: false,
            damaged: false,
            damage_reason: None,
        },
    ]
}

pub fn models_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

/// Resolves the Hugging Face direct file URL for a catalog entry.
pub fn catalog_download_url(model: &CatalogModel) -> String {
    format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        model.repository, model.revision, model.file_name
    )
}

/// Validates and parses a pasted Hugging Face `.gguf` URL (`https://huggingface.co/<owner>/<repo>/resolve|blob/<rev>/<file.gguf>`).
pub fn parse_huggingface_gguf_url(raw: &str) -> Result<(String, String, String, String)> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("https://huggingface.co/")
        .context("URL must start with https://huggingface.co/")?;
    let clean = rest.split(['?', '#']).next().unwrap_or(rest);
    let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 5 || (parts[2] != "resolve" && parts[2] != "blob") {
        bail!("Expected a Hugging Face file URL like https://huggingface.co/<owner>/<repo>/resolve/main/<model>.gguf");
    }
    let repo = format!("{}/{}", parts[0], parts[1]);
    let revision = parts[3].to_string();
    let file_path = parts[4..].join("/");
    let file_name = parts.last().unwrap().to_string();
    if !file_name.to_ascii_lowercase().ends_with(".gguf") {
        bail!("URL must point to a .gguf file");
    }
    let resolve_url = format!("https://huggingface.co/{repo}/resolve/{revision}/{file_path}");
    Ok((repo, revision, file_name, resolve_url))
}

/// Fast integrity check for a local model file: verifies the GGUF header and,
/// for checksum-pinned catalog models, confirms the file wasn't truncated.
pub fn validate_gguf_file(
    path: &Path,
    expected_bytes: Option<u64>,
) -> std::result::Result<(), String> {
    use std::io::Read;
    let meta =
        std::fs::metadata(path).map_err(|_| "This model file could not be read.".to_string())?;
    if !meta.is_file() {
        return Err("This model file is missing.".to_string());
    }
    let len = meta.len();
    if len < 64 {
        return Err("This model file is incomplete or empty.".to_string());
    }
    if let Some(expected) = expected_bytes {
        if len != expected {
            return Err("This model file is incomplete or damaged.".to_string());
        }
    }
    let mut file = std::fs::File::open(path)
        .map_err(|_| "This model file could not be opened.".to_string())?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|_| "This model file is incomplete or unreadable.".to_string())?;
    if &magic != b"GGUF" {
        return Err("This file is damaged or not a valid local model.".to_string());
    }
    Ok(())
}

/// Removes a downloaded model file (and any partial download) or unlinks an imported path.
pub fn delete_model_file(
    data_dir: &Path,
    model_id: &str,
    custom_paths: &mut HashMap<String, String>,
) -> Result<()> {
    let dir = models_dir(data_dir);
    let mut removed_any = custom_paths.remove(model_id).is_some();

    let managed_name = default_catalog()
        .into_iter()
        .find(|m| m.id == model_id)
        .map(|m| m.file_name)
        .or_else(|| model_id.strip_prefix("local:").map(String::from));

    if let Some(file_name) = managed_name {
        if file_name.contains('/') || file_name.contains('\\') || file_name.contains("..") {
            bail!("Invalid model file name");
        }
        let target = dir.join(&file_name);
        let part = partial_download_path(&target);
        if target.is_file() {
            std::fs::remove_file(&target)?;
            removed_any = true;
        }
        if part.is_file() {
            let _ = std::fs::remove_file(&part);
            removed_any = true;
        }
    }

    if !removed_any {
        bail!("No downloaded file found for `{model_id}`");
    }
    Ok(())
}

/// Orders healthy installed models for automatic startup: preferred model first,
/// then safer/lighter models before large desktop models so a broken or heavy
/// model never leaves the user stuck.
pub fn candidate_models_for_auto_start(
    models: &[CatalogModel],
    preferred_id: &str,
) -> Vec<CatalogModel> {
    let mut healthy: Vec<CatalogModel> = models
        .iter()
        .filter(|m| m.installed && !m.damaged && m.local_path.is_some())
        .cloned()
        .collect();
    let tier_rank = |tier: &str| match tier {
        "fast" => 0u8,
        "balanced" => 1,
        "custom" => 2,
        "desktop" => 3,
        _ => 4,
    };
    healthy.sort_by_key(|m| {
        let is_preferred = u8::from(m.id != preferred_id);
        (is_preferred, tier_rank(&m.tier), m.bytes)
    });
    healthy
}

/// Scans `models_dir` plus any custom imported file paths and returns the merged list.
pub fn list_models(data_dir: &Path, custom_paths: &HashMap<String, String>) -> Vec<CatalogModel> {
    let dir = models_dir(data_dir);
    let mut models = default_catalog();

    for m in &mut models {
        let expected_bytes = m.sha256.is_some().then_some(m.bytes);
        let managed = dir.join(&m.file_name);
        let candidate = if managed.is_file() {
            Some(managed)
        } else {
            custom_paths
                .get(&m.id)
                .map(PathBuf::from)
                .filter(|p| p.is_file())
        };
        if let Some(path) = candidate {
            m.local_path = Some(path.to_string_lossy().to_string());
            match validate_gguf_file(&path, expected_bytes) {
                Ok(()) => {
                    m.installed = true;
                    m.damaged = false;
                    m.damage_reason = None;
                    if let Ok(meta) = std::fs::metadata(&path) {
                        m.bytes = meta.len();
                    }
                }
                Err(reason) => {
                    m.installed = false;
                    m.damaged = true;
                    m.damage_reason = Some(reason);
                }
            }
        }
    }

    // Include any extra .gguf files sitting in models_dir or custom_paths not in the pinned catalog.
    let known_files: Vec<String> = models.iter().map(|m| m.file_name.clone()).collect();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("gguf"))
            {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("model.gguf")
                    .to_string();
                if !known_files.contains(&file_name) {
                    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    let id = format!("local:{}", file_name);
                    let health = validate_gguf_file(&path, None);
                    models.push(CatalogModel {
                        id,
                        name: file_name.trim_end_matches(".gguf").to_string(),
                        repository: "local".to_string(),
                        file_name,
                        revision: "local".to_string(),
                        bytes,
                        sha256: None,
                        quantization: "GGUF".to_string(),
                        context_size: 16_384,
                        tier: "custom".to_string(),
                        detail: "Your own model file.".to_string(),
                        local_path: Some(path.to_string_lossy().to_string()),
                        installed: health.is_ok(),
                        damaged: health.is_err(),
                        damage_reason: health.err(),
                    });
                }
            }
        }
    }

    for (id, path_str) in custom_paths {
        if models.iter().any(|m| &m.id == id) {
            continue;
        }
        let path = Path::new(path_str);
        if path.is_file() {
            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("custom.gguf")
                .to_string();
            let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            let health = validate_gguf_file(path, None);
            models.push(CatalogModel {
                id: id.clone(),
                name: file_name.trim_end_matches(".gguf").to_string(),
                repository: "imported".to_string(),
                file_name,
                revision: "imported".to_string(),
                bytes,
                sha256: None,
                quantization: "GGUF".to_string(),
                context_size: 16_384,
                tier: "custom".to_string(),
                detail: format!("Linked local file: {}", path.display()),
                local_path: Some(path_str.clone()),
                installed: health.is_ok(),
                damaged: health.is_err(),
                damage_reason: health.err(),
            });
        }
    }

    models
}

/// Downloads a GGUF file into `dest_path`, verifying SHA-256 when provided.
pub async fn download_gguf<F>(
    url: &str,
    dest_path: &Path,
    expected_sha256: Option<&str>,
    model_id: &str,
    on_progress: F,
) -> Result<u64>
where
    F: FnMut(DownloadProgress),
{
    download_with_pause(url, dest_path, expected_sha256, model_id, None, on_progress).await
}

#[derive(Debug)]
pub struct DownloadPaused;

impl std::fmt::Display for DownloadPaused {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Setup download paused")
    }
}

impl std::error::Error for DownloadPaused {}

async fn wait_for_pause(pause: &mut Option<&mut watch::Receiver<bool>>) {
    if let Some(receiver) = pause {
        if receiver.wait_for(|paused| *paused).await.is_ok() {
            return;
        }
    }
    pending::<()>().await;
}

async fn until_paused<T>(
    pause: &mut Option<&mut watch::Receiver<bool>>,
    step: impl Future<Output = T>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = wait_for_pause(pause) => Err(DownloadPaused.into()),
        result = step => Ok(result),
    }
}

/// Pause network waits while finishing any in-flight disk write before returning.
pub async fn download_with_pause<F>(
    url: &str,
    dest_path: &Path,
    expected_sha256: Option<&str>,
    model_id: &str,
    mut pause: Option<&mut watch::Receiver<bool>>,
    mut on_progress: F,
) -> Result<u64>
where
    F: FnMut(DownloadProgress),
{
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let part_path = partial_download_path(dest_path);
    let client = reqwest::Client::builder()
        .user_agent(format!("keel-desktop/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .read_timeout(std::time::Duration::from_secs(120))
        .timeout(std::time::Duration::from_secs(3600))
        .build()?;
    // Only immutable, checksummed downloads may resume a partial file.
    let partial = if expected_sha256.is_some() {
        fs::metadata(&part_path).await.map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    let mut request = client.get(url);
    if partial > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={partial}-"));
    }
    let mut resp = until_paused(&mut pause, request.send()).await??;
    if resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
        resp = until_paused(&mut pause, client.get(url).send()).await??;
    }
    let resp = resp.error_for_status()?;
    let resume = partial > 0 && resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    if resume {
        let range = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if !range.starts_with(&format!("bytes {partial}-")) {
            bail!("Download server returned an unexpected resume range; try again");
        }
    }
    let mut downloaded = if resume { partial } else { 0 };
    let total = resp.content_length().map(|length| length + downloaded);
    let mut hasher = Sha256::new();
    if resume {
        let mut previous = fs::File::open(&part_path).await?;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let n = until_paused(&mut pause, previous.read(&mut buffer)).await??;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resume)
        .truncate(!resume)
        .open(&part_path)
        .await?;
    let mut stream = resp.bytes_stream();
    let mut last_reported = downloaded;
    on_progress(DownloadProgress {
        model_id: model_id.to_string(),
        downloaded_bytes: downloaded,
        total_bytes: total,
        done: false,
        error: None,
    });

    loop {
        let chunk = match until_paused(&mut pause, stream.next()).await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(paused) => {
                file.flush().await?;
                return Err(paused);
            }
        };
        let bytes = chunk?;
        file.write_all(&bytes).await?;
        hasher.update(&bytes);
        downloaded += bytes.len() as u64;
        if downloaded - last_reported >= 512 * 1024 {
            last_reported = downloaded;
            on_progress(DownloadProgress {
                model_id: model_id.to_string(),
                downloaded_bytes: downloaded,
                total_bytes: total,
                done: false,
                error: None,
            });
        }
    }

    file.flush().await?;
    drop(file);

    let actual_hex = hex::encode(hasher.finalize());
    if let Some(expected) = expected_sha256 {
        if !actual_hex.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&part_path).await;
            bail!(
                "SHA-256 mismatch for {}: expected {}, got {}",
                model_id,
                expected,
                actual_hex
            );
        }
    }

    fs::rename(&part_path, dest_path).await?;
    on_progress(DownloadProgress {
        model_id: model_id.to_string(),
        downloaded_bytes: downloaded,
        total_bytes: total.or(Some(downloaded)),
        done: true,
        error: None,
    });
    Ok(downloaded)
}

pub fn partial_download_path(destination: &Path) -> PathBuf {
    destination.with_extension("gguf.part")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_huggingface_blob_and_resolve_urls() {
        let (repo, rev, file, resolve) = parse_huggingface_gguf_url(
            "https://huggingface.co/prism-ml/Ternary-Bonsai-2-4B-gguf/blob/main/Ternary-Bonsai-2-4B-Q2_0.gguf?download=true",
        )
        .unwrap();
        assert_eq!(repo, "prism-ml/Ternary-Bonsai-2-4B-gguf");
        assert_eq!(rev, "main");
        assert_eq!(file, "Ternary-Bonsai-2-4B-Q2_0.gguf");
        assert_eq!(
            resolve,
            "https://huggingface.co/prism-ml/Ternary-Bonsai-2-4B-gguf/resolve/main/Ternary-Bonsai-2-4B-Q2_0.gguf"
        );
    }

    #[test]
    fn rejects_non_gguf_urls() {
        assert!(parse_huggingface_gguf_url(
            "https://huggingface.co/owner/repo/resolve/main/README.md"
        )
        .is_err());
        assert!(parse_huggingface_gguf_url("https://example.com/model.gguf").is_err());
    }

    #[test]
    fn list_models_detects_healthy_and_damaged_local_files() {
        let dir = std::env::temp_dir().join(format!("keel_models_test_{}", uuid::Uuid::new_v4()));
        let mdir = models_dir(&dir);
        std::fs::create_dir_all(&mdir).unwrap();

        // A valid custom GGUF starts with "GGUF" and is at least 64 bytes.
        let mut valid_gguf = b"GGUF".to_vec();
        valid_gguf.resize(128, 0);
        std::fs::write(mdir.join("custom-test.gguf"), &valid_gguf).unwrap();

        // A truncated/corrupt catalog file is flagged as damaged, not installed.
        std::fs::write(mdir.join("Ternary-Bonsai-2-9B-Q2_0.gguf"), &valid_gguf).unwrap();
        // A non-GGUF file is also flagged as damaged.
        std::fs::write(mdir.join("broken.gguf"), vec![b'X'; 128]).unwrap();

        let models = list_models(&dir, &HashMap::new());
        let custom = models
            .iter()
            .find(|m| m.file_name == "custom-test.gguf")
            .unwrap();
        assert!(custom.installed && !custom.damaged);

        let bonsai9 = models.iter().find(|m| m.id == "bonsai-9b").unwrap();
        assert!(!bonsai9.installed && bonsai9.damaged);
        assert!(bonsai9.damage_reason.is_some());
        assert_eq!(bonsai9.bytes, 2_556_640_128);

        let broken = models
            .iter()
            .find(|m| m.file_name == "broken.gguf")
            .unwrap();
        assert!(!broken.installed && broken.damaged);

        let candidates = candidate_models_for_auto_start(&models, "bonsai-9b");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].file_name, "custom-test.gguf");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_model_removes_managed_file_and_partial_without_deleting_imported_external() {
        let dir = std::env::temp_dir().join(format!("keel_del_test_{}", uuid::Uuid::new_v4()));
        let mdir = models_dir(&dir);
        std::fs::create_dir_all(&mdir).unwrap();

        let managed = mdir.join("Ternary-Bonsai-2-9B-Q2_0.gguf");
        let part = partial_download_path(&managed);
        std::fs::write(&managed, b"bad").unwrap();
        std::fs::write(&part, b"partial").unwrap();

        let external = dir.join("outside.gguf");
        std::fs::write(&external, b"keep-me").unwrap();
        let mut custom = HashMap::new();
        custom.insert(
            "imported:outside.gguf".to_string(),
            external.to_string_lossy().to_string(),
        );

        delete_model_file(&dir, "bonsai-9b", &mut custom).unwrap();
        assert!(!managed.exists());
        assert!(!part.exists());

        delete_model_file(&dir, "imported:outside.gguf", &mut custom).unwrap();
        assert!(!custom.contains_key("imported:outside.gguf"));
        assert!(external.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn candidate_models_order_preferred_then_fast_before_desktop() {
        let mut models = default_catalog();
        for m in &mut models {
            m.installed = true;
            m.local_path = Some(format!("/fake/{}", m.file_name));
        }
        // Mark preferred bonsai-9b as damaged; bonsai-4b/fast should come before bonsai-27b.
        let b9 = models.iter_mut().find(|m| m.id == "bonsai-9b").unwrap();
        b9.installed = false;
        b9.damaged = true;

        let order: Vec<String> = candidate_models_for_auto_start(&models, "bonsai-9b")
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert!(!order.contains(&"bonsai-9b".to_string()));
        assert_eq!(order.last().map(String::as_str), Some("bonsai-27b"));
        assert_eq!(order.first().map(String::as_str), Some("lfm3-1.7b"));
    }
}
