//! Media prefetch daemon.
//!
//! Walks each storage's tree a few levels deep and warms the thumb/preview
//! caches ahead of time, so the first grid render and the first viewer open
//! are served from disk instead of paying a Telegram round trip. For videos it
//! also pulls the first chunk into the chunk cache, so playback starts on the
//! opening Range request instead of waiting out a download.
//!
//! It sweeps repeatedly (every `PREFETCH_INTERVAL_SECS`, and immediately when
//! [`request_sweep`] fires — a storage was just added, say), because storages
//! and files appear while the process runs and a one-shot startup pass leaves
//! everything after it cold. Never blocks the boot path (spawned, not awaited)
//! and never fails startup — every error here is logged and swallowed.

use std::{
    collections::VecDeque,
    future::Future,
    path::Path,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use axum::http::StatusCode;
use futures::{StreamExt, stream};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::{
    common::{media_cache::MediaCache, routing::app_state::AppState},
    config::Config,
    errors::SarcaResult,
    models::files::FSElement,
    repositories::{files::FilesRepository, storages::StoragesRepository},
    routers::files::{FilesRouter, warm_first_chunk},
};

/// Image extensions worth warming. Mirrors what `thumbnails::detect_kind`
/// treats as an image (HEIC/AVIF included — ffmpeg decodes those); this is a
/// selection filter for what to prefetch, and formats with a stored
/// preview/thumb id are served as-is regardless of extension.
const WARM_IMAGE_EXTENSIONS: [&str; 9] =
    ["jpg", "jpeg", "png", "webp", "heic", "heif", "avif", "gif", "bmp"];

/// Video extensions the player can stream, and therefore worth warming a first
/// chunk (and a stored preview) for.
const WARM_VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "mkv", "webm", "mov", "avi", "m4v"];

/// What a warm target is, which decides what gets warmed for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmKind {
    Image,
    Video,
}

/// One file selected for warming.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WarmTarget {
    path: String,
    kind: WarmKind,
    has_thumb: bool,
}

fn extension_of(name: &str) -> Option<String> {
    Path::new(name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase)
}

fn warm_kind(name: &str) -> Option<WarmKind> {
    let ext = extension_of(name)?;
    if WARM_IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return Some(WarmKind::Image);
    }
    if WARM_VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        return Some(WarmKind::Video);
    }
    None
}

/// Wake-up signal for the sweep loop, so a freshly created storage is walked
/// now rather than at the next interval.
static SWEEP_WAKE: LazyLock<Notify> = LazyLock::new(Notify::new);

/// Ask the warmer for an extra sweep as soon as it is idle. Cheap, lossy and
/// never blocks: a caller that fires this while a sweep is already running just
/// queues exactly one more.
pub fn request_sweep() {
    SWEEP_WAKE.notify_one();
}

/// Counts from one warmer run, logged as a single summary line.
#[derive(Debug, Default, Clone, Copy)]
pub struct WarmSummary {
    pub storages_walked: usize,
    pub thumbs_warmed: usize,
    pub previews_warmed: usize,
    pub chunks_warmed: usize,
    pub skipped: usize,
    pub errors: usize,
}

impl WarmSummary {
    fn merge(&mut self, other: Self) {
        self.storages_walked += other.storages_walked;
        self.thumbs_warmed += other.thumbs_warmed;
        self.previews_warmed += other.previews_warmed;
        self.chunks_warmed += other.chunks_warmed;
        self.skipped += other.skipped;
        self.errors += other.errors;
    }
}

/// Resolved knobs for one run, read out of `Config` once up front.
struct PrefetchSettings {
    depth: u32,
    concurrency: usize,
    max_items: usize,
    interval: Duration,
}

/// `None` when the daemon should not run at all this start.
fn settings_if_enabled(config: &Config) -> Option<PrefetchSettings> {
    if !config.prefetch_enabled {
        return None;
    }
    Some(PrefetchSettings {
        depth: config.prefetch_depth,
        // Zero would mean "no concurrency at all", wedging the walk forever.
        concurrency: config.prefetch_concurrency.max(1),
        max_items: config.prefetch_max_items,
        interval: Duration::from_secs(config.prefetch_interval_secs.max(60)),
    })
}

/// Spawn the warmer loop. Call once, right after the app state (and thus the
/// media/thumb semaphores it warms behind) exists.
pub fn spawn(state: Arc<AppState>) {
    tokio::spawn(async move {
        let Some(settings) = settings_if_enabled(&state.config) else {
            tracing::info!("[MEDIA WARMER] disabled (PREFETCH_ENABLED=0)");
            return;
        };

        loop {
            let started = Instant::now();
            let summary = warm_all(&state, &settings).await;
            tracing::info!(
                "[MEDIA WARMER] sweep done in {:.1}s: storages={} thumbs_warmed={} previews_warmed={} chunks_warmed={} skipped={} errors={}",
                started.elapsed().as_secs_f32(),
                summary.storages_walked,
                summary.thumbs_warmed,
                summary.previews_warmed,
                summary.chunks_warmed,
                summary.skipped,
                summary.errors,
            );

            // Whichever comes first: the next scheduled sweep, or someone
            // telling us there is new material (a storage was just created).
            tokio::select! {
                () = tokio::time::sleep(settings.interval) => {},
                () = SWEEP_WAKE.notified() => {
                    tracing::debug!("[MEDIA WARMER] sweep requested");
                },
            }
        }
    });
}

async fn warm_all(state: &Arc<AppState>, settings: &PrefetchSettings) -> WarmSummary {
    let mut summary = WarmSummary::default();

    let storage_ids = match StoragesRepository::new(&state.db).list_all_ids().await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("[MEDIA WARMER] failed to list storages: {e}");
            return summary;
        },
    };

    for storage_id in storage_ids {
        summary.storages_walked += 1;
        summary.merge(warm_storage(state, storage_id, settings).await);
    }

    summary
}

async fn warm_storage(
    state: &Arc<AppState>,
    storage_id: Uuid,
    settings: &PrefetchSettings,
) -> WarmSummary {
    let repo = FilesRepository::new(&state.db);
    let targets = collect_warm_targets(settings.depth, settings.max_items, |path| {
        let repo = &repo;
        async move { repo.list_dir(storage_id, &path).await }
    })
    .await;

    if targets.is_empty() {
        return WarmSummary::default();
    }

    let thumb_cache = MediaCache::thumbs(&state.config.work_dir);
    let preview_cache = MediaCache::previews(&state.config.work_dir);

    stream::iter(targets)
        .map(|target| {
            let thumb_cache = thumb_cache.clone();
            let preview_cache = preview_cache.clone();
            async move { warm_one(state, storage_id, &target, &thumb_cache, &preview_cache).await }
        })
        // Own concurrency cap, acquired ahead of the shared media/thumb
        // semaphores that `thumb_for_path`/`preview_for_path` acquire inside
        // `warm_one`: at most this many warmer requests ever reach those
        // semaphores at once, so an interactive request queued behind them
        // is never waiting on more than a handful of background jobs.
        .buffer_unordered(settings.concurrency)
        .fold(WarmSummary::default(), |mut acc, item| async move {
            acc.merge(item);
            acc
        })
        .await
}

/// One folder queued for listing during the BFS walk.
struct QueuedDir {
    path: String,
    depth: u32,
}

/// Breadth-first walk of a storage's tree to `max_depth` folders deep,
/// collecting every photo and video in listing order, capped at `max_items`.
/// Depth 0 is the storage root; a folder at exactly `max_depth` is still listed
/// (its files still count) but never descended into further, so the walk cost
/// stays bounded no matter how deep the real tree goes.
///
/// A missing thumb id is recorded, not a reason to skip: the preview endpoint
/// builds a JPEG from the original when nothing is stored, which is exactly the
/// slow first open this daemon exists to pay ahead of time.
async fn collect_warm_targets<F, Fut>(
    max_depth: u32,
    max_items: usize,
    mut list_dir: F,
) -> Vec<WarmTarget>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = SarcaResult<Vec<FSElement>>>,
{
    let mut queue = VecDeque::new();
    queue.push_back(QueuedDir {
        path: String::new(),
        depth: 0,
    });
    let mut targets = Vec::new();

    while let Some(dir) = queue.pop_front() {
        if targets.len() >= max_items {
            break;
        }
        let children = match list_dir(dir.path.clone()).await {
            Ok(children) => children,
            Err(e) => {
                tracing::debug!("[MEDIA WARMER] list_dir failed for '{}': {e}", dir.path);
                continue;
            },
        };
        for el in children {
            if targets.len() >= max_items {
                break;
            }
            if el.is_file {
                if let Some(kind) = warm_kind(&el.name) {
                    targets.push(WarmTarget {
                        path: el.path,
                        kind,
                        has_thumb: el.has_thumb,
                    });
                }
            } else if dir.depth < max_depth {
                queue.push_back(QueuedDir {
                    path: el.path,
                    depth: dir.depth + 1,
                });
            }
        }
    }

    targets
}

/// Warm every cache one file needs to open instantly. Skips (and counts)
/// whatever is already cached instead of re-downloading it, and treats a busy
/// storage (503, see `bound token wait on thumb/preview reads`) as a skip
/// rather than an error — the daemon backs off and moves on instead of
/// hammering a storage already at its Telegram rate limit.
///
/// * thumb — only when the file actually has one stored;
/// * preview — always, since the endpoint re-encodes a JPEG from the original when no preview
///   document exists (and backfills one for next time);
/// * first chunk — videos only, so the player's opening Range is a disk read.
async fn warm_one(
    state: &Arc<AppState>,
    storage_id: Uuid,
    target: &WarmTarget,
    thumb_cache: &MediaCache,
    preview_cache: &MediaCache,
) -> WarmSummary {
    let mut summary = WarmSummary::default();
    let path = target.path.as_str();

    if target.has_thumb {
        let thumb_key = thumb_cache.key(storage_id, path);
        if thumb_cache.get(&thumb_key).await.is_some() {
            summary.skipped += 1;
        } else {
            // Same code path `FilesRouter::thumb` serves from: cache check,
            // Telegram download, cache write. Nothing here duplicates it.
            match FilesRouter::thumb_for_path(state.clone(), storage_id, path).await {
                Ok(resp) if resp.status() == StatusCode::OK => summary.thumbs_warmed += 1,
                Ok(_) => summary.skipped += 1,
                Err((status, msg)) => {
                    tracing::debug!(
                        "[MEDIA WARMER] thumb warm failed for '{path}' ({status}): {msg}"
                    );
                    summary.errors += 1;
                },
            }
        }
    }

    let preview_key = preview_cache.key(storage_id, path);
    if preview_cache.get(&preview_key).await.is_some() {
        summary.skipped += 1;
    } else {
        match FilesRouter::preview_for_path(state.clone(), storage_id, path).await {
            Ok(resp) if resp.status() == StatusCode::OK => summary.previews_warmed += 1,
            // A video with no stored preview 415s from the preview endpoint —
            // expected, not a failure worth counting.
            Ok(_) | Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, _)) => summary.skipped += 1,
            Err((status, msg)) => {
                tracing::debug!(
                    "[MEDIA WARMER] preview warm failed for '{path}' ({status}): {msg}"
                );
                summary.errors += 1;
            },
        }
    }

    if target.kind == WarmKind::Video {
        match warm_first_chunk(state, storage_id, path).await {
            // `ensure` is a no-op when the chunk is already on disk, so this
            // counts a warm chunk whether this sweep or an earlier one paid.
            Ok(()) => summary.chunks_warmed += 1,
            Err(e) => {
                tracing::debug!("[MEDIA WARMER] first-chunk warm failed for '{path}': {e}");
                summary.errors += 1;
            },
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn el(path: &str, is_file: bool, has_thumb: bool) -> FSElement {
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        FSElement {
            path: path.to_owned(),
            name,
            size: 0,
            is_file,
            has_thumb,
        }
    }

    fn test_config(prefetch_enabled: bool) -> Config {
        Config {
            sqlite_path: String::new(),
            https_addr: "127.0.0.1:8443".parse().expect("valid addr"),
            acme_http_addr: "127.0.0.1:8080".parse().expect("valid addr"),
            tls_hostname: None,
            acme_directory: String::new(),
            acme_root_ca: None,
            certs_dir: String::new(),
            workers: 1,
            channel_capacity: 8,
            superuser_email: "a@b.c".into(),
            superuser_pass: "pass".into(),
            access_token_expire_in_secs: 1800,
            refresh_token_expire_in_days: 14,
            secret_key: "test-secret".into(),
            telegram_api_base_url: "https://api.telegram.org".into(),
            telegram_rate_limit: 60,
            telegram_proxy_url: None,
            upload_concurrency: 4,
            media_concurrency: 16,
            work_dir: String::new(),
            telegram_chunk_size_mb: 20,
            debug_log: false,
            prefetch_enabled,
            prefetch_depth: 3,
            prefetch_concurrency: 3,
            prefetch_max_items: 2000,
            prefetch_interval_secs: 600,
        }
    }

    fn paths(targets: &[WarmTarget]) -> Vec<String> {
        targets.iter().map(|t| t.path.clone()).collect()
    }

    /// A tiny fixed tree, deeper than `max_depth`, for BFS tests:
    ///
    /// ```text
    /// "" -> a/, root.jpg
    /// a -> a/b/, a/a1.jpg
    /// a/b -> a/b/c/, a/b/b1.jpg
    /// a/b/c -> a/b/c/d/, a/b/c/c1.jpg
    /// a/b/c/d -> a/b/c/d/d1.jpg   (depth 4 — beyond max_depth=3)
    /// ```
    fn fixture_tree() -> HashMap<String, Vec<FSElement>> {
        let mut tree = HashMap::new();
        tree.insert(String::new(), vec![el("a", false, false), el("root.jpg", true, true)]);
        tree.insert("a".to_owned(), vec![el("a/b", false, false), el("a/a1.jpg", true, true)]);
        tree.insert(
            "a/b".to_owned(),
            vec![el("a/b/c", false, false), el("a/b/b1.jpg", true, true)],
        );
        tree.insert(
            "a/b/c".to_owned(),
            vec![el("a/b/c/d", false, false), el("a/b/c/c1.jpg", true, true)],
        );
        tree.insert("a/b/c/d".to_owned(), vec![el("a/b/c/d/d1.jpg", true, true)]);
        tree
    }

    #[tokio::test]
    async fn depth_limit_is_respected() {
        let tree = fixture_tree();
        let targets = collect_warm_targets(3, 100, |path| {
            let tree = &tree;
            async move { Ok(tree.get(&path).cloned().unwrap_or_default()) }
        })
        .await;

        assert_eq!(
            paths(&targets),
            vec!["root.jpg", "a/a1.jpg", "a/b/b1.jpg", "a/b/c/c1.jpg"],
            "depth-3 folder must still be listed, but not descended into further",
        );
        assert!(!paths(&targets).contains(&"a/b/c/d/d1.jpg".to_owned()));
    }

    #[tokio::test]
    async fn max_items_cap_stops_the_walk_early() {
        let tree = fixture_tree();
        let targets = collect_warm_targets(3, 2, |path| {
            let tree = &tree;
            async move { Ok(tree.get(&path).cloned().unwrap_or_default()) }
        })
        .await;

        assert_eq!(targets.len(), 2);
    }

    #[tokio::test]
    async fn non_media_files_are_not_selected() {
        let mut tree = HashMap::new();
        tree.insert(
            String::new(),
            vec![
                el("notes.txt", true, true),
                el("archive.zip", true, false),
                el("photo.jpg", true, true),
            ],
        );
        let targets = collect_warm_targets(3, 100, |path| {
            let tree = &tree;
            async move { Ok(tree.get(&path).cloned().unwrap_or_default()) }
        })
        .await;

        assert_eq!(paths(&targets), vec!["photo.jpg"]);
    }

    /// A photo whose thumb was never built is exactly the one that costs a full
    /// download-and-re-encode on first open, so it must still be warmed — the
    /// thumb step is what gets skipped, not the file.
    #[tokio::test]
    async fn thumb_less_photo_is_still_warmed_for_preview() {
        let mut tree = HashMap::new();
        tree.insert(String::new(), vec![el("photo-no-thumb-yet.jpg", true, false)]);
        let targets = collect_warm_targets(3, 100, |path| {
            let tree = &tree;
            async move { Ok(tree.get(&path).cloned().unwrap_or_default()) }
        })
        .await;

        assert_eq!(
            targets,
            vec![WarmTarget {
                path: "photo-no-thumb-yet.jpg".to_owned(),
                kind: WarmKind::Image,
                has_thumb: false,
            }]
        );
    }

    #[tokio::test]
    async fn videos_are_selected_and_marked_as_videos() {
        let mut tree = HashMap::new();
        tree.insert(String::new(), vec![el("clip.MP4", true, true), el("movie.mkv", true, false)]);
        let targets = collect_warm_targets(3, 100, |path| {
            let tree = &tree;
            async move { Ok(tree.get(&path).cloned().unwrap_or_default()) }
        })
        .await;

        assert_eq!(paths(&targets), vec!["clip.MP4", "movie.mkv"]);
        assert!(targets.iter().all(|t| t.kind == WarmKind::Video));
    }

    #[test]
    fn warm_kind_reads_the_extension_case_insensitively() {
        assert_eq!(warm_kind("a.JPG"), Some(WarmKind::Image));
        assert_eq!(warm_kind("dir/b.HeIc"), Some(WarmKind::Image));
        assert_eq!(warm_kind("c.mov"), Some(WarmKind::Video));
        assert_eq!(warm_kind("d.txt"), None);
        assert_eq!(warm_kind("no-extension"), None);
    }

    #[tokio::test]
    async fn already_cached_thumb_is_skipped_without_touching_telegram() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::thumbs(dir.path());
        let storage_id = Uuid::new_v4();
        let key = cache.key(storage_id, "photo.jpg");
        cache.put(&key, b"already-warm").await.unwrap();

        assert!(cache.get(&key).await.is_some());
        // A different path must not be considered cached.
        assert!(cache.get(&cache.key(storage_id, "other.jpg")).await.is_none());
    }

    #[test]
    fn disabled_flag_does_nothing() {
        assert!(settings_if_enabled(&test_config(false)).is_none());
        let settings = settings_if_enabled(&test_config(true)).expect("enabled run has settings");
        assert_eq!(settings.depth, 3);
        assert_eq!(settings.concurrency, 3);
        assert_eq!(settings.max_items, 2000);
    }

    #[test]
    fn zero_concurrency_config_never_wedges_the_walk() {
        let mut cfg = test_config(true);
        cfg.prefetch_concurrency = 0;
        let settings = settings_if_enabled(&cfg).expect("enabled run has settings");
        assert_eq!(settings.concurrency, 1);
    }
}
