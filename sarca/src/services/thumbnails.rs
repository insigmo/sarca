use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use image::{GenericImageView, codecs::jpeg::JpegEncoder, imageops::FilterType};
use tokio::process::Command;

const THUMB_MAX_EDGE: u32 = 128;
pub const PREVIEW_MAX_EDGE: u32 = 1920;
const PREVIEW_JPEG_QUALITY: u8 = 80;

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
        ThumbKind::Video => {
            match generate_video(file_path).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!("video thumbnail skipped: {e}");
                    return Ok(None);
                },
            }
        },
        ThumbKind::Pdf => {
            match generate_pdf(file_path).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!("pdf thumbnail skipped: {e}");
                    return Ok(None);
                },
            }
        },
    };

    let jpeg = tokio::task::spawn_blocking(move || {
        resize_to_jpeg(&raw, THUMB_MAX_EDGE, THUMB_JPEG_QUALITY)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(Some(jpeg))
}

const THUMB_JPEG_QUALITY: u8 = 75;

/// Whether the logical path is a raster image we can preview-encode.
pub fn is_preview_image(logical_path: &str) -> bool {
    matches!(detect_kind(logical_path), Some(ThumbKind::Image))
}

/// Encode raw image bytes to a screen-sized JPEG preview.
pub async fn generate_preview(raw: Vec<u8>) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || {
        resize_to_jpeg(&raw, PREVIEW_MAX_EDGE, PREVIEW_JPEG_QUALITY)
    })
    .await
    .map_err(|e| e.to_string())?
}

fn detect_kind(logical_path: &str) -> Option<ThumbKind> {
    let ext = Path::new(logical_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)?;

    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => Some(ThumbKind::Image),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" => Some(ThumbKind::Video),
        "pdf" => Some(ThumbKind::Pdf),
        _ => None,
    }
}

async fn generate_image(file_path: &Path) -> Result<Vec<u8>, String> {
    tokio::fs::read(file_path).await.map_err(|e| format!("read image: {e}"))
}

async fn generate_video(file_path: &Path) -> Result<Vec<u8>, String> {
    if which("ffmpeg").await.is_none() {
        return Err("ffmpeg not found in PATH".into());
    }

    let tmp = tempfile_dir().await?;
    let pattern = tmp.join("kf_%02d.jpg");

    let status = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(file_path)
        .args(["-vf", "select=eq(pict_type\\,I)", "-vsync", "vfr", "-frames:v", "3"])
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

        let bytes =
            tokio::fs::read(&fallback).await.map_err(|e| format!("read fallback frame: {e}"))?;
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
    let bytes = tokio::fs::read(&frame).await.map_err(|e| format!("read keyframe: {e}"))?;
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

    let bytes = tokio::fs::read(&page).await.map_err(|e| format!("read pdf page: {e}"))?;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    Ok(bytes)
}

fn resize_to_jpeg(raw: &[u8], max_edge: u32, quality: u8) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(raw).map_err(|e| format!("decode image: {e}"))?;
    let (w, h) = img.dimensions();
    let resized = if w <= max_edge && h <= max_edge {
        img
    } else {
        img.resize(max_edge, max_edge, FilterType::Triangle)
    };
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    let encoder = JpegEncoder::new_with_quality(&mut cursor, quality);
    resized.write_with_encoder(encoder).map_err(|e| format!("encode jpeg: {e}"))?;
    Ok(out)
}

async fn tempfile_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("sarca-thumb-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&dir).await.map_err(|e| format!("create temp dir: {e}"))?;
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
    if path.is_empty() { None } else { Some(PathBuf::from(path)) }
}

#[cfg(test)]
mod tests {
    use image::{ImageBuffer, ImageFormat, Rgb};

    use super::*;

    fn sample_png(w: u32, h: u32) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(w, h, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn preview_shrinks_large_image_and_outputs_jpeg() {
        let raw = sample_png(3000, 2000);
        let jpeg = resize_to_jpeg(&raw, PREVIEW_MAX_EDGE, PREVIEW_JPEG_QUALITY).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert!(decoded.width() <= PREVIEW_MAX_EDGE);
        assert!(decoded.height() <= PREVIEW_MAX_EDGE);
    }

    #[test]
    fn thumb_stays_small() {
        let raw = sample_png(800, 600);
        let jpeg = resize_to_jpeg(&raw, THUMB_MAX_EDGE, THUMB_JPEG_QUALITY).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert!(decoded.width() <= THUMB_MAX_EDGE);
        assert!(decoded.height() <= THUMB_MAX_EDGE);
    }

    #[test]
    fn is_preview_image_matches_extensions() {
        assert!(is_preview_image("a.JPG"));
        assert!(is_preview_image("dir/x.webp"));
        assert!(!is_preview_image("clip.mp4"));
        assert!(!is_preview_image("doc.pdf"));
    }
}
