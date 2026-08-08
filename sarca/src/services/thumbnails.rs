use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use image::{GenericImageView, codecs::jpeg::JpegEncoder, imageops::FilterType};
use tokio::process::Command;

/// Longest edge of a grid tile thumbnail.
const THUMB_MAX_EDGE: u32 = 1920;

/// Longest edge of a stored/served preview.
///
/// This is what a photo open actually costs: the viewer downloads this JPEG,
/// not the original. 2048 keeps a 12MP phone photo indistinguishable on any
/// screen while cutting the bytes (and the re-encode cost) by an order of
/// magnitude relative to the original.
const PREVIEW_MAX_EDGE: u32 = 2048;

/// Quality the preview encoder uses, whether re-encoding a downscaled image
/// or converting a non-JPEG source (PNG/HEIC/etc.) to JPEG.
const PREVIEW_JPEG_QUALITY: u8 = 90;

/// Downscale kernel for thumbnails.
///
/// `Triangle` (bilinear) samples too few source pixels when the scale factor is
/// large — a 12MP phone photo reduced to the preview edge is a >4x reduction —
/// so it drops most of the detail it walks past and the result reads as soft.
/// Lanczos3 keeps the high-frequency detail that makes a preview look like the
/// photo, at a cost paid once per upload.
const DOWNSCALE_FILTER: FilterType = FilterType::Lanczos3;

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
    let img = decode_guarded(raw)?;
    let thumb = encode_jpeg(&resize_within_ref(&img, THUMB_MAX_EDGE), THUMB_JPEG_QUALITY)?;
    let preview = if include_preview {
        match passthrough_preview(raw) {
            Some(as_is) => Some(as_is),
            None => {
                Some(encode_jpeg(&resize_within_ref(&img, PREVIEW_MAX_EDGE), PREVIEW_JPEG_QUALITY)?)
            },
        }
    } else {
        None
    };
    Ok(ThumbAndPreview {
        thumb,
        preview,
    })
}

/// True for the JPEG SOI + marker prefix. The preview pipeline is JPEG end to
/// end — the cache, the stored Telegram document and the response content type
/// all assume it — so only a JPEG can be handed through unmodified.
fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.len() >= 3 && bytes[0..3] == [0xFF, 0xD8, 0xFF]
}

/// The original itself, when it is already a JPEG within the preview size.
///
/// Re-encoding a JPEG that already fits can only lose detail: it decodes
/// pixels that already went through one quantiser and puts them through a
/// second. Anything wider than [`PREVIEW_MAX_EDGE`] still needs the downscale
/// pass, so only dimensions (a header read, not a full decode) are checked
/// here.
fn passthrough_preview(raw: &[u8]) -> Option<Vec<u8>> {
    if !is_jpeg(raw) {
        return None;
    }
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(raw))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    (width.max(height) <= PREVIEW_MAX_EDGE).then(|| raw.to_vec())
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
        ThumbKind::Image => {
            match generate_image(file_path, logical_path).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!("image thumbnail skipped for {logical_path}: {e}");
                    return Ok(None);
                },
            }
        },
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

    // Images and videos both get their screen-sized preview here, off the decode
    // (or the ffmpeg keyframe) the thumbnail already paid for. PDFs have no
    // preview document, so stop at the thumbnail.
    let include_preview = matches!(kind, ThumbKind::Image | ThumbKind::Video);
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
///
/// A JPEG original already within [`PREVIEW_MAX_EDGE`] is returned untouched;
/// everything else is downscaled and encoded by [`fit_preview`].
///
/// A format the `image` crate cannot open (HEIC/AVIF off a phone) is decoded by
/// ffmpeg first and then fitted on the same path, so a photo that used to
/// have no preview at all — and therefore re-downloaded the full original on
/// every open — now caches a small JPEG like every other picture.
pub async fn generate_preview(raw: Vec<u8>) -> Result<Vec<u8>, String> {
    let raw = std::sync::Arc::new(raw);
    if let Some(as_is) = passthrough_preview(&raw) {
        return Ok(as_is);
    }
    let direct = {
        let raw = raw.clone();
        tokio::task::spawn_blocking(move || fit_preview(&raw)).await.map_err(|e| e.to_string())?
    };
    let Err(decode_error) = direct else {
        return direct;
    };

    let transcoded = transcode_image_bytes_to_jpeg(&raw)
        .await
        .map_err(|e| format!("{decode_error}; ffmpeg fallback: {e}"))?;
    tokio::task::spawn_blocking(move || fit_preview(&transcoded))
        .await
        .map_err(|e| e.to_string())?
}

/// Decode `raw` and encode it to a screen-sized JPEG preview.
fn fit_preview(raw: &[u8]) -> Result<Vec<u8>, String> {
    encode_jpeg(&resize_within(decode_guarded(raw)?, PREVIEW_MAX_EDGE), PREVIEW_JPEG_QUALITY)
}

fn detect_kind(logical_path: &str) -> Option<ThumbKind> {
    let ext = Path::new(logical_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)?;

    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => Some(ThumbKind::Image),
        // Phone-camera formats the `image` crate cannot decode. They are
        // images all the same: ffmpeg turns them into a JPEG first, and
        // everything downstream (thumb, preview, cache) is unchanged. Without
        // this a HEIC photo has no preview at all, so every open re-downloads
        // the full original.
        ext if FFMPEG_IMAGE_EXTENSIONS.contains(&ext) => Some(ThumbKind::Image),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" => Some(ThumbKind::Video),
        "pdf" => Some(ThumbKind::Pdf),
        _ => None,
    }
}

/// Image extensions the `image` crate has no decoder for, handed to ffmpeg
/// instead. Kept next to [`transcode_image_to_jpeg`], which is what makes them
/// usable.
const FFMPEG_IMAGE_EXTENSIONS: [&str; 3] = ["heic", "heif", "avif"];

fn needs_ffmpeg_decode(logical_path: &str) -> bool {
    Path::new(logical_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| FFMPEG_IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// Decode one still image with ffmpeg and hand back JPEG bytes.
///
/// Only used for formats the `image` crate cannot open; the result then goes
/// through the ordinary resize/encode path, so there is exactly one place that
/// decides preview and thumbnail sizes.
async fn transcode_image_to_jpeg(file_path: &Path) -> Result<Vec<u8>, String> {
    if which("ffmpeg").await.is_none() {
        return Err("ffmpeg not found in PATH".into());
    }

    let tmp = tempfile_dir().await?;
    let out = tmp.join("still.jpg");

    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i"])
        .arg(file_path)
        // One frame, high quality: the size-reducing pass happens afterwards.
        .args(["-frames:v", "1", "-q:v", "2"])
        .arg(&out)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("spawn ffmpeg: {e}"))?;

    if !status.success() || !out.exists() {
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        return Err("ffmpeg could not decode the image".into());
    }

    let bytes = tokio::fs::read(&out).await.map_err(|e| format!("read transcoded image: {e}"));
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    bytes
}

/// Same, from bytes already in memory (the preview slow path reassembles the
/// original from Telegram chunks and never has it on disk).
async fn transcode_image_bytes_to_jpeg(raw: &[u8]) -> Result<Vec<u8>, String> {
    let tmp = tempfile_dir().await?;
    let src = tmp.join("source.bin");
    tokio::fs::write(&src, raw).await.map_err(|e| format!("spool image: {e}"))?;
    let result = transcode_image_to_jpeg(&src).await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    result
}

async fn generate_image(file_path: &Path, logical_path: &str) -> Result<Vec<u8>, String> {
    if needs_ffmpeg_decode(logical_path) {
        return transcode_image_to_jpeg(file_path).await;
    }
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

/// Reject images whose decoded pixel buffer would be implausibly large before
/// handing them to the decoder — an attacker-controlled file can advertise
/// huge dimensions in a tiny compressed payload (decompression bomb) and blow
/// up memory during `image::load_from_memory`.
const MAX_DECODE_PIXELS: u64 = 40_000_000; // ~40MP, well above any real photo/scan

fn decode_guarded(raw: &[u8]) -> Result<image::DynamicImage, String> {
    if let Ok(reader) = image::ImageReader::new(std::io::Cursor::new(raw)).with_guessed_format() {
        if let Ok((w, h)) = reader.into_dimensions() {
            let pixels = u64::from(w) * u64::from(h);
            if pixels > MAX_DECODE_PIXELS {
                return Err(format!("image too large to decode ({w}x{h} = {pixels} px)"));
            }
        }
    }
    image::load_from_memory(raw).map_err(|e| format!("decode image: {e}"))
}

fn resize_within(img: image::DynamicImage, max_edge: u32) -> image::DynamicImage {
    let (w, h) = img.dimensions();
    if w <= max_edge && h <= max_edge {
        img
    } else {
        img.resize(max_edge, max_edge, DOWNSCALE_FILTER)
    }
}

/// Same as [`resize_within`] but keeps the source image, so one decode can feed
/// several output sizes. Only copies when the source is already small enough.
fn resize_within_ref(img: &image::DynamicImage, max_edge: u32) -> image::DynamicImage {
    let (w, h) = img.dimensions();
    if w <= max_edge && h <= max_edge {
        img.clone()
    } else {
        img.resize(max_edge, max_edge, DOWNSCALE_FILTER)
    }
}

fn encode_jpeg(img: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    let encoder = JpegEncoder::new_with_quality(&mut cursor, quality);
    img.write_with_encoder(encoder).map_err(|e| format!("encode jpeg: {e}"))?;
    Ok(out)
}

/// One-shot decode → downscale → encode. Kept for tests that assert a single
/// rung; production code uses [`resize_within`]/[`resize_within_ref`] directly.
#[cfg(test)]
fn resize_to_jpeg(raw: &[u8], max_edge: u32, quality: u8) -> Result<Vec<u8>, String> {
    encode_jpeg(&resize_within(decode_guarded(raw)?, max_edge), quality)
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

    /// A genuinely smooth gradient. `sample_png` wraps every 256px, and those
    /// repeating hard edges cost a JPEG far more than a real photograph does.
    fn smooth_png(w: u32, h: u32) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
            let r = (x * 255 / w.max(1)) as u8;
            let g = (y * 255 / h.max(1)) as u8;
            Rgb([r, g, 128])
        });
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png).unwrap();
        buf
    }

    /// Worst case for the encoder: per-pixel noise has no redundancy to remove,
    /// so it is what proves the budget is a cap and not a hope.
    fn noisy_png(w: u32, h: u32) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(w, h, |x, y| {
            // Cheap deterministic hash; no rand dependency in this crate.
            let mut v = u64::from(x).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ u64::from(y).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
            v ^= v >> 31;
            v = v.wrapping_mul(0xD6E8_FEB8_6659_FD93);
            Rgb([(v >> 8) as u8, (v >> 24) as u8, (v >> 40) as u8])
        });
        let mut buf = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut buf), ImageFormat::Png).unwrap();
        buf
    }

    #[test]
    fn preview_shrinks_large_image_and_outputs_jpeg() {
        let raw = sample_png(3000, 2000);
        let jpeg = fit_preview(&raw).unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert!(decoded.width() <= PREVIEW_MAX_EDGE);
        assert!(decoded.height() <= PREVIEW_MAX_EDGE);
    }

    #[test]
    fn preview_of_a_dense_photo_stays_within_the_edge() {
        let jpeg = fit_preview(&noisy_png(3000, 2250)).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert!(decoded.width() <= PREVIEW_MAX_EDGE);
        assert!(decoded.height() <= PREVIEW_MAX_EDGE);
    }

    #[test]
    fn preview_of_a_plain_photo_is_downscaled_to_the_preview_edge() {
        let jpeg = fit_preview(&smooth_png(4000, 3000)).unwrap();
        let decoded = image::load_from_memory(&jpeg).expect("preview decodes");
        assert_eq!(decoded.width(), PREVIEW_MAX_EDGE, "a photo that overflows must be capped");
    }

    #[test]
    fn a_small_jpeg_original_is_served_untouched() {
        let jpeg = encode_jpeg(&image::load_from_memory(&sample_png(1200, 900)).unwrap(), 80)
            .expect("sample encodes");

        assert_eq!(
            passthrough_preview(&jpeg).as_deref(),
            Some(jpeg.as_slice()),
            "a JPEG original already within the preview edge must not be re-encoded"
        );
    }

    #[test]
    fn passthrough_refuses_oversized_jpeg_and_non_jpeg_formats() {
        let big_jpeg = encode_jpeg(&image::load_from_memory(&sample_png(3000, 2000)).unwrap(), 80)
            .expect("sample encodes");
        assert!(passthrough_preview(&big_jpeg).is_none(), "an oversized JPEG must be downscaled");

        // PNG: the pipeline serves `image/jpeg`, so it cannot be passed
        // through even though it decodes fine.
        assert!(passthrough_preview(&sample_png(64, 64)).is_none());
    }

    #[test]
    fn thumb_stays_small() {
        let raw = sample_png(800, 600);
        let jpeg = resize_to_jpeg(&raw, THUMB_MAX_EDGE, THUMB_JPEG_QUALITY).unwrap();
        let decoded = image::load_from_memory(&jpeg).unwrap();
        assert!(decoded.width() <= THUMB_MAX_EDGE);
        assert!(decoded.height() <= THUMB_MAX_EDGE);
    }

    #[tokio::test]
    async fn generate_returns_preview_for_images_without_a_second_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.png");
        std::fs::write(&path, sample_png(3000, 2000)).unwrap();

        let result = generate(&path, "photo.png", u64::MAX).await.unwrap().unwrap();

        let preview = image::load_from_memory(&result.preview.unwrap()).unwrap();
        assert!(preview.width() <= PREVIEW_MAX_EDGE);
        assert!(preview.height() <= PREVIEW_MAX_EDGE);
        assert!(preview.width() > THUMB_MAX_EDGE);
    }

    #[test]
    fn is_preview_image_matches_extensions() {
        assert!(is_preview_image("a.JPG"));
        assert!(is_preview_image("dir/x.webp"));
        // Decoded by ffmpeg, but an image as far as previews are concerned.
        assert!(is_preview_image("IMG_0001.HEIC"));
        assert!(is_preview_image("shot.avif"));
        assert!(!is_preview_image("clip.mp4"));
        assert!(!is_preview_image("doc.pdf"));
    }

    #[test]
    fn only_the_undecodable_formats_take_the_ffmpeg_path() {
        assert!(needs_ffmpeg_decode("IMG_0001.HEIC"));
        assert!(needs_ffmpeg_decode("x.heif"));
        assert!(needs_ffmpeg_decode("dir/y.avif"));
        assert!(!needs_ffmpeg_decode("a.jpg"));
        assert!(!needs_ffmpeg_decode("a.png"));
        assert!(!needs_ffmpeg_decode("clip.mp4"));
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
    #fn build_thumb_and_preview_with_preview() {
    #    let raw = sample_png(3000, 2000);
    #    let result = build_thumb_and_preview(&raw, true).unwrap();
    #
    #    let thumb = image::load_from_memory(&result.thumb).unwrap();
    #    assert!(thumb.width() <= THUMB_MAX_EDGE && thumb.height() <= THUMB_MAX_EDGE);
    #
    #    let preview = image::load_from_memory(result.preview.as_ref().unwrap()).unwrap();
    #    assert_eq!(preview.width(), 3000, "preview must keep full resolution");
    #    assert_eq!(preview.height(), 2000, "preview must keep full resolution");
    #}
}
