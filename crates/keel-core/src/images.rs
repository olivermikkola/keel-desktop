//! Optional local image generation via the AUTOMATIC1111 / Forge API.
//! Keel owns its job and artifact records; the user owns the image server and weights.
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageRequest {
    pub endpoint: String,
    pub prompt: String,
    pub negative_prompt: String,
    pub checkpoint: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
}

impl ImageRequest {
    fn validate(&self) -> Result<()> {
        local_endpoint(&self.endpoint)?;
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
    let endpoint = local_endpoint(&request.endpoint)?;
    let mut body = json!({ "prompt": request.prompt, "negative_prompt": request.negative_prompt,
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
