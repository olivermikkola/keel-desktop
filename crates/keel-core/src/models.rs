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
            detail: "Smarter everyday helper for a computer that can keep up.".to_string(),
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
        CatalogModel {
            id: "bonsai-2.5-27b".to_string(),
            name: "Bonsai 2.5 · 27B".to_string(),
            repository: "prism-ml/Ternary-Bonsai-2.5-27B-GGUF".to_string(),
            file_name: "Ternary-Bonsai-2.5-27B-Q2_0.gguf".to_string(),
            revision: "main".to_string(),
            bytes: 7_670_000_000,
            sha256: None,
            quantization: "Q2_0".to_string(),
            context_size: 32_768,
            tier: "desktop".to_string(),
            detail: "Newer agentic 27B ternary model for multi-step coding and tools. Needs 16 GB+ memory."
                .to_string(),
            local_path: None,
            installed: false,
            damaged: false,
            damage_reason: None,
        },
        CatalogModel {
            id: "inkling-2-mini".to_string(),
            name: "Inkling 2 Mini".to_string(),
            repository: "unsloth/Inkling-2-Mini-GGUF".to_string(),
            file_name: "Inkling-2-Mini-UD-Q4_K_M.gguf".to_string(),
            revision: "8078d149286eec7193795413b9e0643da6d11fa1".to_string(),
            bytes: 16_610_544_608,
            sha256: Some(
                "78fcedcf93c5d1bfe83e225513b71392bb52259be0eaa5f8899c856a2e5834ae".to_string(),
            ),
            quantization: "Q4_K_M".to_string(),
            context_size: 32_768,
            tier: "desktop".to_string(),
            detail: "26.4B Looped MoE (3.4B active) for everyday assistant and coding work. Needs 24 GB+ memory."
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
    for seg in &parts[4..] {
        if *seg == "."
            || *seg == ".."
            || seg.contains('\\')
            || seg.contains(':')
            || seg.chars().any(|c| c.is_control())
        {
            bail!("URL contains an invalid file path segment");
        }
    }
    let file_path = parts[4..].join("/");
    let file_name = parts.last().unwrap().to_string();
    if !file_name.to_ascii_lowercase().ends_with(".gguf") {
        bail!("URL must point to a .gguf file");
    }
    let resolve_url = format!("https://huggingface.co/{repo}/resolve/{revision}/{file_path}");
    Ok((repo, revision, file_name, resolve_url))
}

/// Returns an error if a custom download filename would overwrite a built-in catalog model.
pub fn ensure_non_catalog_filename(file_name: &str) -> Result<()> {
    if default_catalog()
        .iter()
        .any(|m| m.file_name.eq_ignore_ascii_case(file_name))
    {
        bail!(
            "That filename belongs to a built-in catalog model. Download it from the Models list instead."
        );
    }
    Ok(())
}

/// Reads `*.context_length` from a GGUF file header without loading weights.
/// Skips arrays by seeking so even 20+ GB files parse in under a millisecond.
pub fn read_gguf_context_length(path: &Path) -> Option<u32> {
    use std::io::{Read, Seek, SeekFrom};

    fn read_u32<R: Read>(r: &mut R) -> Option<u32> {
        let mut b = [0u8; 4];
        r.read_exact(&mut b).ok()?;
        Some(u32::from_le_bytes(b))
    }
    fn read_u64<R: Read>(r: &mut R) -> Option<u64> {
        let mut b = [0u8; 8];
        r.read_exact(&mut b).ok()?;
        Some(u64::from_le_bytes(b))
    }
    fn read_string<R: Read>(r: &mut R) -> Option<String> {
        let len = read_u64(r)?;
        if len > 1_048_576 {
            return None;
        }
        let mut buf = vec![0u8; len as usize];
        r.read_exact(&mut buf).ok()?;
        String::from_utf8(buf).ok()
    }
    fn skip_string<R: Read + Seek>(r: &mut R) -> Option<()> {
        let len = read_u64(r)?;
        if len > 64 * 1024 * 1024 {
            return None;
        }
        r.seek(SeekFrom::Current(len as i64)).ok()?;
        Some(())
    }
    fn scalar_size(vtype: u32) -> Option<i64> {
        match vtype {
            0 | 1 | 7 => Some(1), // uint8, int8, bool
            2 | 3 => Some(2),     // uint16, int16
            4..=6 => Some(4),     // uint32, int32, float32
            10..=12 => Some(8),   // uint64, int64, float64
            _ => None,
        }
    }
    fn skip_value<R: Read + Seek>(r: &mut R, vtype: u32, depth: u8) -> Option<()> {
        if depth > 4 {
            return None;
        }
        if let Some(sz) = scalar_size(vtype) {
            r.seek(SeekFrom::Current(sz)).ok()?;
            return Some(());
        }
        match vtype {
            8 => skip_string(r),
            9 => {
                let elem_type = read_u32(r)?;
                let count = read_u64(r)?;
                if count > 4_000_000 {
                    return None;
                }
                if let Some(sz) = scalar_size(elem_type) {
                    let bytes = (count as i64).checked_mul(sz)?;
                    r.seek(SeekFrom::Current(bytes)).ok()?;
                    return Some(());
                }
                for _ in 0..count {
                    skip_value(r, elem_type, depth + 1)?;
                }
                Some(())
            }
            _ => None,
        }
    }

    let mut file = std::fs::File::open(path).ok()?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }
    let version = read_u32(&mut file)?;
    if !(2..=3).contains(&version) {
        return None;
    }
    let _tensor_count = read_u64(&mut file)?;
    let kv_count = read_u64(&mut file)?;
    if kv_count > 4096 {
        return None;
    }
    for _ in 0..kv_count {
        let key = read_string(&mut file)?;
        let vtype = read_u32(&mut file)?;
        if key.ends_with(".context_length") {
            let val: Option<u64> = match vtype {
                4 => read_u32(&mut file).map(u64::from),
                5 => {
                    let mut b = [0u8; 4];
                    file.read_exact(&mut b).ok()?;
                    u64::try_from(i32::from_le_bytes(b)).ok()
                }
                10 => read_u64(&mut file),
                11 => {
                    let mut b = [0u8; 8];
                    file.read_exact(&mut b).ok()?;
                    u64::try_from(i64::from_le_bytes(b)).ok()
                }
                _ => {
                    skip_value(&mut file, vtype, 0)?;
                    None
                }
            };
            if let Some(ctx) = val.and_then(|v| u32::try_from(v).ok()) {
                if ctx >= 512 {
                    return Some(ctx);
                }
            }
        } else {
            skip_value(&mut file, vtype, 0)?;
        }
    }
    None
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
                    if let Some(native_ctx) = read_gguf_context_length(&path) {
                        if native_ctx > m.context_size {
                            m.context_size = native_ctx;
                        }
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
                    let quantization = infer_quant_label(&file_name);
                    let context_size = read_gguf_context_length(&path).unwrap_or(32_768);
                    models.push(CatalogModel {
                        id,
                        name: file_name.trim_end_matches(".gguf").to_string(),
                        repository: "local".to_string(),
                        file_name,
                        revision: "local".to_string(),
                        bytes,
                        sha256: None,
                        quantization,
                        context_size,
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
            let quantization = infer_quant_label(&file_name);
            let context_size = read_gguf_context_length(path).unwrap_or(32_768);
            models.push(CatalogModel {
                id: id.clone(),
                name: file_name.trim_end_matches(".gguf").to_string(),
                repository: "imported".to_string(),
                file_name,
                revision: "imported".to_string(),
                bytes,
                sha256: None,
                quantization,
                context_size,
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

/// A GGUF model discovered from Hugging Face Hub (`/api/models`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveredModel {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub author: String,
    pub likes: u64,
    pub downloads: u64,
    pub trending_score: f64,
    pub file_name: String,
    pub resolve_url: String,
    pub repo_url: String,
    pub bytes: Option<u64>,
    pub quantization: String,
    pub architecture: Option<String>,
    pub context_size: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct HfApiModel {
    id: String,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    downloads: u64,
    #[serde(default, rename = "trendingScore")]
    trending_score: f64,
    #[serde(default)]
    siblings: Vec<HfApiSibling>,
    #[serde(default)]
    gguf: Option<HfApiGguf>,
    #[serde(default)]
    config: Option<HfApiConfig>,
}

#[derive(Debug, Deserialize)]
struct HfApiSibling {
    rfilename: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HfApiLfs>,
}

impl HfApiSibling {
    fn byte_size(&self) -> Option<u64> {
        self.size.or_else(|| self.lfs.as_ref().and_then(|l| l.size))
    }
}

#[derive(Debug, Deserialize)]
struct HfApiLfs {
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct HfApiGguf {
    #[serde(default)]
    total: Option<u64>,
    #[serde(default)]
    architecture: Option<String>,
    #[serde(default)]
    context_length: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct HfApiConfig {
    #[serde(default)]
    model_type: Option<String>,
    #[serde(default)]
    max_position_embeddings: Option<u32>,
}

/// Extracts a readable quantization label from a `.gguf` filename (e.g. `Q4_K_M`, `Q2_0`, `IQ4_XS`).
pub fn infer_quant_label(file_name: &str) -> String {
    let stem = file_name
        .strip_suffix(".gguf")
        .or_else(|| file_name.strip_suffix(".GGUF"))
        .unwrap_or(file_name);
    let upper = stem.to_ascii_uppercase();
    for token in [
        "Q4_K_XL", "Q4_K_M", "Q4_K_S", "Q4_K_L", "Q4_K", "Q5_K_XL", "Q5_K_M", "Q5_K_S", "Q6_K",
        "Q8_0", "Q3_K_XL", "Q3_K_M", "Q3_K_S", "Q3_K_L", "Q2_K_XL", "Q2_K", "Q2_0", "IQ4_XS",
        "IQ4_NL", "IQ3_XXS", "IQ3_XS", "IQ3_M", "IQ2_XXS", "IQ2_XS", "PQ2_0", "PTQ1_0", "BF16",
        "F16",
    ] {
        if upper.contains(token) {
            return token.to_string();
        }
    }
    "GGUF".to_string()
}

fn is_single_model_gguf(rfilename: &str) -> bool {
    let lower = rfilename.to_ascii_lowercase();
    lower.ends_with(".gguf")
        && !lower.contains("mmproj")
        && !lower.contains("-of-0")
        && !lower.contains("imatrix")
}

fn quant_preference_rank(rfilename: &str) -> u8 {
    let upper = rfilename.to_ascii_uppercase();
    if upper.contains("Q4_K_M") {
        0
    } else if upper.contains("Q4_K_S") || upper.contains("Q4_K") {
        1
    } else if upper.contains("Q2_0") && !upper.contains("PQ2_0") {
        2
    } else if upper.contains("Q5_K_M") {
        3
    } else if upper.contains("IQ4_XS") || upper.contains("IQ4_NL") {
        4
    } else if upper.contains("Q6_K") {
        5
    } else if upper.contains("Q3_K_M") {
        6
    } else if upper.contains("Q8_0") {
        7
    } else if upper.contains("Q2_K") {
        8
    } else {
        9
    }
}

fn pick_best_gguf_sibling(siblings: &[HfApiSibling]) -> Option<&HfApiSibling> {
    siblings
        .iter()
        .filter(|s| is_single_model_gguf(&s.rfilename))
        .min_by_key(|s| {
            (
                quant_preference_rank(&s.rfilename),
                s.byte_size().unwrap_or(u64::MAX),
                s.rfilename.len(),
            )
        })
}

fn api_model_to_discovered(model: HfApiModel) -> Option<DiscoveredModel> {
    let sibling = pick_best_gguf_sibling(&model.siblings)?;
    let (author, repo_name) = model
        .id
        .split_once('/')
        .unwrap_or(("huggingface", model.id.as_str()));
    let file_name = sibling
        .rfilename
        .rsplit('/')
        .next()
        .unwrap_or(sibling.rfilename.as_str())
        .to_string();
    let resolve_url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        model.id, sibling.rfilename
    );
    let repo_url = format!("https://huggingface.co/{}", model.id);
    let bytes = sibling
        .byte_size()
        .or_else(|| model.gguf.as_ref().and_then(|g| g.total));
    let architecture = model
        .gguf
        .as_ref()
        .and_then(|g| g.architecture.clone())
        .or_else(|| model.config.as_ref().and_then(|c| c.model_type.clone()));
    let context_size = model
        .gguf
        .as_ref()
        .and_then(|g| g.context_length)
        .or_else(|| {
            model
                .config
                .as_ref()
                .and_then(|c| c.max_position_embeddings)
        });
    Some(DiscoveredModel {
        id: format!("hf:{}", model.id),
        repo_id: model.id.clone(),
        name: repo_name.to_string(),
        author: author.to_string(),
        likes: model.likes,
        downloads: model.downloads,
        trending_score: model.trending_score,
        quantization: infer_quant_label(&file_name),
        file_name,
        resolve_url,
        repo_url,
        bytes,
        architecture,
        context_size,
    })
}

/// Parses a Hugging Face `/api/models` JSON array into ready-to-download GGUF entries.
pub fn parse_hf_api_models(json_bytes: &[u8]) -> Result<Vec<DiscoveredModel>> {
    let raw: Vec<HfApiModel> =
        serde_json::from_slice(json_bytes).context("Unexpected response from Hugging Face")?;
    Ok(raw
        .into_iter()
        .filter_map(api_model_to_discovered)
        .collect())
}

/// Parses a Hugging Face repo URL (`https://huggingface.co/<owner>/<repo>`) or `owner/repo` slug.
pub fn parse_huggingface_repo_input(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("https://huggingface.co/")
        .or_else(|| trimmed.strip_prefix("http://huggingface.co/"))
        .unwrap_or(trimmed);
    let clean = rest.split(['?', '#']).next().unwrap_or(rest);
    let parts: Vec<&str> = clean.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        bail!("Expected a Hugging Face link or owner/repo name");
    }
    let (owner, repo) = (parts[0], parts[1]);
    for seg in [owner, repo] {
        if seg == "."
            || seg == ".."
            || seg.contains('\\')
            || seg.contains(':')
            || seg.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            bail!("Invalid Hugging Face repository name");
        }
    }
    Ok(format!("{owner}/{repo}"))
}

/// Fetches trending or searched GGUF models from Hugging Face Hub.
pub async fn discover_huggingface_models(
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<DiscoveredModel>> {
    let client = reqwest::Client::builder()
        .user_agent(format!("keel-desktop/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let clamped = limit.clamp(1, 50).to_string();
    let mut url = reqwest::Url::parse("https://huggingface.co/api/models")?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("sort", "trendingScore");
        pairs.append_pair("direction", "-1");
        pairs.append_pair("limit", &clamped);
        pairs.append_pair("filter", "gguf");
        pairs.append_pair("full", "true");
        if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
            pairs.append_pair("search", q);
        }
    }
    let resp = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("Could not reach Hugging Face. Check your internet connection and try again.")?
        .error_for_status()
        .context("Hugging Face returned an error while loading models.")?;
    let bytes = resp.bytes().await?;
    if bytes.len() > 4 * 1024 * 1024 {
        bail!("Hugging Face response was unexpectedly large");
    }
    parse_hf_api_models(&bytes)
}

/// Accepts either a direct `.gguf` Hugging Face link OR a repo URL / `owner/repo` slug,
/// automatically picking the best single-file `.gguf` when given a repo.
pub async fn resolve_huggingface_input(raw: &str) -> Result<(String, String, String, String)> {
    if let Ok(parsed) = parse_huggingface_gguf_url(raw) {
        return Ok(parsed);
    }
    let repo_id = parse_huggingface_repo_input(raw)?;
    let client = reqwest::Client::builder()
        .user_agent(format!("keel-desktop/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let url = format!("https://huggingface.co/api/models/{repo_id}?full=true");
    let resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("Could not reach Hugging Face to inspect that repository.")?
        .error_for_status()
        .with_context(|| format!("Hugging Face repository `{repo_id}` could not be found."))?;
    let model: HfApiModel = resp
        .json()
        .await
        .context("Unexpected repository response from Hugging Face")?;
    let discovered = api_model_to_discovered(model)
        .with_context(|| format!("No standalone .gguf model file was found in `{repo_id}`."))?;
    Ok((
        discovered.repo_id,
        "main".to_string(),
        discovered.file_name,
        discovered.resolve_url,
    ))
}

/// Recursively finds valid standalone `.gguf` files (skipping `mmproj` projectors) up to `max_depth`.
pub fn scan_local_ggufs_in_dir(dir: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    fn walk(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < max_depth {
                    walk(&path, depth + 1, max_depth, out);
                }
            } else if path.is_file() {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if is_single_model_gguf(name) && validate_gguf_file(&path, None).is_ok() {
                    out.push(path);
                }
            }
        }
    }
    walk(dir, 0, max_depth, &mut found);
    found.sort();
    found
}

/// Returns `~/Models` (or `~/models`) when present on this machine.
pub fn default_user_models_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    for name in ["Models", "models"] {
        let candidate = home.join(name);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
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
        assert_eq!(order.last().map(String::as_str), Some("inkling-2-mini"));
        assert_eq!(order.first().map(String::as_str), Some("lfm3-1.7b"));
    }

    #[test]
    fn catalog_includes_inkling_2_mini() {
        let models = default_catalog();
        let inkling = models
            .iter()
            .find(|m| m.id == "inkling-2-mini")
            .expect("inkling-2-mini in catalog");
        assert_eq!(inkling.repository, "unsloth/Inkling-2-Mini-GGUF");
        assert_eq!(inkling.file_name, "Inkling-2-Mini-UD-Q4_K_M.gguf");
        assert!(inkling.sha256.is_some());
    }

    #[test]
    fn catalog_includes_bonsai_2_5_27b() {
        let models = default_catalog();
        let b25 = models
            .iter()
            .find(|m| m.id == "bonsai-2.5-27b")
            .expect("bonsai-2.5-27b in catalog");
        assert_eq!(b25.repository, "prism-ml/Ternary-Bonsai-2.5-27B-GGUF");
        assert_eq!(b25.file_name, "Ternary-Bonsai-2.5-27B-Q2_0.gguf");
    }

    #[test]
    fn parses_hf_api_models_and_picks_best_standalone_gguf() {
        let sample = br#"[
            {
                "id": "prism-ml/Ternary-Bonsai-2.5-27B-GGUF",
                "likes": 420,
                "downloads": 12000,
                "trendingScore": 98.5,
                "gguf": {"total": 7670000000, "architecture": "qwen3", "context_length": 32768},
                "siblings": [
                    {"rfilename": "README.md", "size": 4096},
                    {"rfilename": "mmproj-BF16.gguf", "size": 930000000},
                    {"rfilename": "Ternary-Bonsai-2.5-27B-Q2_0.gguf", "size": 7670000000},
                    {"rfilename": "Ternary-Bonsai-2.5-27B-PQ2_0.gguf", "size": 7250000000}
                ]
            },
            {
                "id": "example/Only-Shards-GGUF",
                "siblings": [
                    {"rfilename": "model-00001-of-00003.gguf", "size": 4000000000}
                ]
            }
        ]"#;
        let parsed = parse_hf_api_models(sample).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].repo_id, "prism-ml/Ternary-Bonsai-2.5-27B-GGUF");
        assert_eq!(parsed[0].file_name, "Ternary-Bonsai-2.5-27B-Q2_0.gguf");
        assert_eq!(parsed[0].quantization, "Q2_0");
        assert_eq!(parsed[0].bytes, Some(7_670_000_000));
        assert_eq!(parsed[0].context_size, Some(32_768));
    }

    #[test]
    fn parses_hf_repo_inputs_and_scans_local_ggufs() {
        assert_eq!(
            parse_huggingface_repo_input(
                "https://huggingface.co/prism-ml/Ternary-Bonsai-2.5-27B-GGUF/tree/main"
            )
            .unwrap(),
            "prism-ml/Ternary-Bonsai-2.5-27B-GGUF"
        );
        assert_eq!(
            parse_huggingface_repo_input("unsloth/Inkling-2-Mini-GGUF").unwrap(),
            "unsloth/Inkling-2-Mini-GGUF"
        );
        assert!(parse_huggingface_repo_input("not-a-repo").is_err());

        let dir = std::env::temp_dir().join(format!("keel_scan_test_{}", uuid::Uuid::new_v4()));
        let sub = dir.join("inkling-2-mini");
        std::fs::create_dir_all(&sub).unwrap();
        let mut valid_gguf = b"GGUF".to_vec();
        valid_gguf.resize(128, 0);
        std::fs::write(sub.join("Inkling-2-Mini-UD-Q4_K_M.gguf"), &valid_gguf).unwrap();
        std::fs::write(sub.join("mmproj-F16.gguf"), &valid_gguf).unwrap();
        std::fs::write(sub.join("bad.gguf"), vec![0u8; 128]).unwrap();

        let found = scan_local_ggufs_in_dir(&dir, 2);
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("Inkling-2-Mini-UD-Q4_K_M.gguf"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_traversal_and_catalog_collisions_in_hf_urls() {
        assert!(parse_huggingface_gguf_url(
            "https://huggingface.co/owner/repo/resolve/main/..\\evil.gguf"
        )
        .is_err());
        assert!(parse_huggingface_gguf_url(
            "https://huggingface.co/owner/repo/resolve/main/../evil.gguf"
        )
        .is_err());
        assert!(ensure_non_catalog_filename("Ternary-Bonsai-2-4B-Q2_0.gguf").is_err());
        assert!(ensure_non_catalog_filename("my-custom-model.gguf").is_ok());
    }

    #[test]
    fn reads_native_context_length_from_gguf_header() {
        let dir = std::env::temp_dir().join(format!("keel_gguf_ctx_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Inkling-2-Mini-UD-Q4_K_XL.gguf");
        let mut buf = Vec::new();
        buf.extend_from_slice(b"GGUF");
        buf.extend_from_slice(&3u32.to_le_bytes()); // version 3
        buf.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        buf.extend_from_slice(&2u64.to_le_bytes()); // kv_count
                                                    // KV 1: general.architecture = "inkling2_loop" (type 8 = string)
        let k1 = b"general.architecture";
        let v1 = b"inkling2_loop";
        buf.extend_from_slice(&(k1.len() as u64).to_le_bytes());
        buf.extend_from_slice(k1);
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(&(v1.len() as u64).to_le_bytes());
        buf.extend_from_slice(v1);
        // KV 2: inkling2_loop.context_length = 1048576 (type 4 = uint32)
        let k2 = b"inkling2_loop.context_length";
        buf.extend_from_slice(&(k2.len() as u64).to_le_bytes());
        buf.extend_from_slice(k2);
        buf.extend_from_slice(&4u32.to_le_bytes());
        buf.extend_from_slice(&1_048_576u32.to_le_bytes());
        buf.resize(128, 0);
        std::fs::write(&file, &buf).unwrap();

        assert_eq!(read_gguf_context_length(&file), Some(1_048_576));
        assert_eq!(
            infer_quant_label("Inkling-2-Mini-UD-Q4_K_XL.gguf"),
            "Q4_K_XL"
        );

        let mut custom = HashMap::new();
        custom.insert(
            "imported:Inkling-2-Mini-UD-Q4_K_XL.gguf".to_string(),
            file.to_string_lossy().to_string(),
        );
        let models = list_models(&dir, &custom);
        let imported = models
            .iter()
            .find(|m| m.id == "imported:Inkling-2-Mini-UD-Q4_K_XL.gguf")
            .unwrap();
        assert_eq!(imported.context_size, 1_048_576);
        assert_eq!(imported.quantization, "Q4_K_XL");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
