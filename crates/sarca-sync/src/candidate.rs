use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
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

pub fn collect_fs_candidates(root: &Path, media_only: bool) -> Result<Vec<LocalCandidate>> {
    if !root.exists() {
        bail!("local folder missing or unreadable: {}", root.display());
    }
    let mut out = Vec::new();
    for entry in WalkDir::new(root) {
        let entry = entry.with_context(|| format!("walk {}", root.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path().to_path_buf();
        if media_only && !is_media_file(&path) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.is_empty() {
            continue;
        }
        let meta = entry
            .metadata()
            .with_context(|| format!("meta {}", path.display()))?;
        let mtime = meta.modified().ok().map(mtime_ms_from_system).unwrap_or(0);
        out.push(LocalCandidate {
            relative_path: rel,
            absolute_path: path,
            size: meta.len() as i64,
            mtime_ms: mtime,
            ephemeral: false,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn strip_dcim_prefix_strips_only_dcim_root() {
        assert_eq!(strip_dcim_prefix("DCIM/Camera/a.jpg"), "Camera/a.jpg");
        assert_eq!(strip_dcim_prefix("DCIM/a.jpg"), "a.jpg");
        assert_eq!(strip_dcim_prefix("Camera/a.jpg"), "Camera/a.jpg");
        assert_eq!(strip_dcim_prefix("dcim/x.jpg"), "dcim/x.jpg"); // case-sensitive; MediaStore uses DCIM
    }

    #[test]
    fn collect_fs_candidates_lists_media_and_fails_on_missing_root() {
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
        assert!(collect_fs_candidates(&missing, true).is_err());
    }
}
