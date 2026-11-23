//! Explicit remote text-provider connect rules and API-key storage.
//!
//! Non-loopback endpoints require user opt-in. Optional Bearer keys are kept
//! outside DesktopConfig (never in config.json) under data_dir/secrets/.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
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
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("Enter a base URL without credentials, a query, or a fragment");
    }
    if !is_loopback_url(trimmed) && !confirm_remote {
        bail!(
            "This server is not on this computer. Confirm that you want to send prompts there (it may leave this computer and may incur provider cost)."
        );
    }
    Ok(())
}

pub fn validate_external_key_transport(base_url: &str) -> Result<()> {
    let url = reqwest::Url::parse(base_url)?;
    if url.scheme() != "https" && !is_loopback_url(base_url) {
        bail!("Use an HTTPS address to connect to a remote server with an API key.");
    }
    Ok(())
}

pub fn external_api_key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("secrets").join("external_api_key")
}

#[derive(Deserialize, Serialize)]
struct ExternalCredential<'a> {
    base_url: std::borrow::Cow<'a, str>,
    key: std::borrow::Cow<'a, str>,
}

fn endpoint_identity(base_url: &str) -> &str {
    base_url.trim().trim_end_matches('/')
}

/// Only return a credential associated with this exact server and API path.
pub fn load_external_api_key(data_dir: &Path, base_url: &str) -> Option<String> {
    let raw = std::fs::read_to_string(external_api_key_path(data_dir)).ok()?;
    let credential: ExternalCredential = serde_json::from_str(&raw).ok()?;
    (credential.base_url == endpoint_identity(base_url)).then(|| credential.key.into_owned())
}

/// Save the endpoint and key together so concurrent reads cannot mix them.
pub fn save_external_api_key(data_dir: &Path, base_url: &str, key: Option<&str>) -> Result<()> {
    let Some(key) = key.map(str::trim).filter(|key| !key.is_empty()) else {
        return clear_external_api_key(data_dir);
    };
    let path = external_api_key_path(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("Could not create secrets folder")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let record = ExternalCredential {
        base_url: endpoint_identity(base_url).into(),
        key: key.into(),
    };
    let bytes = serde_json::to_vec(&record)?;
    write_private_file(&path, &bytes).context("Could not save API key outside config")
}

pub fn clear_external_api_key(data_dir: &Path) -> Result<()> {
    match std::fs::remove_file(external_api_key_path(data_dir)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result.context("Could not clear saved API key"),
    }
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = opts.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result?;
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
        let provider = "https://models.example.com/v1";
        save_external_api_key(&data_dir, provider, Some("sk-secret-should-not-leak")).unwrap();

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
            load_external_api_key(&data_dir, provider).as_deref(),
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

        assert_eq!(
            load_external_api_key(&data_dir, " https://models.example.com/v1/ "),
            load_external_api_key(&data_dir, provider)
        );
        assert!(load_external_api_key(&data_dir, "https://other.example.com/v1").is_none());
        assert!(load_external_api_key(&data_dir, "https://models.example.com/other").is_none());
        save_external_api_key(&data_dir, "https://other.example.com/v1", Some("other-key"))
            .unwrap();
        assert!(load_external_api_key(&data_dir, provider).is_none());
        assert_eq!(
            load_external_api_key(&data_dir, "https://other.example.com/v1").as_deref(),
            Some("other-key")
        );

        clear_external_api_key(&data_dir).unwrap();
        assert!(!external_api_key_path(&data_dir).exists());
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
