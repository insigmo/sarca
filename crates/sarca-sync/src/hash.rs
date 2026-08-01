use std::{io::Read, path::Path, path::PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

/// Compute `sha256:<hex>` for a local file.
///
/// Runs on the blocking pool so large photo/video hashes do not stall the
/// Tokio runtime (and thus Tauri IPC / `list_bindings` while Camera sync runs).
pub async fn sha256_file(path: &Path) -> Result<String> {
    let path = PathBuf::from(path);
    tokio::task::spawn_blocking(move || hash_file_blocking(&path))
        .await
        .context("sha256 task join")?
}

fn hash_file_blocking(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}
