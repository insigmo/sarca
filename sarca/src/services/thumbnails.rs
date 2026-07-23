use std::path::{Path, PathBuf};
use std::process::Stdio;

use image::imageops::FilterType;
use image::ImageFormat;
use tokio::process::Command;

const THUMB_MAX_EDGE: u32 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThumbKind {
    Image,
    Video,
    Pdf,
}

/// Try to build a JPEG thumbnail for the given file.
/// Returns `Ok(None)` when the type is unsupported or helpers are missing.
pub async fn generate(file_path: &Path, logical_path: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(kind) = detect_kind(logical_path) else {
        return Ok(None);
    };

    let raw = match kind {
        ThumbKind::Image => generate_image(file_path).await?,
        ThumbKind::Video => match generate_video(file_path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("video thumbnail skipped: {e}");
                return Ok(None);
            }
        },
        ThumbKind::Pdf => match generate_pdf(file_path).await {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("pdf thumbnail skipped: {e}");
                return Ok(None);
            }
        },
    };

    let jpeg = tokio::task::spawn_blocking(move || resize_to_jpeg(&raw))
        .await
        .map_err(|e| e.to_string())??;

    Ok(Some(jpeg))
}

fn detect_kind(logical_path: &str) -> Option<ThumbKind> {
    let ext = Path::new(logical_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())?;

    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => Some(ThumbKind::Image),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" => Some(ThumbKind::Video),
        "pdf" => Some(ThumbKind::Pdf),
        _ => None,
    }
}

async fn generate_image(file_path: &Path) -> Result<Vec<u8>, String> {
    tokio::fs::read(file_path)
        .await
        .map_err(|e| format!("read image: {e}"))
}

async fn generate_video(file_path: &Path) -> Result<Vec<u8>, String> {
    if which("ffmpeg").await.is_none() {
        return Err("ffmpeg not found in PATH".into());
    }

    let tmp = tempfile_dir().await?;
    let pattern = tmp.join("kf_%02d.jpg");

    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
        ])
        .arg(file_path)
        .args([
            "-vf",
            "select=eq(pict_type\\,I)",
            "-vsync",
            "vfr",
            "-frames:v",
            "3",
        ])
        .arg(&pattern)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("spawn ffmpeg: {e}"))?;

    if !status.success() {
        // Fallback: grab a single frame near 10% of duration / 1s.
        let fallback = tmp.join("fallback.jpg");
        let status = Command::new("ffmpeg")
            .args(["-y", "-hide_banner", "-loglevel", "error", "-ss", "1", "-i"])
            .arg(file_path)
            .args(["-frames:v", "1", "-q:v", "3"])
            .arg(&fallback)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|e| format!("spawn ffmpeg fallback: {e}"))?;

        if !status.success() || !fallback.exists() {
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            return Err("ffmpeg could not extract a frame".into());
        }

        let bytes = tokio::fs::read(&fallback)
            .await
            .map_err(|e| format!("read fallback frame: {e}"))?;
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Ok(bytes);
    }

    let candidates = ["kf_03.jpg", "kf_02.jpg", "kf_01.jpg"];
    let mut chosen: Option<PathBuf> = None;
    for name in candidates {
        let p = tmp.join(name);
        if p.exists() {
            chosen = Some(p);
            break;
        }
    }

    let Some(frame) = chosen else {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err("no keyframes extracted".into());
    };

    // Prefer the 3rd keyframe when present (kf_03); otherwise last available.
    let bytes = tokio::fs::read(&frame)
        .await
        .map_err(|e| format!("read keyframe: {e}"))?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(bytes)
}

async fn generate_pdf(file_path: &Path) -> Result<Vec<u8>, String> {
    if which("pdftoppm").await.is_none() {
        return Err("pdftoppm not found in PATH".into());
    }

    let tmp = tempfile_dir().await?;
    let out_prefix = tmp.join("page");

    let status = Command::new("pdftoppm")
        .args(["-f", "1", "-l", "1", "-jpeg", "-singlefile", "-scale-to", "256"])
        .arg(file_path)
        .arg(&out_prefix)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("spawn pdftoppm: {e}"))?;

    let page = tmp.join("page.jpg");
    if !status.success() || !page.exists() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err("pdftoppm failed to render first page".into());
    }

    let bytes = tokio::fs::read(&page)
        .await
        .map_err(|e| format!("read pdf page: {e}"))?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(bytes)
}

fn resize_to_jpeg(raw: &[u8]) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(raw).map_err(|e| format!("decode image: {e}"))?;
    let resized = img.resize(THUMB_MAX_EDGE, THUMB_MAX_EDGE, FilterType::Triangle);
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    resized
        .write_to(&mut cursor, ImageFormat::Jpeg)
        .map_err(|e| format!("encode jpeg: {e}"))?;
    Ok(out)
}

async fn tempfile_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("sarca-thumb-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("create temp dir: {e}"))?;
    Ok(dir)
}

async fn which(bin: &str) -> Option<PathBuf> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}
