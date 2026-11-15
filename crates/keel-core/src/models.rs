use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

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
            detail: "Fast local starter model. 16K context, small download, quick tool calls."
                .to_string(),
            local_path: None,
            installed: false,
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
            detail: "Recommended everyday agent model. 32K context and strong tool reliability."
                .to_string(),
            local_path: None,
            installed: false,
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
            detail: "Compact low-RAM text and tool model with 8K context.".to_string(),
            local_path: None,
            installed: false,
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
            detail: "Compact 2B tool-capable model. Good for light laptops and quick checks."
                .to_string(),
            local_path: None,
            installed: false,
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
            detail: "Desktop-tier ternary model for 16 GB+ machines. Higher coding accuracy."
                .to_string(),
            local_path: None,
            installed: false,
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

/// Scans `models_dir` plus any custom imported file paths and returns the merged list.
pub fn list_models(data_dir: &Path, custom_paths: &HashMap<String, String>) -> Vec<CatalogModel> {
    let dir = models_dir(data_dir);
    let mut models = default_catalog();

    for m in &mut models {
        let managed = dir.join(&m.file_name);
        if managed.is_file() {
            m.local_path = Some(managed.to_string_lossy().to_string());
            m.installed = true;
            if let Ok(meta) = std::fs::metadata(&managed) {
                m.bytes = meta.len();
            }
        } else if let Some(custom) = custom_paths.get(&m.id) {
            let p = Path::new(custom);
            if p.is_file() {
                m.local_path = Some(custom.clone());
                m.installed = true;
                if let Ok(meta) = std::fs::metadata(p) {
                    m.bytes = meta.len();
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
                        detail: "Imported local GGUF model.".to_string(),
                        local_path: Some(path.to_string_lossy().to_string()),
                        installed: true,
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
                installed: true,
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
    mut on_progress: F,
) -> Result<u64>
where
    F: FnMut(DownloadProgress),
{
    if let Some(parent) = dest_path.parent() {
        fs::create_dir_all(parent).await?;
    }

    let part_path = dest_path.with_extension("gguf.part");
    let client = reqwest::Client::builder()
        .user_agent("keel-desktop/0.1")
        .build()?;
    let resp = client.get(url).send().await?.error_for_status()?;
    let total = resp.content_length();

    let mut file = fs::File::create(&part_path).await?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    let mut last_reported: u64 = 0;

    while let Some(chunk) = stream.next().await {
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
    fn list_models_detects_local_gguf_files() {
        let dir = std::env::temp_dir().join(format!("keel_models_test_{}", uuid::Uuid::new_v4()));
        let mdir = models_dir(&dir);
        std::fs::create_dir_all(&mdir).unwrap();
        std::fs::write(mdir.join("MiniCPM5-2B-Q4_K_M.gguf"), b"gguf").unwrap();
        std::fs::write(mdir.join("custom-test.gguf"), b"gguf2").unwrap();

        let models = list_models(&dir, &HashMap::new());
        let minicpm = models.iter().find(|m| m.id == "minicpm5-2b").unwrap();
        assert!(minicpm.installed);
        assert!(models.iter().any(|m| m.file_name == "custom-test.gguf"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
