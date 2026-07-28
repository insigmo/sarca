//! Optional append-only client log file for debugging auto-upload on device.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

static ENABLED: AtomicBool = AtomicBool::new(false);

pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("sarca-client.log")
}

pub fn set_enabled(enabled: bool, data_dir: &Path) {
    ENABLED.store(enabled, Ordering::Relaxed);
    if enabled {
        let _ = fs::create_dir_all(data_dir.join("logs"));
        write_line(data_dir, "logging enabled");
    }
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn write_line(data_dir: &Path, msg: &str) {
    if !is_enabled() {
        return;
    }
    let path = log_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(file, "{ts} {msg}");
}

/// Read log file contents (capped) for export / share.
pub fn read_export(data_dir: &Path, max_bytes: usize) -> Result<String, String> {
    let path = log_path(data_dir);
    if !path.is_file() {
        return Ok(String::from(
            "(no log file yet — enable logging and reproduce the issue)\n",
        ));
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    if bytes.len() <= max_bytes {
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    let start = bytes.len() - max_bytes;
    Ok(format!(
        "(truncated, showing last {max_bytes} bytes)\n{}",
        String::from_utf8_lossy(&bytes[start..])
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn write_and_read_when_enabled() {
        let dir = env::temp_dir().join(format!("sarca-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        set_enabled(true, &dir);
        write_line(&dir, "hello-auto-upload");
        let text = read_export(&dir, 64 * 1024).unwrap();
        assert!(text.contains("hello-auto-upload"));
        set_enabled(false, &dir);
        let _ = fs::remove_dir_all(&dir);
    }
}
