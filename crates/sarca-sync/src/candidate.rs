use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tracing::{debug, warn};
use walkdir::WalkDir;

use crate::index::mtime_ms_from_system;

#[derive(Debug, Clone)]
pub struct LocalCandidate {
    pub relative_path: String,
    pub absolute_path: PathBuf,
    pub size: i64,
    pub mtime_ms: i64,
    pub ephemeral: bool,
}

pub fn strip_dcim_prefix(relative_path: &str) -> String {
    let r = relative_path.trim().trim_matches('/');
    if let Some(rest) = r.strip_prefix("DCIM/") {
        return rest.trim_matches('/').to_owned();
    }
    if r == "DCIM" {
        return String::new();
    }
    r.to_owned()
}

/// Photo/video extensions accepted for [`BindingMode::AutoUpload`] (Camera gallery).
/// [`BindingMode::FolderUpload`] uploads every file and does not use this filter.
pub fn is_media_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "heic"
            | "heif"
            | "tif"
            | "tiff"
            | "bmp"
            | "avif"
            | "mp4"
            | "mov"
            | "m4v"
            | "mkv"
            | "webm"
            | "avi"
            | "3gp"
            | "3gpp"
    )
}

/// Walks `root` collecting candidate files. Individual entry errors (e.g. a
/// permission-denied subdirectory, or a file removed mid-walk) are logged and
/// skipped rather than aborting the whole scan — a single flaky entry should
/// not block auto-upload of every other file in the tree. Only bails when
/// walking produced zero usable files *and* at least one error occurred,
/// since that combination usually means the whole root is unreadable.
pub fn collect_fs_candidates(root: &Path, media_only: bool) -> Result<Vec<LocalCandidate>> {
    if !root.exists() {
        // Folder deleted locally (by hand or accident) — recreate it rather
        // than erroring forever, so the next push_local pass can redownload
        // its indexed content instead of leaving the binding stuck.
        std::fs::create_dir_all(root)
            .with_context(|| format!("recreate local folder: {}", root.display()))?;
    }
    let mut out = Vec::new();
    let mut walk_errors = 0usize;
    // Resolved once: every symlink target is compared against it below. `None`
    // means the root itself could not be resolved, and then no link is followed.
    let real_root = root.canonicalize().ok();
    // Do not follow directory symlinks (can explode into huge trees). Symlink
    // *files* are included only when they resolve back inside the root; see the
    // check below.
    for entry in WalkDir::new(root) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(root = %root.display(), error = %e, "walk entry error, skipping");
                walk_errors += 1;
                continue;
            }
        };
        let path = entry.path().to_path_buf();
        let ft = entry.file_type();
        let is_file = if ft.is_symlink() {
            // A link is only followed while its target stays inside the root.
            // `paths::validate_local_dir` confines a binding to the user's own
            // folders, but it checks the root alone: an escaping link would
            // carry `~/.ssh/id_rsa` to the server under an innocuous name.
            match path.canonicalize() {
                Ok(target) if real_root.as_ref().is_some_and(|r| target.starts_with(r)) => {
                    target.is_file()
                }
                Ok(target) => {
                    debug!(
                        path = %path.display(),
                        target = %target.display(),
                        "skip symlink leaving the binding root"
                    );
                    false
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "symlink metadata error, skipping");
                    walk_errors += 1;
                    false
                }
            }
        } else {
            ft.is_file()
        };
        if !is_file {
            continue;
        }
        if media_only && !is_media_file(&path) {
            continue;
        }
        let Ok(rel_os) = path.strip_prefix(root) else {
            warn!(
                path = %path.display(),
                root = %root.display(),
                "skip entry outside binding root"
            );
            continue;
        };
        let rel = rel_os.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "stat error, skipping");
                walk_errors += 1;
                continue;
            }
        };
        let mtime = meta.modified().ok().map(mtime_ms_from_system).unwrap_or(0);
        out.push(LocalCandidate {
            relative_path: rel,
            absolute_path: path,
            size: meta.len() as i64,
            mtime_ms: mtime,
            ephemeral: false,
        });
    }
    if out.is_empty() && walk_errors > 0 {
        bail!(
            "failed to walk {} ({walk_errors} error(s), 0 files found)",
            root.display()
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_dcim_prefix_strips_only_dcim_root() {
        assert_eq!(strip_dcim_prefix("DCIM/Camera/a.jpg"), "Camera/a.jpg");
        assert_eq!(strip_dcim_prefix("DCIM/a.jpg"), "a.jpg");
        assert_eq!(strip_dcim_prefix("Camera/a.jpg"), "Camera/a.jpg");
        assert_eq!(strip_dcim_prefix("dcim/x.jpg"), "dcim/x.jpg"); // case-sensitive; MediaStore uses DCIM
    }

    #[test]
    #[cfg(unix)]
    fn collect_fs_candidates_follows_symlink_to_media_file() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("a.jpg"), b"x").unwrap();
        let link = dir.path().join("link.jpg");
        std::os::unix::fs::symlink(real.join("a.jpg"), &link).unwrap();

        let got = collect_fs_candidates(dir.path(), true).unwrap();
        assert!(
            got.iter().any(|c| c.relative_path == "link.jpg"),
            "symlink-to-file must be collected as link.jpg: {got:?}"
        );
    }

    #[test]
    fn collect_fs_candidates_lists_media_and_recreates_missing_root() {
        let dir = tempfile::tempdir().unwrap();
        let pics = dir.path().join("pics");
        std::fs::create_dir_all(&pics).unwrap();
        std::fs::write(pics.join("a.jpg"), b"x").unwrap();
        std::fs::write(pics.join("note.txt"), b"y").unwrap();
        let got = collect_fs_candidates(&pics, true).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].relative_path, "a.jpg");
        assert!(!got[0].ephemeral);

        let missing = dir.path().join("nope");
        let got = collect_fs_candidates(&missing, true).unwrap();
        assert!(got.is_empty());
        assert!(missing.is_dir());
    }

    #[test]
    #[cfg(unix)]
    fn collect_fs_candidates_skips_symlinks_pointing_outside_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = tempfile::tempdir().unwrap();
        let secret = secrets.path().join("id_rsa");
        std::fs::write(&secret, b"PRIVATE KEY").unwrap();

        std::fs::write(dir.path().join("a.jpg"), b"x").unwrap();
        std::os::unix::fs::symlink(&secret, dir.path().join("b.jpg")).unwrap();
        std::os::unix::fs::symlink(secrets.path(), dir.path().join("linked")).unwrap();

        let got = collect_fs_candidates(dir.path(), true).unwrap();

        assert_eq!(
            got.len(),
            1,
            "only the in-root file may be a candidate: {got:?}"
        );
        assert_eq!(got[0].relative_path, "a.jpg");
    }

    #[test]
    #[cfg(unix)]
    fn collect_fs_candidates_tolerates_walk_errors_when_some_files_found() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"x").unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("b.jpg"), b"y").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = collect_fs_candidates(dir.path(), true);

        // Restore permissions unconditionally so tempdir cleanup can proceed.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        let got = result.expect("a single unreadable subdir must not fail the whole walk");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].relative_path, "a.jpg");
    }

    #[test]
    #[cfg(unix)]
    fn collect_fs_candidates_errors_when_every_entry_fails() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("b.jpg"), b"y").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let result = collect_fs_candidates(dir.path(), true);

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            result.is_err(),
            "zero files found plus a walk error should still hard-fail"
        );
    }
}
