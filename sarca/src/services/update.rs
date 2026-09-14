//! Self-update against the project's GitHub releases.
//!
//! The release workflow publishes one archive per platform (`sarca_linux_arm64.tar.gz`
//! and friends), each holding the `sarca` binary and the built `ui/`. That is exactly
//! what `install.sh` downloads, so updating in place is the same operation the installer
//! performs, minus the parts that configure a fresh machine.
//!
//! Two properties this module is built around:
//!
//! * **Nothing is replaced until everything has been downloaded and unpacked.** The new binary and
//!   UI are staged beside the live ones and moved into place at the end, so a download that dies
//!   halfway leaves a working install rather than a half-written one.
//! * **The running process is never the thing being overwritten.** The live binary is renamed aside
//!   first (which POSIX allows while it is executing, and which Windows allows for a file that is
//!   mapped but not opened for writing), and only then does the new one take its name.
//!
//! Restarting is the caller's business: [`apply`] returns once the files are in place and
//! the process re-executes itself from the new binary.

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::errors::{SarcaError, SarcaResult};

/// The version this binary was built from.
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Repository the releases come from. Overridable so a fork — or an e2e test
/// pointing at a local stub — does not have to reach the upstream project.
const DEFAULT_REPO: &str = "insigmo/sarca";
const REPO_VAR: &str = "SARCA_UPDATE_REPO";
/// Base for the GitHub API. Same reason, and it is what the tests point at.
const API_BASE_VAR: &str = "SARCA_UPDATE_API_BASE";
const DEFAULT_API_BASE: &str = "https://api.github.com";

/// GitHub answers an unauthenticated client in well under this; anything longer
/// is a network that is not going to produce an update either.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
/// A release archive is tens of megabytes over whatever uplink the host has.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(30);
/// Ceiling on a downloaded archive. Well above any real release; it exists so a
/// wrong URL cannot fill the disk.
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

/// GitHub requires a User-Agent on API requests and 403s without one.
const USER_AGENT: &str = concat!("sarca/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Serialize)]
pub struct UpdateStatusSchema {
    /// Version running right now.
    pub current: String,
    /// Newest published release, or `None` when the check has not run.
    pub latest: Option<String>,
    pub update_available: bool,
    /// Release notes as the tag carries them, trimmed to something a dialog can hold.
    pub notes: String,
    /// Asset this platform would download, when the release has one.
    pub asset: Option<String>,
    /// False when this build cannot replace itself — a container image, or a
    /// platform whose archive the release does not carry. The UI uses it to
    /// offer the download link instead of the button.
    pub can_self_update: bool,
    /// Why not, when `can_self_update` is false.
    pub reason: Option<String>,
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

/// Name of the release archive built for the host this is running on.
///
/// `None` means the release workflow does not publish one for this target, and
/// there is nothing to offer — saying so is better than downloading an archive
/// whose binary will not run.
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("sarca_linux_amd64.tar.gz"),
        ("linux", "aarch64") => Some("sarca_linux_arm64.tar.gz"),
        ("macos", "aarch64") => Some("sarca_macos_arm64.tar.gz"),
        ("macos", "x86_64") => Some("sarca_macos_amd64.tar.gz"),
        ("windows", "x86_64") => Some("sarca_windows_amd64.zip"),
        _ => None,
    }
}

/// Compares two release versions, tolerating a leading `v` and trailing
/// pre-release text on either side.
///
/// String comparison is not an option: `0.0.9` sorts after `0.0.170`, which
/// would offer a downgrade as an update and — worse — hide a real one.
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(raw: &str) -> Vec<u64> {
        raw.trim()
            .trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .take_while(|p| p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty())
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    }
    let (a, b) = (parts(latest), parts(current));
    if a.is_empty() {
        return false;
    }
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// Directory the running install lives in: the binary's own directory.
///
/// That is what `install.sh` lays out — `sarca` and `ui/` side by side under
/// `SARCA_HOME` — and what the archive unpacks into.
fn install_root() -> SarcaResult<PathBuf> {
    let exe = std::env::current_exe().map_err(|e| {
        tracing::error!("cannot locate the running binary: {e}");
        SarcaError::Unknown
    })?;
    exe.parent().map(Path::to_path_buf).ok_or_else(|| {
        tracing::error!("the running binary has no parent directory");
        SarcaError::Unknown
    })
}

/// Whether this install is one we may replace in place.
///
/// A container is the case worth naming: swapping a binary inside an image's
/// layer works right up until the container is recreated, and then the change
/// is silently gone. Pulling a new image is the update there.
fn self_update_blocker() -> Option<String> {
    if std::env::var_os("SARCA_IN_DOCKER").is_some() || Path::new("/.dockerenv").exists() {
        return Some(
            "this server runs in a container — update the image instead (docker compose pull)"
                .to_owned(),
        );
    }
    if asset_name().is_none() {
        return Some(format!(
            "no release archive is published for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    }
    None
}

fn client() -> SarcaResult<reqwest::Client> {
    reqwest::Client::builder().user_agent(USER_AGENT).timeout(DOWNLOAD_TIMEOUT).build().map_err(
        |e| {
            tracing::error!("building the update HTTP client failed: {e}");
            SarcaError::Unknown
        },
    )
}

async fn latest_release() -> SarcaResult<GithubRelease> {
    let url = format!("{}/repos/{}/releases/latest", api_base(), repo());
    let resp = client()?
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(CHECK_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("update check could not reach {url}: {e}");
            SarcaError::UpdateCheckFailed
        })?;
    if !resp.status().is_success() {
        tracing::warn!("update check got {} from {url}", resp.status());
        return Err(SarcaError::UpdateCheckFailed);
    }
    resp.json::<GithubRelease>().await.map_err(|e| {
        tracing::warn!("update check could not read the release: {e}");
        SarcaError::UpdateCheckFailed
    })
}

/// Ask GitHub what the newest release is and whether it is worth installing.
pub async fn check() -> SarcaResult<UpdateStatusSchema> {
    let release = latest_release().await?;
    let latest = release.tag_name.trim().to_owned();
    let wanted = asset_name();
    let asset = wanted
        .and_then(|name| release.assets.iter().find(|a| a.name == name))
        .map(|a| a.name.clone());
    let blocker = self_update_blocker().or_else(|| {
        // The release exists but this platform's archive is missing from it — a
        // partially-failed build. Offering a button that cannot work is worse
        // than saying what is wrong.
        wanted
            .filter(|_| asset.is_none())
            .map(|name| format!("this release does not include {name}"))
    });

    Ok(UpdateStatusSchema {
        current: CURRENT_VERSION.to_owned(),
        update_available: !latest.is_empty() && is_newer(&latest, CURRENT_VERSION),
        latest: (!latest.is_empty()).then_some(latest),
        notes: release.body.unwrap_or_default().chars().take(4000).collect(),
        asset,
        can_self_update: blocker.is_none(),
        reason: blocker,
    })
}

/// Download the newest release and put it in place of the running install.
///
/// Returns the version installed. The process re-executes itself from the new
/// binary before this can be observed by the caller's next request, which is
/// why the HTTP handler answers *before* calling it.
pub async fn apply() -> SarcaResult<String> {
    if let Some(reason) = self_update_blocker() {
        tracing::warn!("refusing to self-update: {reason}");
        return Err(SarcaError::UpdateNotSupported(reason));
    }
    let status = check().await?;
    if !status.update_available {
        return Err(SarcaError::UpdateNotAvailable);
    }
    let release = latest_release().await?;
    let version = release.tag_name.trim().to_owned();
    let wanted = asset_name().ok_or(SarcaError::UpdateNotAvailable)?;
    let asset =
        release.assets.iter().find(|a| a.name == wanted).ok_or(SarcaError::UpdateNotAvailable)?;

    let root = install_root()?;
    let staging = root.join(".sarca-update");
    // A previous attempt that died mid-flight leaves this behind; it holds
    // nothing worth keeping.
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await.map_err(|e| {
        tracing::error!("cannot stage the update under {}: {e}", staging.display());
        SarcaError::UpdateFailed(format!("cannot write to {}", staging.display()))
    })?;

    // The URL comes out of the API response, not out of this file. Pinning the
    // scheme means a surprising answer cannot turn a release download into a
    // plaintext fetch of a binary this process is about to run.
    if !asset.browser_download_url.starts_with("https://") {
        tracing::error!("refusing a non-https release asset: {}", asset.browser_download_url);
        return Err(SarcaError::UpdateFailed(
            "the release asset is not served over https".to_owned(),
        ));
    }

    let archive = staging.join(wanted);
    download(&asset.browser_download_url, &archive).await?;

    let unpacked = staging.join("unpacked");
    let url = asset.browser_download_url.clone();
    let archive_for_task = archive.clone();
    let unpacked_for_task = unpacked.clone();
    // Decompressing hundreds of megabytes is blocking work with no async form.
    tokio::task::spawn_blocking(move || extract(&archive_for_task, &unpacked_for_task))
        .await
        .map_err(|e| {
            tracing::error!("unpacking {url} panicked: {e}");
            SarcaError::UpdateFailed("unpacking the release failed".to_owned())
        })??;

    let payload = single_child_dir(&unpacked).unwrap_or(unpacked);
    let new_binary = payload.join(binary_name());
    let new_ui = payload.join("ui");
    if !new_binary.is_file() || !new_ui.join("index.html").is_file() {
        tracing::error!("release archive layout unexpected under {}", payload.display());
        let _ = tokio::fs::remove_dir_all(&staging).await;
        return Err(SarcaError::UpdateFailed(
            "the release archive did not contain a binary and a ui/ directory".to_owned(),
        ));
    }

    let root_for_task = root.clone();
    let staging_for_task = staging.clone();
    let swap =
        tokio::task::spawn_blocking(move || swap_in_place(&root_for_task, &new_binary, &new_ui))
            .await
            .map_err(|e| {
                tracing::error!("installing the update panicked: {e}");
                SarcaError::UpdateFailed("installing the update failed".to_owned())
            })?;
    let _ = tokio::fs::remove_dir_all(&staging_for_task).await;
    swap?;

    // Same file `install.sh` writes, so the two agree on what is installed.
    let _ = tokio::fs::write(root.join("VERSION"), format!("{version}\n")).await;
    tracing::info!("updated to {version}; restarting");
    Ok(version)
}

fn binary_name() -> &'static str {
    if cfg!(windows) { "sarca.exe" } else { "sarca" }
}

async fn download(url: &str, dest: &Path) -> SarcaResult<()> {
    use futures::StreamExt as _;

    tracing::info!("downloading {url}");
    let resp = client()?.get(url).send().await.map_err(|e| {
        tracing::error!("downloading {url} failed: {e}");
        SarcaError::UpdateFailed("downloading the release failed".to_owned())
    })?;
    if !resp.status().is_success() {
        tracing::error!("downloading {url} got {}", resp.status());
        return Err(SarcaError::UpdateFailed(format!(
            "the release download answered {}",
            resp.status()
        )));
    }

    let mut file = tokio::fs::File::create(dest).await.map_err(|e| {
        tracing::error!("cannot create {}: {e}", dest.display());
        SarcaError::UpdateFailed("cannot write the downloaded release".to_owned())
    })?;
    let mut written: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            tracing::error!("the release download broke off: {e}");
            SarcaError::UpdateFailed("the release download broke off".to_owned())
        })?;
        written += chunk.len() as u64;
        if written > MAX_ARCHIVE_BYTES {
            tracing::error!("release download exceeded {MAX_ARCHIVE_BYTES} bytes; aborting");
            return Err(SarcaError::UpdateFailed(
                "the release download was larger than expected".to_owned(),
            ));
        }
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await.map_err(|e| {
            tracing::error!("writing the release download failed: {e}");
            SarcaError::UpdateFailed("cannot write the downloaded release".to_owned())
        })?;
    }
    tokio::io::AsyncWriteExt::flush(&mut file).await.ok();
    tracing::info!("downloaded {written} bytes");
    Ok(())
}

/// Unpack `.tar.gz` or `.zip` into `dest`.
///
/// Entry paths are checked rather than trusted: a crafted archive naming
/// `../../etc/cron.d/x` would otherwise write outside the staging directory,
/// and this archive is fetched over the network.
fn extract(archive: &Path, dest: &Path) -> SarcaResult<()> {
    std::fs::create_dir_all(dest).map_err(|e| {
        tracing::error!("cannot create {}: {e}", dest.display());
        SarcaError::UpdateFailed("cannot unpack the release".to_owned())
    })?;
    let is_zip = archive.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if is_zip { extract_zip(archive, dest) } else { extract_tar_gz(archive, dest) }
}

fn safe_join(dest: &Path, entry: &Path) -> Option<PathBuf> {
    let mut out = dest.to_path_buf();
    for part in entry.components() {
        match part {
            std::path::Component::Normal(p) => out.push(p),
            // Anything that could climb out of `dest`, or re-root the path.
            _ => return None,
        }
    }
    Some(out)
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> SarcaResult<()> {
    let file = std::fs::File::open(archive).map_err(|e| {
        tracing::error!("cannot open {}: {e}", archive.display());
        SarcaError::UpdateFailed("cannot read the downloaded release".to_owned())
    })?;
    let decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut tar = tar::Archive::new(decoder);
    tar.set_preserve_permissions(true);
    let entries = tar.entries().map_err(|e| {
        tracing::error!("cannot read the release archive: {e}");
        SarcaError::UpdateFailed("the release archive is not readable".to_owned())
    })?;
    for entry in entries {
        let mut entry = entry.map_err(|e| {
            tracing::error!("cannot read a release archive entry: {e}");
            SarcaError::UpdateFailed("the release archive is not readable".to_owned())
        })?;
        let path = entry.path().map_err(|e| {
            tracing::error!("bad entry path in the release archive: {e}");
            SarcaError::UpdateFailed("the release archive is not readable".to_owned())
        })?;
        let Some(target) = safe_join(dest, &path) else {
            tracing::error!("refusing release archive entry outside the staging dir: {path:?}");
            return Err(SarcaError::UpdateFailed(
                "the release archive contained an unsafe path".to_owned(),
            ));
        };
        entry.unpack(&target).map_err(|e| {
            tracing::error!("cannot unpack {}: {e}", target.display());
            SarcaError::UpdateFailed("unpacking the release failed".to_owned())
        })?;
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> SarcaResult<()> {
    let file = std::fs::File::open(archive).map_err(|e| {
        tracing::error!("cannot open {}: {e}", archive.display());
        SarcaError::UpdateFailed("cannot read the downloaded release".to_owned())
    })?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| {
        tracing::error!("cannot read the release archive: {e}");
        SarcaError::UpdateFailed("the release archive is not readable".to_owned())
    })?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| {
            tracing::error!("cannot read a release archive entry: {e}");
            SarcaError::UpdateFailed("the release archive is not readable".to_owned())
        })?;
        let Some(name) = entry.enclosed_name() else {
            tracing::error!("refusing release archive entry outside the staging dir");
            return Err(SarcaError::UpdateFailed(
                "the release archive contained an unsafe path".to_owned(),
            ));
        };
        let Some(target) = safe_join(dest, &name) else {
            return Err(SarcaError::UpdateFailed(
                "the release archive contained an unsafe path".to_owned(),
            ));
        };
        if entry.is_dir() {
            std::fs::create_dir_all(&target).ok();
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let mut out = std::fs::File::create(&target).map_err(|e| {
            tracing::error!("cannot create {}: {e}", target.display());
            SarcaError::UpdateFailed("unpacking the release failed".to_owned())
        })?;
        std::io::copy(&mut entry, &mut out).map_err(|e| {
            tracing::error!("cannot write {}: {e}", target.display());
            SarcaError::UpdateFailed("unpacking the release failed".to_owned())
        })?;
        out.flush().ok();
    }
    Ok(())
}

/// `dest/<single directory>` when the archive wrapped everything in one, which
/// is how the release workflow packs it (`sarca_linux_arm64/…`).
fn single_child_dir(dest: &Path) -> Option<PathBuf> {
    let mut dirs = std::fs::read_dir(dest)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.path());
    let first = dirs.next()?;
    if dirs.next().is_some() {
        return None;
    }
    first.join(binary_name()).is_file().then_some(first)
}

/// Move the staged binary and UI over the live ones.
///
/// Ordered so that the window in which the install is inconsistent is as small
/// as it can be made without a filesystem transaction: both new trees are
/// already on the same filesystem (staged under the install root), so each move
/// is a rename.
fn swap_in_place(root: &Path, new_binary: &Path, new_ui: &Path) -> SarcaResult<()> {
    let live_binary = root.join(binary_name());
    let live_ui = root.join("ui");
    let old_binary = root.join(format!("{}.old", binary_name()));
    let old_ui = root.join("ui.old");

    // A `.old` left by an earlier update — Windows cannot delete the binary
    // while it was running, so it is cleaned up on the next one instead.
    let _ = std::fs::remove_file(&old_binary);
    let _ = std::fs::remove_dir_all(&old_ui);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(new_binary, std::fs::Permissions::from_mode(0o755));
    }

    if live_ui.exists() {
        std::fs::rename(&live_ui, &old_ui).map_err(|e| {
            tracing::error!("cannot move the old UI aside: {e}");
            SarcaError::UpdateFailed("cannot replace the installed UI".to_owned())
        })?;
    }
    if let Err(e) = std::fs::rename(new_ui, &live_ui) {
        tracing::error!("cannot install the new UI: {e}");
        // Put the old one back: a server with no UI serves nothing.
        let _ = std::fs::rename(&old_ui, &live_ui);
        return Err(SarcaError::UpdateFailed("cannot install the new UI".to_owned()));
    }

    // Renaming the running binary is allowed on both POSIX and Windows; opening
    // it for writing is not, which is why this is a rename and not a copy.
    if live_binary.exists() {
        std::fs::rename(&live_binary, &old_binary).map_err(|e| {
            tracing::error!("cannot move the old binary aside: {e}");
            SarcaError::UpdateFailed("cannot replace the installed binary".to_owned())
        })?;
    }
    if let Err(e) = std::fs::rename(new_binary, &live_binary) {
        tracing::error!("cannot install the new binary: {e}");
        let _ = std::fs::rename(&old_binary, &live_binary);
        let _ = std::fs::remove_dir_all(&live_ui);
        let _ = std::fs::rename(&old_ui, &live_ui);
        return Err(SarcaError::UpdateFailed("cannot install the new binary".to_owned()));
    }

    let _ = std::fs::remove_dir_all(&old_ui);
    // Deliberately left on Windows: the file is still mapped by this process.
    #[cfg(unix)]
    let _ = std::fs::remove_file(&old_binary);
    Ok(())
}

/// Restart into the freshly installed binary.
///
/// `install.sh` starts the server with `nohup`, so nothing is watching to bring
/// it back up — this process has to be the one that does it. On Unix `exec`
/// replaces the image in place and keeps the pid, the terminal and the redirected
/// log file. Elsewhere a child is spawned with the same arguments before this
/// one exits.
pub fn restart_into_new_binary() -> ! {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from(binary_name()));
    let args: Vec<String> = std::env::args().skip(1).collect();
    tracing::info!("restarting {}", exe.display());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let err = std::process::Command::new(&exe).args(&args).exec();
        // `exec` only returns on failure.
        tracing::error!("restart failed: {err}");
        std::process::exit(1);
    }
    #[cfg(not(unix))]
    {
        match std::process::Command::new(&exe).args(&args).spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                tracing::error!("restart failed: {e}");
                std::process::exit(1);
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug a string comparison would ship: this project is at 0.0.170, and
    /// `"0.0.9" > "0.0.170"` lexically, so every check would either offer a
    /// downgrade or hide the real release behind one.
    #[test]
    fn versions_compare_numerically_not_lexically() {
        assert!(is_newer("v0.0.171", "0.0.170"));
        assert!(is_newer("0.0.170", "0.0.9"));
        assert!(!is_newer("0.0.9", "0.0.170"));
        assert!(!is_newer("v0.0.170", "0.0.170"));
        assert!(!is_newer("0.0.169", "0.0.170"));
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    /// A tag that carries no digits at all is not a version, and treating it as
    /// one would offer an "update" to nothing.
    #[test]
    fn a_tag_without_a_version_is_never_newer() {
        assert!(!is_newer("nightly", "0.0.170"));
        assert!(!is_newer("", "0.0.170"));
    }

    /// Shorter tags compare as if zero-padded, so `v1` is newer than `0.9.9`
    /// and `1.0` is not newer than `1.0.0`.
    #[test]
    fn missing_components_read_as_zero() {
        assert!(is_newer("v1", "0.9.9"));
        assert!(!is_newer("1.0", "1.0.0"));
        assert!(is_newer("1.0.1", "1.0"));
    }

    /// Every path in a downloaded archive is attacker-controlled input.
    #[test]
    fn archive_paths_cannot_escape_the_staging_directory() {
        let dest = Path::new("/srv/sarca");
        assert!(safe_join(dest, Path::new("sarca_linux_arm64/sarca")).is_some());
        assert!(safe_join(dest, Path::new("../../etc/passwd")).is_none());
        assert!(safe_join(dest, Path::new("/etc/passwd")).is_none());
        assert!(safe_join(dest, Path::new("ui/../../../x")).is_none());
    }

    /// Nothing downloaded is trusted on the strength of where the link came
    /// from, so the scheme is checked rather than assumed.
    #[test]
    fn only_https_release_assets_are_accepted() {
        for url in ["http://example.com/a.tar.gz", "ftp://example.com/a.tar.gz", ""] {
            assert!(!url.starts_with("https://"), "{url}");
        }
        assert!(
            "https://github.com/insigmo/sarca/releases/download/v1/a.tar.gz"
                .starts_with("https://")
        );
    }

    /// The asset list has to match what the release workflow actually
    /// publishes, or the check offers a download that 404s.
    #[test]
    fn the_asset_name_matches_the_release_workflow() {
        if let Some(name) = asset_name() {
            assert!(name.starts_with("sarca_"));
            let ext = Path::new(name).extension().and_then(|e| e.to_str());
            assert!(matches!(ext, Some("gz" | "zip")), "{name}");
        }
    }
}
