//! Validation for filesystem paths that cross the IPC boundary.
//!
//! `add_binding` / `update_binding_local_path` take a directory chosen in the
//! WebView. Without a check, anything that reaches the native bridge could bind
//! `/` or `~/.ssh` and let the sync engine upload it to the attacker's storage.
//! Every path is therefore canonicalized (which resolves `..` and symlinks)
//! and then confined to the user's own data roots.

use std::path::{Path, PathBuf};

/// Longest path we accept. Stops a hostile caller from making us canonicalize a
/// multi-megabyte string.
const MAX_PATH_LEN: usize = 4096;

/// Strip the Windows extended-length prefix that `canonicalize` adds, so the
/// value we persist and show is the path the user recognises.
fn strip_verbatim(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path.to_path_buf(),
    }
}

/// True when any component of `path` is a dot-directory.
///
/// On Unix that is where credentials live (`.ssh`, `.gnupg`, `.aws`,
/// `.config`, `.local/share/keyrings`, browser profiles), and a denylist of
/// specific names would always be one entry short.
fn has_hidden_component(path: &Path) -> Option<String> {
    path.components().find_map(|c| {
        let name = c.as_os_str().to_string_lossy();
        // "." and ".." are gone after canonicalization; a bare "." would not be
        // a hidden directory anyway.
        if name.len() > 1 && name.starts_with('.') {
            Some(name.into_owned())
        } else {
            None
        }
    })
}

/// Platform directories that hold application state rather than user files.
fn is_reserved_component(path: &Path) -> Option<String> {
    const RESERVED: &[&str] = &["AppData", "Library", "Application Data", "node_modules"];
    path.components().find_map(|c| {
        let name = c.as_os_str().to_string_lossy();
        RESERVED
            .iter()
            .find(|r| r.eq_ignore_ascii_case(name.as_ref()))
            .map(|r| (*r).to_owned())
    })
}

fn is_under(path: &Path, root: &Path) -> bool {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    path == root || path.starts_with(&root)
}

/// Canonicalize `raw` and confirm it is a directory the user may sync.
///
/// * `allowed_roots` — the user's own file roots (home, Android shared storage).
///   An empty list means "no root could be resolved", which is refused rather
///   than treated as "allow everything".
/// * `denied_roots` — application-owned directories (the client's own data /
///   config / cache dirs). Binding one of those would upload the session tokens
///   and the local sync database.
pub fn validate_local_dir(
    raw: &str,
    allowed_roots: &[PathBuf],
    denied_roots: &[PathBuf],
) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Local folder is required".into());
    }
    if trimmed.len() > MAX_PATH_LEN {
        return Err("Local folder path is too long".into());
    }
    if trimmed.contains('\0') {
        return Err("Local folder path contains a NUL byte".into());
    }

    let requested = PathBuf::from(trimmed);
    if !requested.is_absolute() {
        return Err(format!("Local folder must be an absolute path: {trimmed}"));
    }

    // Resolve `..` and symlinks before comparing. A textual prefix check is
    // escapable both by `~/Pictures/../../etc` and by a symlink planted inside
    // an allowed root.
    let real = requested
        .canonicalize()
        .map_err(|_| format!("Local folder does not exist: {trimmed}"))?;

    if !real.is_dir() {
        return Err(format!("Local path is not a folder: {trimmed}"));
    }

    if allowed_roots.is_empty() {
        return Err("Could not resolve a user folder to sync from".into());
    }

    for denied in denied_roots {
        if is_under(&real, denied) {
            return Err("That folder belongs to Sarca itself and cannot be synced".into());
        }
    }

    if !allowed_roots.iter().any(|root| is_under(&real, root)) {
        return Err(format!(
            "Local folder must be inside your own user folder: {trimmed}"
        ));
    }

    // Only check the part below the root: an allowed root may itself sit under
    // a hidden directory (Flatpak installs home under `~/.var/app/...`).
    let root = allowed_roots
        .iter()
        .find(|root| is_under(&real, root))
        .map(|root| root.canonicalize().unwrap_or_else(|_| root.clone()))
        .unwrap_or_default();
    let relative = real.strip_prefix(&root).unwrap_or(&real);

    if let Some(name) = has_hidden_component(relative) {
        return Err(format!(
            "Hidden folders can hold credentials and cannot be synced: {name}"
        ));
    }
    if let Some(name) = is_reserved_component(relative) {
        return Err(format!("System folders cannot be synced: {name}"));
    }

    Ok(strip_verbatim(&real).to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sarca-paths-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn accepts_a_plain_folder_under_the_root() {
        let root = tmp_root("ok");
        let pictures = root.join("Pictures");
        std::fs::create_dir_all(&pictures).unwrap();

        let got = validate_local_dir(pictures.to_str().unwrap(), &[root.clone()], &[]).unwrap();
        assert_eq!(PathBuf::from(got), pictures.canonicalize().unwrap());
    }

    #[test]
    fn rejects_traversal_out_of_the_root() {
        let root = tmp_root("traversal");
        let escaped = format!("{}/Pictures/../../..", root.display());
        std::fs::create_dir_all(root.join("Pictures")).unwrap();

        let err = validate_local_dir(&escaped, &[root], &[]).unwrap_err();
        assert!(err.contains("inside your own user folder"), "{err}");
    }

    #[test]
    fn rejects_hidden_directories() {
        let root = tmp_root("hidden");
        let ssh = root.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();

        let err = validate_local_dir(ssh.to_str().unwrap(), &[root], &[]).unwrap_err();
        assert!(err.contains("Hidden folders"), "{err}");
    }

    #[test]
    fn rejects_the_apps_own_data_dir() {
        let root = tmp_root("appdata");
        let data = root.join("sarca-data");
        std::fs::create_dir_all(&data).unwrap();

        let err = validate_local_dir(data.to_str().unwrap(), &[root], &[data.clone()]).unwrap_err();
        assert!(err.contains("belongs to Sarca"), "{err}");
    }

    #[test]
    fn rejects_relative_empty_and_missing_paths() {
        let root = tmp_root("bad");
        for raw in ["", "   ", "Pictures", "../etc"] {
            assert!(
                validate_local_dir(raw, &[root.clone()], &[]).is_err(),
                "{raw}"
            );
        }
        let missing = root.join("nope");
        assert!(validate_local_dir(missing.to_str().unwrap(), &[root], &[]).is_err());
    }

    #[test]
    fn rejects_when_no_root_is_known() {
        let root = tmp_root("noroot");
        let err = validate_local_dir(root.to_str().unwrap(), &[], &[]).unwrap_err();
        assert!(err.contains("Could not resolve"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlink_that_escapes_the_root() {
        let root = tmp_root("symlink");
        let outside = tmp_root("symlink-outside");
        let link = root.join("escape");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let err = validate_local_dir(link.to_str().unwrap(), &[root], &[]).unwrap_err();
        assert!(err.contains("inside your own user folder"), "{err}");
    }
}
