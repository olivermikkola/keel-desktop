mod support;
use base64::{engine::general_purpose::STANDARD, Engine};
use keel_core::images::{
    apply_style_prompts, generate_workspace_image, local_endpoint, read_workspace_image,
    read_workspace_png, sd_cpp_argv, ImageBackendKind, ImageJobStatus, ImageRequest, ImageService,
    ImageStyle,
};
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j5uoAAAAASUVORK5CYII=";
fn root() -> PathBuf {
    let path = std::env::temp_dir().join(format!("keel_images_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn request(endpoint: String) -> ImageRequest {
    ImageRequest {
        backend: ImageBackendKind::Server,
        endpoint,
        prompt: "A lakeside cabin".into(),
        negative_prompt: String::new(),
        checkpoint: "local-checkpoint".into(),
        style: None,
        sd_cli_path: None,
        model_path: None,
        width: 512,
        height: 512,
        steps: 20,
    }
}

#[tokio::test]
async fn image_job_uses_server_contract_saves_png_and_survives_restart() {
    let root = root();
    let (endpoint, server) = support::server(vec![(
        "application/json".into(),
        json!({"images":[PNG]}).to_string(),
        Duration::ZERO,
    )])
    .await;
    let service = ImageService::new(&root.join("data")).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let started = service
        .start(&root, request(endpoint), move |job| {
            tx.send(job).unwrap();
        })
        .await
        .unwrap();
    assert_eq!(started.status, ImageJobStatus::Running);
    let job = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.status, ImageJobStatus::Succeeded);
    let artifact = job.artifact_path.as_ref().unwrap();
    assert_eq!(
        std::fs::read(root.join(artifact)).unwrap(),
        STANDARD.decode(PNG).unwrap()
    );
    assert_eq!(
        read_workspace_png(&root, artifact).unwrap(),
        format!("data:image/png;base64,{PNG}")
    );
    let requests = server.await.unwrap();
    assert_eq!(requests[0]["prompt"], "A lakeside cabin");
    assert_eq!(
        requests[0]["override_settings"]["sd_model_checkpoint"],
        "local-checkpoint"
    );
    assert_eq!(requests[0]["batch_size"], 1);
    assert_eq!(requests[0]["save_images"], false);
    let restored = ImageService::new(&root.join("data")).unwrap();
    assert_eq!(restored.list().await[0].status, ImageJobStatus::Succeeded);
    assert!(read_workspace_png(&root, "../escape.png").is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn bad_server_response_fails_without_creating_an_artifact() {
    let root = root();
    let (endpoint, server) = support::server(vec![(
        "application/json".into(),
        json!({"images":["invalid"]}).to_string(),
        Duration::ZERO,
    )])
    .await;
    let service = ImageService::new(&root.join("data")).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    service
        .start(&root, request(endpoint), move |job| {
            tx.send(job).unwrap();
        })
        .await
        .unwrap();
    let job = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.status, ImageJobStatus::Failed);
    assert!(job.artifact_path.is_none());
    assert!(!root.join("assets").exists());
    server.await.unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn cancellation_is_persisted_and_running_jobs_are_not_replayed_after_restart() {
    let root = root();
    let (endpoint, server) = support::server(vec![(
        "application/json".into(),
        json!({"images":[PNG]}).to_string(),
        Duration::from_secs(1),
    )])
    .await;
    let service = ImageService::new(&root.join("data")).unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let job = service
        .start(&root, request(endpoint), move |job| {
            let _ = tx.send(job);
        })
        .await
        .unwrap();
    let cancelled = service.cancel(&job.id).await.unwrap();
    assert_eq!(cancelled.status, ImageJobStatus::Cancelled);
    assert!(!root.join("assets").exists());
    assert!(rx.recv().await.is_none());
    let restored = ImageService::new(&root.join("data")).unwrap();
    assert_eq!(restored.list().await[0].status, ImageJobStatus::Cancelled);
    // Simulate a job interrupted by an app exit.
    std::fs::write(
        root.join("data/image-jobs")
            .join(format!("{}.json", job.id)),
        serde_json::to_vec(&job).unwrap(),
    )
    .unwrap();
    let restored = ImageService::new(&root.join("data")).unwrap();
    assert_eq!(restored.list().await[0].status, ImageJobStatus::Failed);
    server.abort();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn media_connections_are_explicit_loopback_only() {
    assert_eq!(
        local_endpoint("http://127.0.0.1:7860/").unwrap(),
        "http://127.0.0.1:7860"
    );
    assert!(local_endpoint("http://[::1]:7860").is_ok());
    for bad in [
        "https://hosted.example",
        "http://127.0.0.1.evil.example",
        "http://user:pass@localhost:7860",
        "http://localhost:7860/path",
        "file:///tmp/image",
    ] {
        assert!(local_endpoint(bad).is_err(), "{bad}");
    }
}

#[test]
fn workspace_image_reader_supports_jpeg_webp_gif_svg_and_rejects_non_images() {
    let root = root();
    std::fs::write(root.join("photo.jpg"), [0xFF, 0xD8, 0xFF, 0xE0, 0x00]).unwrap();
    std::fs::write(root.join("pic.webp"), b"RIFF\x04\x00\x00\x00WEBP").unwrap();
    std::fs::write(root.join("anim.gif"), b"GIF89a\x01\x00").unwrap();
    std::fs::write(
        root.join("logo.svg"),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
    )
    .unwrap();
    std::fs::write(root.join("fake.jpg"), b"not a jpeg").unwrap();

    assert!(read_workspace_image(&root, "photo.jpg")
        .unwrap()
        .starts_with("data:image/jpeg;base64,"));
    assert!(read_workspace_image(&root, "pic.webp")
        .unwrap()
        .starts_with("data:image/webp;base64,"));
    assert!(read_workspace_image(&root, "anim.gif")
        .unwrap()
        .starts_with("data:image/gif;base64,"));
    assert!(read_workspace_image(&root, "logo.svg")
        .unwrap()
        .starts_with("data:image/svg+xml;base64,"));
    assert!(read_workspace_image(&root, "fake.jpg").is_err());
    assert!(read_workspace_image(&root, "../escape.jpg").is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn image_style_presets_enrich_prompts_and_sd_cpp_argv() {
    let (p, n) = apply_style_prompts("Nordic lighthouse", "", Some(ImageStyle::Illustration));
    assert!(p.contains("Nordic lighthouse"));
    assert!(p.contains("editorial illustration"));
    assert!(n.contains("watermark"));

    let req = ImageRequest {
        backend: ImageBackendKind::StableDiffusionCpp,
        endpoint: String::new(),
        prompt: "Nordic lighthouse".into(),
        negative_prompt: String::new(),
        checkpoint: String::new(),
        style: Some(ImageStyle::Photo),
        sd_cli_path: Some("sd-cli".into()),
        model_path: Some("/models/sd.gguf".into()),
        width: 512,
        height: 512,
        steps: 8,
    };
    let argv = sd_cpp_argv(
        &req,
        "/models/sd.gguf",
        std::path::Path::new("/tmp/out.png"),
    );
    assert!(argv.windows(2).any(|w| w == ["-m", "/models/sd.gguf"]));
    assert!(argv.windows(2).any(|w| w == ["-W", "512"]));
    assert!(argv.windows(2).any(|w| w == ["--steps", "8"]));
    assert!(argv.windows(2).any(|w| w == ["-o", "/tmp/out.png"]));
}

#[cfg(unix)]
#[tokio::test]
async fn sd_cpp_direct_backend_runs_fake_binary_and_saves_png() {
    use std::os::unix::fs::PermissionsExt;

    let root = root();
    let model = root.join("model.gguf");
    std::fs::write(&model, b"GGUF").unwrap();
    let png_bytes = STANDARD.decode(PNG).unwrap();
    let src_png = root.join("fixture.png");
    std::fs::write(&src_png, &png_bytes).unwrap();

    let fake_sd = root.join("fake-sd-cli.sh");
    std::fs::write(
        &fake_sd,
        format!(
            "#!/bin/sh\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"-o\" ]; then OUT=\"$2\"; shift 2; continue; fi\n  shift\ndone\ncp '{}' \"$OUT\"\n",
            src_png.display()
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&fake_sd).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_sd, perms).unwrap();

    let req = ImageRequest {
        backend: ImageBackendKind::StableDiffusionCpp,
        endpoint: String::new(),
        prompt: "A red fox in snow".into(),
        negative_prompt: String::new(),
        checkpoint: String::new(),
        style: Some(ImageStyle::Sketch),
        sd_cli_path: Some(fake_sd.to_string_lossy().into_owned()),
        model_path: Some(model.to_string_lossy().into_owned()),
        width: 512,
        height: 512,
        steps: 8,
    };
    let rel = generate_workspace_image(&root, "assets/fox.png", &req)
        .await
        .unwrap();
    assert_eq!(rel, "assets/fox.png");
    assert_eq!(
        std::fs::read(root.join("assets/fox.png")).unwrap(),
        png_bytes
    );
    assert!(generate_workspace_image(&root, "../escape.png", &req)
        .await
        .is_err());
    std::fs::remove_dir_all(root).unwrap();
}
