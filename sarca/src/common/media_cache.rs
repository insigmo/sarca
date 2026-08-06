use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use uuid::Uuid;

pub const PREVIEW_CACHE_LIMIT_BYTES: u64 = 1 << 30;
pub const PREVIEW_CACHE_EVICT_BYTES: u64 = 100 << 20;
pub const PREVIEW_FORMAT_VERSION: &str = "v3-2560-500kb-lanczos3";

pub const THUMB_CACHE_LIMIT_BYTES: u64 = 256 << 20;
pub const THUMB_CACHE_EVICT_BYTES: u64 = 32 << 20;
pub const THUMB_FORMAT_VERSION: &str = "v2-320";

/// On-disk cache of encoded JPEGs under `WORK_DIR/<dir>`.
///
/// Covers both previews and grid thumbnails, each under its own root so a
/// large preview working set cannot evict the much cheaper thumbnail tiles.
#[derive(Clone, Debug)]
pub struct MediaCache {
    root: PathBuf,
    limit: u64,
    evict: u64,
    format_version: &'static str,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

impl MediaCache {
    pub fn previews(work_dir: impl AsRef<Path>) -> Self {
        Self {
            root: work_dir.as_ref().join("preview_cache"),
            limit: PREVIEW_CACHE_LIMIT_BYTES,
            evict: PREVIEW_CACHE_EVICT_BYTES,
            format_version: PREVIEW_FORMAT_VERSION,
        }
    }

    pub fn thumbs(work_dir: impl AsRef<Path>) -> Self {
        Self {
            root: work_dir.as_ref().join("thumb_cache"),
            limit: THUMB_CACHE_LIMIT_BYTES,
            evict: THUMB_CACHE_EVICT_BYTES,
            format_version: THUMB_FORMAT_VERSION,
        }
    }

    /// Cache key for a logical file. The format version is folded in so that
    /// changing the encode parameters invalidates old entries instead of
    /// serving stale sizes forever.
    pub fn key(&self, storage_id: Uuid, logical_path: &str) -> String {
        let mut h = DefaultHasher::new();
        storage_id.hash(&mut h);
        logical_path.hash(&mut h);
        self.format_version.hash(&mut h);
        format!("{:016x}", h.finish())
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.jpg"))
    }

    /// Read cached JPEG bytes and touch mtime for LRU-ish eviction.
    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.path_for(key);
        let bytes = tokio::fs::read(&path).await.ok()?;
        if bytes.is_empty() {
            return None;
        }
        touch(&path).await;
        Some(bytes)
    }

    pub async fn remove(&self, key: &str) {
        let _ = tokio::fs::remove_file(self.path_for(key)).await;
    }

    /// Atomically store JPEG bytes, then evict oldest entries if over limit.
    pub async fn put(&self, key: &str, bytes: &[u8]) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.root).await.map_err(|e| e.to_string())?;
        let dest = self.path_for(key);
        let tmp = self.root.join(format!("{}.tmp", Uuid::new_v4()));
        if let Err(e) = tokio::fs::write(&tmp, bytes).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e.to_string());
        }
        if dest.is_file() {
            let _ = tokio::fs::remove_file(&dest).await;
        }
        if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(e.to_string());
        }
        self.ensure_under_limit(self.limit, self.evict).await
    }

    /// Once over `limit`, delete oldest files down to `limit - min_evict` so a
    /// single put does not trigger an eviction pass on every subsequent put.
    pub async fn ensure_under_limit(&self, limit: u64, min_evict: u64) -> Result<(), String> {
        // No grace needed: `get` reads the bytes into memory before returning,
        // so nothing here holds a path it has yet to open.
        let target = limit.saturating_sub(min_evict);
        evict_oldest(&self.root, target, Duration::ZERO).await
    }
}

/// Delete the oldest files in `root` until it fits in `limit`.
///
/// Shared with the chunk cache: same LRU-ish policy by mtime, and every cache
/// here is touched on read, so "oldest" means "least recently used".
///
/// Entries younger than `grace` are counted but never deleted, so a reader that
/// has just been handed a path still finds the file when it opens it.
pub async fn evict_oldest(root: &Path, limit: u64, grace: Duration) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }

    let root = root.to_path_buf();
    let (mut total, mut entries) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut total = 0u64;
        let mut entries = Vec::new();
        let read_dir = std::fs::read_dir(&root).map_err(|e| e.to_string())?;
        for entry in read_dir.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            let size = meta.len();
            total = total.saturating_add(size);
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push(CacheEntry {
                path,
                size,
                modified,
            });
        }
        Ok((total, entries))
    })
    .await
    .map_err(|e| e.to_string())??;

    if total <= limit {
        return Ok(());
    }

    entries.sort_by_key(|e| e.modified);
    let cutoff = SystemTime::now().checked_sub(grace).unwrap_or(SystemTime::UNIX_EPOCH);
    for entry in entries {
        if total <= limit {
            break;
        }
        if entry.modified > cutoff {
            continue;
        }
        if tokio::fs::remove_file(&entry.path).await.is_ok() {
            total = total.saturating_sub(entry.size);
        }
    }

    Ok(())
}

/// Bump a cached file's mtime so eviction sees it as recently used.
pub async fn touch(path: &Path) {
    let path = path.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&path) {
            let _ = f.set_modified(SystemTime::now());
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    async fn write_file(path: &Path, bytes: &[u8]) {
        tokio::fs::write(path, bytes).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn eviction_deletes_oldest_until_under_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = MediaCache::previews(tmp.path());
        tokio::fs::create_dir_all(&cache.root).await.unwrap();

        write_file(&cache.path_for("old"), &[0u8; 600]).await;
        write_file(&cache.path_for("mid"), &[0u8; 500]).await;
        write_file(&cache.path_for("new"), &[0u8; 400]).await;

        // limit 900, min_evict 100 => evicts down to the 800-byte target, so
        // both "old" and "mid" are removed and only "new" (400) survives.
        cache.ensure_under_limit(900, 100).await.unwrap();

        assert!(!cache.path_for("old").exists());
        assert!(!cache.path_for("mid").exists());
        assert!(cache.path_for("new").exists());

        let mut total = 0u64;
        let mut read_dir = tokio::fs::read_dir(&cache.root).await.unwrap();
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            if let Ok(meta) = entry.metadata().await {
                total += meta.len();
            }
        }
        assert!(total <= 800);
    }

    #[tokio::test]
    async fn thumbs_and_previews_use_separate_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let previews = MediaCache::previews(tmp.path());
        let thumbs = MediaCache::thumbs(tmp.path());
        let storage_id = Uuid::new_v4();

        assert_ne!(previews.root, thumbs.root);

        let preview_key = previews.key(storage_id, "photos/a.jpg");
        let thumb_key = thumbs.key(storage_id, "photos/a.jpg");
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        previews.put(&preview_key, &jpeg).await.unwrap();
        thumbs.put(&thumb_key, &jpeg).await.unwrap();

        // A large preview working set must not evict thumb tiles: bump the
        // preview cache well past its limit and confirm the thumb survives.
        previews.ensure_under_limit(0, 0).await.unwrap();
        assert!(previews.get(&preview_key).await.is_none());
        assert_eq!(thumbs.get(&thumb_key).await.as_deref(), Some(jpeg.as_slice()));
    }

    #[tokio::test]
    async fn eviction_spares_entries_inside_the_grace_window() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("cache");
        tokio::fs::create_dir_all(&root).await.unwrap();
        write_file(&root.join("a.bin"), &[0u8; 600]).await;
        write_file(&root.join("b.bin"), &[0u8; 600]).await;

        evict_oldest(&root, 100, Duration::from_mins(1)).await.unwrap();

        assert!(root.join("a.bin").exists());
        assert!(root.join("b.bin").exists());
    }

    #[tokio::test]
    async fn put_and_get_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = MediaCache::previews(tmp.path());
        let key = cache.key(Uuid::new_v4(), "photos/a.jpg");
        let jpeg = [0xFF, 0xD8, 0xFF, 0xD9];
        cache.put(&key, &jpeg).await.unwrap();
        assert_eq!(cache.get(&key).await.as_deref(), Some(jpeg.as_slice()));
    }

    #[tokio::test]
    async fn keys_differ_per_storage_and_path() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = MediaCache::previews(tmp.path());
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        assert_ne!(cache.key(a, "x.jpg"), cache.key(b, "x.jpg"));
        assert_ne!(cache.key(a, "x.jpg"), cache.key(a, "y.jpg"));
    }
}
