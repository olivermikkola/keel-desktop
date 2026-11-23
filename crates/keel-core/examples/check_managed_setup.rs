//! Opt-in setup smoke check: downloads the pinned runner and starts a supplied GGUF.
use anyhow::{Context, Result};
#[tokio::main]
async fn main() -> Result<()> {
    let model = std::env::var("KEEL_EVAL_MODEL_PATH")
        .context("Set KEEL_EVAL_MODEL_PATH to an existing GGUF")?;
    let directory = std::env::temp_dir().join(format!("keel_setup_{}", uuid::Uuid::new_v4()));
    let runner = keel_core::setup::install_runtime(&directory, |_| {}).await?;
    let mut runtime = keel_core::runtime::RuntimeManager::new(19182);
    let status = runtime
        .start_model(keel_core::runtime::ModelLaunch {
            model_id: "setup-smoke",
            model_path: std::path::Path::new(&model),
            context_size: 4096,
            custom_bin: runner.to_str(),
            offload_args: &[],
            run_where: "processor",
            graphics_share: "auto",
            settings_bin: None,
        })
        .await?;
    anyhow::ensure!(status.state == "running", "Expected a live managed runtime");
    let models = keel_core::runtime::probe_openai_models(&status.base_url).await?;
    anyhow::ensure!(
        models.contains(&"setup-smoke".to_string()),
        "Model alias did not match"
    );
    runtime.stop().await;
    anyhow::ensure!(runtime.status().state == "stopped", "Runtime did not stop");
    println!("PASS: downloaded and checksum-verified the pinned runner, extracted it, loaded a real GGUF, exposed the expected model alias, and stopped the process cleanly.");
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
