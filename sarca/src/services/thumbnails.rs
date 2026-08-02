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

/// Result of generating a file's thumbnail (and, for video, a
/// screen-sized preview from the same extracted keyframe — one ffmpeg
/// invocation instead of two).
pub struct ThumbAndPreview {
    pub thumb: Vec<u8>,
    pub preview: Option<Vec<u8>>,
}

fn build_thumb_and_preview(raw: &[u8], include_preview: bool) -> Result<ThumbAndPreview, String> {
    let thumb = resize_to_jpeg(raw, THUMB_MAX_EDGE, THUMB_JPEG_QUALITY)?;
    let preview = if include_preview {
        Some(resize_to_jpeg(raw, PREVIEW_MAX_EDGE, PREVIEW_JPEG_QUALITY)?)
    } else {
        None
    };
    Ok(ThumbAndPreview {
        thumb,
        preview,
    })
}

/// Try to build a JPEG thumbnail for the given file.
/// Returns `Ok(None)` when the type is unsupported or helpers are missing.
pub async fn generate(
    file_path: &Path,
    logical_path: &str,
    chunk_size_bytes: u64,
) -> Result<Option<ThumbAndPreview>, String> {
    let Some(kind) = detect_kind(logical_path) else {
        return Ok(None);
    };

    let raw = match kind {
        ThumbKind::Image => generate_image(file_path).await?,
        ThumbKind::Video => {
            match generate_video(file_path, chunk_size_bytes).await {
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

    let include_preview = kind == ThumbKind::Video;
    let result =
        tokio::task::spawn_blocking(move || build_thumb_and_preview(&raw, include_preview))
            .await
            .map_err(|e| e.to_string())??;

    Ok(Some(result))
}

const THUMB_JPEG_QUALITY: u8 = 75;

const KEYFRAME_TARGET: u32 = 10;

/// Candidate keyframe filenames in preference order: highest-numbered
/// (latest-extracted) first, matching the existing `kf_%02d.jpg` ffmpeg
/// output pattern. Picking the highest-numbered file that actually exists
/// is how we select "the 10th keyframe if present, else the last one
/// ffmpeg managed to extract."
fn keyframe_candidate_names(count: u32) -> Vec<String> {
    (1..=count).rev().map(|n| format!("kf_{n:02}.jpg")).collect()
}

fn pick_existing_keyframe(dir: &Path, names: &[String]) -> Option<PathBuf> {
    names.iter().map(|name| dir.join(name)).find(|p| p.exists())
}

/// Copy at most `max_bytes` from the start of `src` into `dst`. Used to
/// run ffmpeg against just the first upload chunk of a video instead of
/// the full file, so keyframe extraction cost is bounded independent of
/// video length.
async fn truncate_prefix(src: &Path, dst: &Path, max_bytes: u64) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut input = tokio::fs::File::open(src).await.map_err(|e| format!("open source: {e}"))?;
    let mut output =
        tokio::fs::File::create(dst).await.map_err(|e| format!("create prefix file: {e}"))?;

    let mut remaining = max_bytes;
    let mut buf = vec![0u8; 64 * 1024].into_boxed_slice();
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        let n = input.read(&mut buf[..want]).await.map_err(|e| format!("read source: {e}"))?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n]).await.map_err(|e| format!("write prefix: {e}"))?;
        remaining -= n as u64;
    }
    output.flush().await.map_err(|e| format!("flush prefix: {e}"))?;
    Ok(())
}

/// Whether the logical path is a raster image we can preview-encode.
pub fn is_preview_image(logical_path: &str) -> bool {
    matches!(detect_kind(logical_path), Some(ThumbKind::Image))
}

/// Read an image file and encode it to a screen-sized JPEG preview.
pub async fn generate_preview_from_path(file_path: &Path) -> Result<Vec<u8>, String> {
    let raw = tokio::fs::read(file_path).await.map_err(|e| format!("read image: {e}"))?;
    generate_preview(raw).await
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

async fn generate_video(file_path: &Path, chunk_size_bytes: u64) -> Result<Vec<u8>, String> {
    if which("ffmpeg").await.is_none() {
        return Err("ffmpeg not found in PATH".into());
    }

    let tmp = tempfile_dir().await?;
    let names = keyframe_candidate_names(KEYFRAME_TARGET);

    let prefix_path = tmp.join("prefix.bin");
    if truncate_prefix(file_path, &prefix_path, chunk_size_bytes).await.is_ok()
        && extract_keyframes(&prefix_path, &tmp, KEYFRAME_TARGET).await.is_ok()
    {
        if let Some(frame) = pick_existing_keyframe(&tmp, &names) {
            let bytes = tokio::fs::read(&frame).await.map_err(|e| format!("read keyframe: {e}"));
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            return bytes;
        }
    }

    // The truncated prefix didn't decode (e.g. an mp4 without `+faststart` has
    // its `moov` index at the end of the file) or yielded no keyframes — retry
    // against the full original before falling back to a single near-start frame.
    if extract_keyframes(file_path, &tmp, KEYFRAME_TARGET).await.is_ok() {
        if let Some(frame) = pick_existing_keyframe(&tmp, &names) {
            let bytes = tokio::fs::read(&frame).await.map_err(|e| format!("read keyframe: {e}"));
            let _ = tokio::fs::remove_dir_all(&tmp).await;
            return bytes;
        }
    }

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

    let bytes = tokio::fs::read(&fallback).await.map_err(|e| format!("read fallback frame: {e}"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    bytes
}

/// Extract up to `count` I-frames from `input` into `out_dir/kf_%02d.jpg`.
/// `Err` means ffmpeg exited non-zero (e.g. no usable index in `input`).
async fn extract_keyframes(input: &Path, out_dir: &Path, count: u32) -> Result<(), String> {
    let pattern = out_dir.join("kf_%02d.jpg");
    let status = Command::new("ffmpeg")
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input)
        .args(["-vf", "select=eq(pict_type\\,I)", "-vsync", "vfr", "-frames:v"])
        .arg(count.to_string())
        .arg(&pattern)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("spawn ffmpeg: {e}"))?;

    if status.success() { Ok(()) } else { Err("ffmpeg keyframe extraction failed".into()) }
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

    #[test]
    fn keyframe_candidate_names_orders_high_to_low() {
        let names = keyframe_candidate_names(10);
        assert_eq!(names.len(), 10);
        assert_eq!(names.first().unwrap(), "kf_10.jpg");
        assert_eq!(names.last().unwrap(), "kf_01.jpg");
    }

    #[test]
    fn pick_existing_keyframe_prefers_highest_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("kf_05.jpg"), b"x").unwrap();
        std::fs::write(dir.path().join("kf_03.jpg"), b"x").unwrap();
        let names = keyframe_candidate_names(10);
        let picked = pick_existing_keyframe(dir.path(), &names).unwrap();
        assert_eq!(picked.file_name().unwrap(), "kf_05.jpg");
    }

    #[test]
    fn pick_existing_keyframe_none_when_nothing_matches() {
        let dir = tempfile::tempdir().unwrap();
        let names = keyframe_candidate_names(10);
        assert!(pick_existing_keyframe(dir.path(), &names).is_none());
    }

    #[tokio::test]
    async fn truncate_prefix_copies_only_requested_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bin");
        let dst = dir.path().join("dst.bin");
        let data: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        std::fs::write(&src, &data).unwrap();

        truncate_prefix(&src, &dst, 100).await.unwrap();

        let out = std::fs::read(&dst).unwrap();
        assert_eq!(out.len(), 100);
        assert_eq!(out, data[..100]);
    }

    #[tokio::test]
    async fn truncate_prefix_copies_whole_file_when_smaller_than_limit() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.bin");
        let dst = dir.path().join("dst.bin");
        std::fs::write(&src, b"short").unwrap();

        truncate_prefix(&src, &dst, 1_000_000).await.unwrap();

        assert_eq!(std::fs::read(&dst).unwrap(), b"short");
    }

    async fn make_test_video(
        dir: &std::path::Path,
        name: &str,
        extra_args: &[&str],
    ) -> std::path::PathBuf {
        let out = dir.join(name);
        let status = tokio::process::Command::new("ffmpeg")
            .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=size=320x240:rate=10:duration=2")
            .args(["-pix_fmt", "yuv420p", "-c:v", "libx264"])
            .args(extra_args)
            .arg(&out)
            .status()
            .await
            .expect("spawn ffmpeg");
        assert!(status.success(), "test fixture encode failed");
        out
    }

    #[tokio::test]
    async fn generate_video_full_file_still_works() {
        if which("ffmpeg").await.is_none() {
            eprintln!("skip: ffmpeg not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let video = make_test_video(dir.path(), "full.mp4", &["-movflags", "+faststart"]).await;
        let size = std::fs::metadata(&video).unwrap().len();

        let jpeg = generate_video(&video, size).await.unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[tokio::test]
    async fn generate_video_falls_back_to_full_file_when_prefix_lacks_moov() {
        if which("ffmpeg").await.is_none() {
            eprintln!("skip: ffmpeg not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // No +faststart: the mp4 muxer puts `moov` at the end of the file, so a
        // small byte prefix has no index and ffmpeg cannot decode it directly.
        let video = make_test_video(dir.path(), "no_faststart.mp4", &[]).await;

        let jpeg = generate_video(&video, 4096).await.unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[test]
    fn build_thumb_and_preview_without_preview() {
        let raw = sample_png(800, 600);
        let result = build_thumb_and_preview(&raw, false).unwrap();

        let thumb = image::load_from_memory(&result.thumb).unwrap();
        assert!(thumb.width() <= THUMB_MAX_EDGE && thumb.height() <= THUMB_MAX_EDGE);
        assert!(result.preview.is_none());
    }

    #[test]
    fn build_thumb_and_preview_with_preview() {
        let raw = sample_png(3000, 2000);
        let result = build_thumb_and_preview(&raw, true).unwrap();

        let thumb = image::load_from_memory(&result.thumb).unwrap();
        assert!(thumb.width() <= THUMB_MAX_EDGE && thumb.height() <= THUMB_MAX_EDGE);

        let preview = image::load_from_memory(result.preview.as_ref().unwrap()).unwrap();
        assert!(preview.width() <= PREVIEW_MAX_EDGE && preview.height() <= PREVIEW_MAX_EDGE);
    }
}
