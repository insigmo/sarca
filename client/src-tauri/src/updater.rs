//! Checking for, and installing, a newer client from the project's GitHub releases.
//!
//! Not `tauri-plugin-updater`: that wants a `latest.json` manifest signed with a
//! minisign key held by CI, and this project publishes plain installers. What it
//! does publish is one artifact per platform with a predictable name, so the
//! check is "ask the releases API for the newest tag, look for this platform's
//! file in it".
//!
//! Installing hands the downloaded file to the platform's own installer and
//! stops there. Nothing here replaces files inside the running app: on every
//! platform the shipped format (`.exe` setup, `.dmg`, `.deb`, `.apk`) already
//! knows how to upgrade in place, and the installer is also what asks the user
//! for the elevation the update genuinely needs.

use std::{path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};

use crate::client_log;

const DEFAULT_REPO: &str = "insigmo/sarca";
const REPO_VAR: &str = "SARCA_UPDATE_REPO";
const API_BASE_VAR: &str = "SARCA_UPDATE_API_BASE";
const DEFAULT_API_BASE: &str = "https://api.github.com";

const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Ceiling on a downloaded installer. Far above any real one; it is here so a
/// wrong URL cannot fill the user's disk.
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;

/// GitHub 403s an API request without one.
const USER_AGENT: &str = concat!("sarca-client/", env!("CARGO_PKG_VERSION"));

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize)]
pub struct UpdateStatusDto {
    pub current: String,
    pub latest: Option<String>,
    pub update_available: bool,
    pub notes: String,
    /// Release asset for this platform, when the release carries one.
    pub asset: Option<String>,
    pub download_url: Option<String>,
    /// False when this platform's update is a manual download rather than
    /// something the app can start itself.
    pub can_install: bool,
    /// Why not, when `can_install` is false.
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InstallStartedDto {
    pub version: String,
    /// Where the downloaded installer was put, so the user can find it if the
    /// handoff to the installer did not surface a window.
    pub path: String,
    /// True when the app handed the file to the platform installer. False means
    /// the download is all that happened and the path is the deliverable.
    pub launched: bool,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    #[serde(default)]
    name: String,
    #[serde(default)]
    browser_download_url: String,
}

fn repo() -> String {
    std::env::var(REPO_VAR)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_REPO.to_owned())
}

fn api_base() -> String {
    std::env::var(API_BASE_VAR)
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_owned())
}

/// The release asset built for this platform, matching the names the release
/// workflow publishes.
///
/// `None` on iOS: distribution there goes through the App Store or a signed
/// sideload, and nothing the app downloads can install itself.
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("sarca_client_windows_amd64-setup.exe"),
        ("windows", "aarch64") => Some("sarca_client_windows_arm64-setup.exe"),
        ("macos", "aarch64") => Some("sarca_client_macos_arm64.dmg"),
        ("macos", "x86_64") => Some("sarca_client_macos_amd64.dmg"),
        ("linux", "x86_64") => Some("sarca_client_linux_amd64.deb"),
        ("linux", "aarch64") => Some("sarca_client_linux_arm64.deb"),
        ("android", _) => Some("sarca_client_android_arm64.apk"),
        _ => None,
    }
}

/// Numeric version comparison; see the twin in the server's `services::update`.
/// `"0.0.9" > "0.0.170"` lexically, so a string compare would hide real releases.
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(raw: &str) -> Vec<u64> {
        raw.trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .take_while(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    }
    let (a, b) = (parts(latest), parts(current));
    if a.is_empty() {
        return false;
    }
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            b.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x > y;
        }
    }
    false
}

/// Whether this build can start its own update.
///
/// Android is the interesting one: installing an APK needs
/// `REQUEST_INSTALL_PACKAGES` and a `FileProvider` grant, neither of which this
/// app declares, so the honest answer is "download it and let the system's own
/// download/install flow take over".
fn install_blocker() -> Option<String> {
    match std::env::consts::OS {
        "windows" | "macos" | "linux" => None,
        "android" => Some("install the downloaded APK from your notifications".to_owned()),
        other => Some(format!("automatic updates are not available on {other}")),
    }
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .map_err(|e| format!("could not start the update check: {e}"))
}

async fn latest_release() -> Result<GithubRelease, String> {
    let url = format!("{}/repos/{}/releases/latest", api_base(), repo());
    let resp = client(CHECK_TIMEOUT)?
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("could not reach GitHub: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub answered {}", resp.status()));
    }
    resp.json::<GithubRelease>()
        .await
        .map_err(|e| format!("could not read the release: {e}"))
}

pub async fn check(data_dir: &std::path::Path) -> Result<UpdateStatusDto, String> {
    client_log::debug_line(data_dir, "update check: asking GitHub for the latest release");
    let release = latest_release().await?;
    let latest = release.tag_name.trim().to_owned();
    let wanted = asset_name();
    let found = wanted.and_then(|name| release.assets.iter().find(|a| a.name == name));
    let blocker = install_blocker().or_else(|| {
        found
            .is_none()
            .then(|| match wanted {
                Some(name) => format!("this release does not include {name}"),
                None => format!(
                    "no client build is published for {}",
                    std::env::consts::OS
                ),
            })
    });
    let update_available = !latest.is_empty() && is_newer(&latest, CURRENT_VERSION);
    client_log::write_line(
        data_dir,
        &format!(
            "update check: current={CURRENT_VERSION} latest={latest} available={update_available}"
        ),
    );

    Ok(UpdateStatusDto {
        current: CURRENT_VERSION.to_owned(),
        update_available,
        latest: (!latest.is_empty()).then_some(latest),
        notes: release.body.unwrap_or_default().chars().take(4000).collect(),
        asset: found.map(|a| a.name.clone()),
        download_url: found.map(|a| a.browser_download_url.clone()),
        can_install: blocker.is_none() && found.is_some(),
        reason: blocker,
    })
}

/// Download the newest installer and hand it to the platform.
///
/// The download lands in the app's own data directory rather than a temp path:
/// a failed handoff should leave something the user can still double-click, and
/// a temp file that the OS reaps an hour later is not that.
pub async fn install(data_dir: &std::path::Path) -> Result<InstallStartedDto, String> {
    let status = check(data_dir).await?;
    if !status.update_available {
        return Err("this client is already on the newest release".to_owned());
    }
    let (Some(url), Some(name)) = (status.download_url.as_deref(), status.asset.as_deref()) else {
        return Err(status
            .reason
            .unwrap_or_else(|| "there is nothing to install for this platform".to_owned()));
    };

    // Same reasoning as the server's twin: the URL comes out of the API
    // response, and this file is about to be executed on the user's machine.
    if !url.starts_with("https://") {
        return Err("the release asset is not served over https".to_owned());
    }

    let dir = data_dir.join("updates");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    // One update at a time, and never a half-written file from a previous try.
    let dest = dir.join(name);
    let _ = std::fs::remove_file(&dest);

    client_log::write_line(data_dir, &format!("update: downloading {url}"));
    download(url, &dest).await.inspect_err(|e| {
        client_log::write_line(data_dir, &format!("update: download failed: {e}"));
        let _ = std::fs::remove_file(&dest);
    })?;

    let version = status.latest.unwrap_or_else(|| CURRENT_VERSION.to_owned());
    let launched = launch_installer(&dest);
    client_log::write_line(
        data_dir,
        &format!(
            "update: downloaded {} ({} bytes), launched={launched}",
            dest.display(),
            std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0)
        ),
    );

    Ok(InstallStartedDto {
        version,
        path: dest.display().to_string(),
        launched,
    })
}

async fn download(url: &str, dest: &PathBuf) -> Result<(), String> {
    let mut resp = client(DOWNLOAD_TIMEOUT)?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("the download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("the download answered {}", resp.status()));
    }

    let mut file =
        std::fs::File::create(dest).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    let mut written: u64 = 0;
    // `chunk()` rather than a stream adapter: an installer is tens of megabytes
    // and must not be buffered whole in memory, and this needs no extra crate.
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("the download broke off: {e}"))?
    {
        written += chunk.len() as u64;
        if written > MAX_INSTALLER_BYTES {
            return Err("the download was larger than expected".to_owned());
        }
        std::io::Write::write_all(&mut file, &chunk)
            .map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    }
    std::io::Write::flush(&mut file).map_err(|e| e.to_string())?;
    Ok(())
}

/// Start the platform's installer for `path`, returning whether it went.
///
/// Best-effort on purpose: a desktop session with no handler registered for
/// `.deb`, or an `xdg-open` that is simply not installed, must not turn a
/// completed download into a failed update. The caller reports the path either
/// way, so the user can still open it themselves.
fn launch_installer(path: &std::path::Path) -> bool {
    if install_blocker().is_some() {
        return false;
    }
    // Windows runs the NSIS setup directly; the other two hand the file to the
    // desktop, which is what knows how to mount a `.dmg` or open a `.deb` in
    // the package installer — and which asks for the elevation an install needs.
    let mut command = match std::env::consts::OS {
        "windows" => std::process::Command::new(path),
        "macos" => {
            let mut c = std::process::Command::new("open");
            c.arg(path);
            c
        },
        _ => {
            let mut c = std::process::Command::new("xdg-open");
            c.arg(path);
            c
        },
    };
    match command.spawn() {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "could not launch the installer");
            false
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Same trap as the server's: this project is on 0.0.170, and a lexical
    /// comparison calls 0.0.9 newer than all of it.
    #[test]
    fn versions_compare_numerically_not_lexically() {
        assert!(is_newer("v0.0.171", "0.0.170"));
        assert!(!is_newer("0.0.9", "0.0.170"));
        assert!(!is_newer("v0.0.170", "0.0.170"));
        assert!(is_newer("0.1.0", "0.0.999"));
    }

    #[test]
    fn a_tag_without_a_version_is_never_newer() {
        assert!(!is_newer("nightly", "0.0.170"));
        assert!(!is_newer("", "0.0.170"));
    }

    /// The names have to be the ones the release workflow writes, or the check
    /// finds nothing in a release that does contain a build for this platform.
    #[test]
    fn asset_names_match_the_release_workflow() {
        if let Some(name) = asset_name() {
            assert!(name.starts_with("sarca_client_"), "{name}");
            assert!(
                name.ends_with("-setup.exe")
                    || name.ends_with(".dmg")
                    || name.ends_with(".deb")
                    || name.ends_with(".apk"),
                "{name}"
            );
        }
    }
}
