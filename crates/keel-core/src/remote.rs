//! Explicit remote text-provider connect rules and API-key storage.
//!
//! Non-loopback endpoints require user opt-in. Optional Bearer keys are kept
//! outside DesktopConfig (never in config.json) under data_dir/secrets/.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::runtime::{is_default_managed_loopback, is_loopback_url};

/// Rejects Keel's own managed loopback and non-loopback hosts without opt-in.
pub fn validate_external_connect(base_url: &str, confirm_remote: bool) -> Result<()> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        bail!("Enter a server address first.");
    }
    if is_default_managed_loopback(trimmed) {
        bail!(
            "That address is Keel's own built-in AI engine. Pick a downloaded model in Models instead, or connect Ollama (11434) or LM Studio (1234)."
        );
    }
    let parsed =
        reqwest::Url::parse(trimmed).context("Enter a valid http:// or https:// server URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        bail!("Server URL must use http:// or https://");
    }
    if !is_loopback_url(trimmed) && !confirm_remote {
        bail!(
            "This server is not on this computer. Confirm that you want to send prompts there (it may leave this computer and may incur provider cost)."
        );
    }
    Ok(())
}

pub fn external_api_key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("secrets").join("external_api_key")
}

/// Loads a previously saved external API key, if any.
pub fn load_external_api_key(data_dir: &Path) -> Option<String> {
    let path = external_api_key_path(data_dir);
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Saves or clears the external API key. Never write this into DesktopConfig.
pub fn save_external_api_key(data_dir: &Path, key: Option<&str>) -> Result<()> {
    let path = external_api_key_path(data_dir);
    let trimmed = key.map(str::trim).filter(|s| !s.is_empty());
    match trimmed {
        None => {
            if path.is_file() {
                std::fs::remove_file(&path).context("Could not clear saved API key")?;
            }
            Ok(())
        }
        Some(value) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).context("Could not create secrets folder")?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
                }
            }
            write_private_file(&path, value.as_bytes())
                .context("Could not save API key outside config")?;
            Ok(())
        }
    }
}

/// Removes any saved external API key (used when disconnecting).
pub fn clear_external_api_key(data_dir: &Path) -> Result<()> {
    save_external_api_key(data_dir, None)
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

/// Builds `Authorization: Bearer <key>` when a non-empty key is provided.
pub fn bearer_authorization(api_key: Option<&str>) -> Option<String> {
    api_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|key| format!("Bearer {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::{load_config, save_config, DesktopConfig};

    fn temp_dir(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("keel_remote_{label}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn loopback_connect_without_opt_in_is_ok() {
        validate_external_connect("http://127.0.0.1:11434/v1", false).unwrap();
        validate_external_connect("http://localhost:1234/v1", false).unwrap();
        validate_external_connect("http://[::1]:8080/v1", false).unwrap();
    }

    #[test]
    fn non_loopback_without_opt_in_fails() {
        let err = validate_external_connect("https://models.example.com/v1", false)
            .unwrap_err()
            .to_string();
        assert!(
            err.to_lowercase().contains("not on this computer"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn non_loopback_with_opt_in_ok() {
        validate_external_connect("https://models.example.com/v1", true).unwrap();
    }

    #[test]
    fn api_key_never_appears_in_saved_config_json() {
        let root = temp_dir("key");
        let config_dir = root.join("config");
        let data_dir = root.join("data");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();

        let cfg = DesktopConfig {
            external_base_url: Some("https://models.example.com/v1".into()),
            external_model_name: Some("remote-model".into()),
            ..Default::default()
        };
        save_config(&config_dir, &cfg).unwrap();
        save_external_api_key(&data_dir, Some("sk-secret-should-not-leak")).unwrap();

        let raw = std::fs::read_to_string(config_dir.join("config.json")).unwrap();
        assert!(
            !raw.contains("sk-secret"),
            "API key leaked into config.json: {raw}"
        );
        assert!(!raw.to_lowercase().contains("api_key"));
        let loaded = load_config(&config_dir);
        assert_eq!(
            loaded.external_base_url.as_deref(),
            Some("https://models.example.com/v1")
        );
        assert_eq!(
            load_external_api_key(&data_dir).as_deref(),
            Some("sk-secret-should-not-leak")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(external_api_key_path(&data_dir))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "secret file mode was {mode:o}");
        }

        clear_external_api_key(&data_dir).unwrap();
        assert!(load_external_api_key(&data_dir).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bearer_header_only_when_key_present() {
        assert_eq!(bearer_authorization(None), None);
        assert_eq!(bearer_authorization(Some("")), None);
        assert_eq!(
            bearer_authorization(Some("  sk-test  ")).as_deref(),
            Some("Bearer sk-test")
        );
    }
}
