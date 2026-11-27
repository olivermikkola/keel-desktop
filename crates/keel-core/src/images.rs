//! Optional local image generation via AUTOMATIC1111 / Forge API or direct
//! `stable-diffusion.cpp` (`sd-cli`, MIT, Vulkan/Metal/CUDA/CPU) local binary.
//! Keel owns its job and artifact records; the user owns the image engine and weights.
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// Which local engine produces images. Never a silent cloud fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageBackendKind {
    /// Loopback AUTOMATIC1111 / Forge HTTP API (`http://127.0.0.1:7860`).
    #[default]
    Server,
    /// Direct `leejet/stable-diffusion.cpp` CLI (`sd-cli`, MIT, Vulkan/Metal/CUDA/CPU).
    StableDiffusionCpp,
}

/// Optional visual style presets ported from Keel Mobile (`ImageRequest.Style`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageStyle {
    Photo,
    Illustration,
    Animation,
    Sketch,
}

impl ImageStyle {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "photo" => Some(Self::Photo),
            "illustration" => Some(Self::Illustration),
            "animation" => Some(Self::Animation),
            "sketch" => Some(Self::Sketch),
            _ => None,
        }
    }

    /// Suffix added cleanly to the user prompt when a Mobile-parity style preset is chosen.
    pub fn prompt_suffix(self) -> &'static str {
        match self {
            Self::Photo => "natural lighting, realistic photograph, crisp focus, high detail",
            Self::Illustration => {
                "clean editorial illustration, rich texture, balanced composition"
            }
            Self::Animation => {
                "stylized animated film still, expressive lighting, clean cel shading"
            }
            Self::Sketch => "hand-drawn pencil and ink sketch on warm paper, fine linework",
        }
    }

    pub fn default_negative(self) -> &'static str {
        "blurry, deformed, bad anatomy, low quality, watermark, garbled text"
    }
}

/// Enrich prompt & negative prompt when an optional style preset is selected.
pub fn apply_style_prompts(
    prompt: &str,
    negative: &str,
    style: Option<ImageStyle>,
) -> (String, String) {
    let p = prompt.trim();
    let n = negative.trim();
    match style {
        None => (p.to_string(), n.to_string()),
        Some(st) => {
            let styled_prompt = format!("{p}, {}", st.prompt_suffix());
            let styled_neg = if n.is_empty() {
                st.default_negative().to_string()
            } else {
                n.to_string()
            };
            (styled_prompt, styled_neg)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRequest {
    #[serde(default)]
    pub backend: ImageBackendKind,
    #[serde(default)]
    pub endpoint: String,
    pub prompt: String,
    #[serde(default)]
    pub negative_prompt: String,
    #[serde(default)]
    pub checkpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<ImageStyle>,
    /// Local `sd-cli` executable path when `backend == StableDiffusionCpp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sd_cli_path: Option<String>,
    /// Local `.gguf` or `.safetensors` diffusion checkpoint when `backend == StableDiffusionCpp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
}

impl ImageRequest {
    fn validate(&self) -> Result<()> {
        match self.backend {
            ImageBackendKind::Server => {
                local_endpoint(&self.endpoint)?;
            }
            ImageBackendKind::StableDiffusionCpp => {
                let cli = self
                    .sd_cli_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .context("Configure a local sd-cli binary path first")?;
                let cli_path = Path::new(cli);
                let looks_like_path =
                    cli_path.is_absolute() || cli.contains('/') || cli.contains('\\');
                if looks_like_path && !cli_path.is_file() {
                    bail!("sd-cli binary '{}' was not found on this computer.", cli);
                }
                let model = self
                    .model_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .context("Choose a local .gguf or .safetensors image model file first")?;
                if !Path::new(model).is_file() {
                    bail!("Image model '{}' does not exist on this computer.", model);
                }
            }
        }
        if self.prompt.trim().is_empty() || self.prompt.len() > 8000 {
            bail!("Write an image prompt (up to 8000 bytes)");
        }
        if self.negative_prompt.len() > 8000 {
            bail!("Negative prompt is too long");
        }
        if [self.width, self.height]
            .iter()
            .any(|n| !(256..=1536).contains(n) || n % 64 != 0)
            || !(1..=50).contains(&self.steps)
        {
            bail!("Use dimensions from 256 to 1536 in multiples of 64, and 1–50 steps");
        }
        Ok(())
    }
}

/// Detected local open-source media binaries on this machine (no downloads).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalMediaTools {
    pub audio_cpp: Option<String>,
    pub sd_cli: Option<String>,
    pub ffmpeg: Option<String>,
    pub whisper_cli: Option<String>,
    pub piper: Option<String>,
}

fn find_on_path(names: &[&str]) -> Option<String> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        for name in names {
            let cand = dir.join(name);
            if cand.is_file() {
                return Some(cand.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Detect an installed `sd-cli` (stable-diffusion.cpp) binary on PATH or `~/.local/share/stable-diffusion.cpp/`.
pub fn detect_sd_cli_binary() -> Option<String> {
    let names: &[&str] = if cfg!(windows) {
        &["sd-cli.exe", "sd.exe"]
    } else {
        &["sd-cli", "sd"]
    };
    if let Some(found) = find_on_path(names) {
        return Some(found);
    }
    if let Some(home) = dirs::home_dir() {
        let base = home
            .join(".local")
            .join("share")
            .join("stable-diffusion.cpp");
        for name in names {
            let cand = base.join(name);
            if cand.is_file() {
                return Some(cand.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Detect locally installed MIT/Apache-2.0 media binaries on this computer.
pub fn detect_local_media_tools() -> LocalMediaTools {
    let ffmpeg_names: &[&str] = if cfg!(windows) {
        &["ffmpeg.exe"]
    } else {
        &["ffmpeg"]
    };
    let whisper_names: &[&str] = if cfg!(windows) {
        &["whisper-cli.exe", "whisper-cpp.exe"]
    } else {
        &["whisper-cli", "whisper-cpp"]
    };
    let piper_names: &[&str] = if cfg!(windows) {
        &["piper.exe"]
    } else {
        &["piper"]
    };
    LocalMediaTools {
        audio_cpp: crate::audio::detect_audio_cpp_binary(),
        sd_cli: detect_sd_cli_binary(),
        ffmpeg: find_on_path(ffmpeg_names),
        whisper_cli: find_on_path(whisper_names),
        piper: find_on_path(piper_names),
    }
}

/// Build CLI argv for `stable-diffusion.cpp` (`sd-cli`).
pub fn sd_cpp_argv(request: &ImageRequest, model: &str, out_png: &Path) -> Vec<String> {
    let (prompt, negative) =
        apply_style_prompts(&request.prompt, &request.negative_prompt, request.style);
    let mut argv = vec![
        "-m".into(),
        model.to_string(),
        "-p".into(),
        prompt,
        "-W".into(),
        request.width.to_string(),
        "-H".into(),
        request.height.to_string(),
        "--steps".into(),
        request.steps.to_string(),
        "-o".into(),
        out_png.to_string_lossy().into_owned(),
    ];
    if !negative.is_empty() {
        argv.push("-n".into());
        argv.push(negative);
    }
    argv
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageJobStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageJob {
    pub id: String,
    pub status: ImageJobStatus,
    pub request: ImageRequest,
    pub workspace_path: String,
    pub artifact_path: Option<String>,
    pub error: Option<String>,
    pub created_at: u64,
}

struct ImageState {
    jobs: HashMap<String, ImageJob>,
    tasks: HashMap<String, AbortHandle>,
}

#[derive(Clone)]
pub struct ImageService {
    directory: PathBuf,
    state: Arc<Mutex<ImageState>>,
}

impl ImageService {
    pub fn new(data_dir: &Path) -> Result<Self> {
        let directory = data_dir.join("image-jobs");
        std::fs::create_dir_all(&directory)?;
        let mut jobs = HashMap::new();
        for entry in std::fs::read_dir(&directory)?.flatten() {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            let Ok(mut job) = serde_json::from_str::<ImageJob>(&raw) else {
                continue;
            };
            // Never replay a generation after restarting the desktop app.
            if job.status == ImageJobStatus::Running {
                job.status = ImageJobStatus::Failed;
                job.error =
                    Some("Keel closed before this generation finished. You can try again.".into());
                save_job(&directory, &job)?;
            }
            jobs.insert(job.id.clone(), job);
        }
        Ok(Self {
            directory,
            state: Arc::new(Mutex::new(ImageState {
                jobs,
                tasks: HashMap::new(),
            })),
        })
    }

    pub async fn list(&self) -> Vec<ImageJob> {
        let mut jobs: Vec<_> = self.state.lock().await.jobs.values().cloned().collect();
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        jobs
    }

    pub async fn start<F>(
        &self,
        workspace: &Path,
        request: ImageRequest,
        on_change: F,
    ) -> Result<ImageJob>
    where
        F: Fn(ImageJob) + Send + 'static,
    {
        request.validate()?;
        let root = crate::tools::validate_workspace_root(workspace)?;
        let job = ImageJob {
            id: uuid::Uuid::new_v4().to_string(),
            status: ImageJobStatus::Running,
            request,
            workspace_path: root.to_string_lossy().into_owned(),
            artifact_path: None,
            error: None,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        };
        let mut state = self.state.lock().await;
        if !state.tasks.is_empty() {
            bail!("An image generation is already running. Wait for it or stop waiting first.");
        }
        save_job(&self.directory, &job)?;
        state.jobs.insert(job.id.clone(), job.clone());
        let service = self.clone();
        let mut running = job.clone();
        let handle = tokio::spawn(async move {
            let result = generate_png(&running.request).await;
            let mut state = service.state.lock().await;
            if state.jobs.get(&running.id).map(|j| &j.status) != Some(&ImageJobStatus::Running) {
                return;
            }
            let result = result.and_then(|bytes| save_artifact(&root, &running.id, &bytes));
            match result {
                Ok(path) => {
                    running.artifact_path = Some(path);
                    running.status = ImageJobStatus::Succeeded;
                }
                Err(error) => {
                    running.status = ImageJobStatus::Failed;
                    running.error = Some(error.to_string());
                }
            }
            if let Err(error) = save_job(&service.directory, &running) {
                running.error = Some(format!("Could not save job history: {error}"));
            }
            state.tasks.remove(&running.id);
            state.jobs.insert(running.id.clone(), running.clone());
            on_change(running);
        });
        state.tasks.insert(job.id.clone(), handle.abort_handle());
        Ok(job)
    }

    /// Cancels Keel's request, never the server's global queue or someone else's job.
    pub async fn cancel(&self, id: &str) -> Result<ImageJob> {
        let mut state = self.state.lock().await;
        let mut job = state.jobs.get(id).context("Unknown image job")?.clone();
        if job.status == ImageJobStatus::Running {
            if let Some(handle) = state.tasks.remove(id) {
                handle.abort();
            }
            job.status = ImageJobStatus::Cancelled;
            job.error =
                Some("Stopped waiting. The image server may still finish its computation.".into());
            save_job(&self.directory, &job)?;
            state.jobs.insert(id.to_string(), job.clone());
        }
        Ok(job)
    }
}

fn save_job(directory: &Path, job: &ImageJob) -> Result<()> {
    let path = directory.join(format!("{}.json", job.id));
    crate::projects::write_json(&path, job)
}

pub fn local_endpoint(raw: &str) -> Result<String> {
    let url = reqwest::Url::parse(raw.trim()).context("Enter a local image server URL")?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("Use a loopback image server, such as http://127.0.0.1:7860. Hosted providers are not configured.");
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}

async fn bounded_json(mut response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    if !status.is_success() {
        bail!("Image server returned HTTP {status}");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > MAX_IMAGE_BYTES * 2 {
            bail!("Image response is too large");
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).context("Image server returned invalid JSON")
}

pub async fn probe_checkpoints(endpoint: &str) -> Result<Vec<String>> {
    let endpoint = local_endpoint(endpoint)?;
    let value = bounded_json(
        client()?
            .get(format!("{endpoint}/sdapi/v1/sd-models"))
            .send()
            .await?,
    )
    .await?;
    let entries = value
        .as_array()
        .context("Expected an AUTOMATIC1111 / Forge model list")?;
    Ok(entries
        .iter()
        .filter_map(|m| m.get("title")?.as_str().map(String::from))
        .collect())
}

async fn generate_png(request: &ImageRequest) -> Result<Vec<u8>> {
    match request.backend {
        ImageBackendKind::Server => generate_png_server(request).await,
        ImageBackendKind::StableDiffusionCpp => generate_png_sd_cpp(request).await,
    }
}

async fn generate_png_server(request: &ImageRequest) -> Result<Vec<u8>> {
    let endpoint = local_endpoint(&request.endpoint)?;
    let (prompt, negative) =
        apply_style_prompts(&request.prompt, &request.negative_prompt, request.style);
    let mut body = json!({ "prompt": prompt, "negative_prompt": negative,
        "width": request.width, "height": request.height, "steps": request.steps,
        "batch_size": 1, "n_iter": 1, "seed": -1, "send_images": true, "save_images": false });
    if !request.checkpoint.is_empty() {
        body["override_settings"] = json!({ "sd_model_checkpoint": request.checkpoint });
        body["override_settings_restore_afterwards"] = json!(true);
    }
    let response = client()?
        .post(format!("{endpoint}/sdapi/v1/txt2img"))
        .json(&body)
        .send()
        .await?;
    let value = bounded_json(response).await?;
    let image = value
        .get("images")
        .and_then(|a| a.get(0))
        .and_then(Value::as_str)
        .context("Image server returned no image")?;
    let encoded = image
        .strip_prefix("data:image/png;base64,")
        .unwrap_or(image);
    let bytes = STANDARD
        .decode(encoded)
        .context("Image server returned invalid image data")?;
    if bytes.len() > MAX_IMAGE_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bail!("Expected a PNG image under 20 MB; configure the server's output format to PNG");
    }
    Ok(bytes)
}

async fn generate_png_sd_cpp(request: &ImageRequest) -> Result<Vec<u8>> {
    let cli = request
        .sd_cli_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .context("No sd-cli binary configured")?
        .to_string();
    let model = request
        .model_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .context("No image model file configured")?
        .to_string();
    let out_dir = std::env::temp_dir().join(format!("keel_sd_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&out_dir)?;
    let out_png = out_dir.join("out.png");
    let argv = sd_cpp_argv(request, &model, &out_png);
    let run_res = tokio::process::Command::new(&cli)
        .args(&argv)
        .kill_on_drop(true)
        .output()
        .await
        .with_context(|| format!("Failed to launch sd-cli '{cli}'"));
    let output = match run_res {
        Ok(o) => o,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&out_dir);
            return Err(err);
        }
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let _ = std::fs::remove_dir_all(&out_dir);
        bail!(
            "sd-cli exited with {}: {} {}",
            output.status,
            stderr.trim(),
            stdout.trim()
        );
    }
    let bytes = std::fs::read(&out_png).context("sd-cli finished without creating output PNG");
    let _ = std::fs::remove_dir_all(&out_dir);
    let bytes = bytes?;
    if bytes.len() > MAX_IMAGE_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bail!("sd-cli produced invalid or oversized PNG output");
    }
    Ok(bytes)
}

/// Generate a PNG via the configured backend and write it safely to a workspace-relative `.png` path.
pub async fn generate_workspace_image(
    workspace: &Path,
    relative_path: &str,
    request: &ImageRequest,
) -> Result<String> {
    request.validate()?;
    let root = crate::tools::validate_workspace_root(workspace)?;
    let clean_rel = relative_path.trim();
    if !clean_rel.to_ascii_lowercase().ends_with(".png") {
        bail!("Image output path must end with .png (for example assets/hero.png)");
    }
    let target = crate::tools::confine_path(&root, clean_rel)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = generate_png(request).await?;
    std::fs::write(&target, &bytes)?;
    Ok(clean_rel.to_string())
}

fn save_artifact(root: &Path, id: &str, bytes: &[u8]) -> Result<String> {
    use std::io::Write;
    let assets = crate::tools::confine_path(root, "assets")?;
    std::fs::create_dir_all(assets)?;
    let relative = format!("assets/keel-{id}.png");
    let path = crate::tools::confine_path(root, &relative)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(relative)
}

pub fn read_workspace_png(root: &Path, relative: &str) -> Result<String> {
    let path = crate::tools::confine_path(root, relative)?;
    let mut file = std::fs::File::open(path)?;
    use std::io::Read;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((MAX_IMAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_IMAGE_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        bail!("Preview requires a PNG image under 20 MB");
    }
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

pub fn read_workspace_image(root: &Path, relative: &str) -> Result<String> {
    let path = crate::tools::confine_path(root, relative)?;
    let mut file = std::fs::File::open(&path)?;
    use std::io::Read;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take((MAX_IMAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        bail!("Preview requires an image under 20 MB");
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
    {
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| anyhow::anyhow!("SVG preview requires valid UTF-8"))?;
        let trimmed = text.trim_start_matches('\u{feff}').trim_start();
        if !trimmed.starts_with('<') || !text.to_ascii_lowercase().contains("<svg") {
            bail!("Preview requires a valid SVG image");
        }
        "image/svg+xml"
    } else {
        bail!("Preview requires a PNG, JPEG, WebP, GIF, or SVG image under 20 MB");
    };
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}
