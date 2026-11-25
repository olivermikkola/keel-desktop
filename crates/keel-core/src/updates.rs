use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const LATEST_RELEASE_API_URL: &str =
    "https://api.github.com/repos/olivermikkola/keel-desktop/releases/latest";
pub const RELEASES_PAGE_URL: &str = "https://github.com/olivermikkola/keel-desktop/releases/latest";

/// Result of checking whether a newer Keel Desktop version is published.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppUpdateInfo {
    pub current_version: String,
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub release_url: String,
    /// True on macOS, Windows, and Linux AppImage where Tauri's signed updater can
    /// replace the running app in place and relaunch. False on `.deb`/`.rpm`/dev builds.
    pub can_install_in_place: bool,
}

#[derive(Debug, Deserialize)]
struct GithubReleasePayload {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

/// True when Tauri's in-place signed updater can replace the running executable on this OS.
pub fn can_install_in_place_on_this_host() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("APPIMAGE").is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// Compares semantic versions (`0.7.0`, `v0.7.1`, `0.8.0-rc.1`).
/// Returns `true` strictly when `candidate` is newer than `current`.
pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    let Some((c_core, c_pre)) = parse_semver(current) else {
        return false;
    };
    let Some((n_core, n_pre)) = parse_semver(candidate) else {
        return false;
    };
    if n_core != c_core {
        return n_core > c_core;
    }
    // Stable release (no prerelease tag) is newer than a prerelease of the same core version.
    match (c_pre.is_empty(), n_pre.is_empty()) {
        (false, true) => true,
        (true, false) => false,
        _ => n_pre > c_pre,
    }
}

fn parse_semver(raw: &str) -> Option<([u64; 3], &str)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    if trimmed.is_empty() {
        return None;
    }
    let (core, pre) = match trimmed.split_once('-') {
        Some((c, p)) => (c, p),
        None => (trimmed, ""),
    };
    let mut parts = core.split('.');
    let major: u64 = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next().unwrap_or("0").parse().ok()?;
    let patch: u64 = parts.next().unwrap_or("0").parse().ok()?;
    Some(([major, minor, patch], pre))
}

pub fn parse_github_latest_release(current_version: &str, body: &[u8]) -> Result<AppUpdateInfo> {
    let payload: GithubReleasePayload =
        serde_json::from_slice(body).context("Unexpected response from GitHub Releases")?;
    let latest = payload.tag_name.trim().trim_start_matches(['v', 'V']);
    let available = is_newer_version(current_version, latest);
    let release_url = payload
        .html_url
        .filter(|u| u.starts_with("https://github.com/"))
        .unwrap_or_else(|| RELEASES_PAGE_URL.to_string());
    Ok(AppUpdateInfo {
        current_version: current_version.to_string(),
        available,
        version: available.then(|| latest.to_string()),
        notes: available.then_some(payload.body).flatten(),
        release_url,
        can_install_in_place: false,
    })
}

/// Checks GitHub's `/releases/latest` endpoint as a fallback when no signed
/// `latest.json` updater manifest is available for this platform or release.
pub async fn fetch_github_latest_release(current_version: &str) -> Result<AppUpdateInfo> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(format!("keel-desktop/{current_version}"))
        .build()?;
    let resp = client
        .get(LATEST_RELEASE_API_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("Could not reach GitHub to check for updates")?;
    if !resp.status().is_success() {
        anyhow::bail!("GitHub update check returned HTTP {}", resp.status());
    }
    let bytes = resp.bytes().await?;
    parse_github_latest_release(current_version, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_cleanly() {
        assert!(is_newer_version("0.7.0", "0.7.1"));
        assert!(is_newer_version("0.7.0", "v0.8.0"));
        assert!(is_newer_version("0.7.0", "1.0.0"));
        assert!(is_newer_version("0.7.1-rc.1", "0.7.1"));
        assert!(!is_newer_version("0.7.0", "0.7.0"));
        assert!(!is_newer_version("0.7.1", "0.7.0"));
        assert!(!is_newer_version("0.7.1", "0.7.1-rc.1"));
        assert!(!is_newer_version("0.7.0", "not-a-version"));
    }

    #[test]
    fn parses_github_latest_release_payload() {
        let json = br#"{
            "tag_name": "v0.8.0",
            "body": "In-app updates and polish.",
            "html_url": "https://github.com/olivermikkola/keel-desktop/releases/tag/v0.8.0"
        }"#;
        let info = parse_github_latest_release("0.7.0", json).unwrap();
        assert!(info.available);
        assert_eq!(info.version.as_deref(), Some("0.8.0"));
        assert_eq!(info.notes.as_deref(), Some("In-app updates and polish."));
        assert!(!info.can_install_in_place);
        assert!(info.release_url.ends_with("/tag/v0.8.0"));

        let same = parse_github_latest_release("0.8.0", json).unwrap();
        assert!(!same.available);
        assert!(same.version.is_none());
    }
}
