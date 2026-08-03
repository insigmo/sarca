use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Prune dead slots once the map grows past this many keys. Slots are tiny, so
/// this is about bounding a long-lived process, not about reclaiming much.
const PRUNE_THRESHOLD: usize = 1024;

/// Serializes identical cache-fill work by key.
///
/// Opening a photo folder fires many requests for the same few blobs at once
/// (a grid repaint, a neighbor prefetch, a second device). Without this, every
/// one of them misses the cold cache simultaneously and each pays its own
/// Telegram round trip — or, on the slow preview path, its own full-file
/// download and JPEG re-encode. Holding the key's lock means the first request
/// does that work and the rest find the finished bytes on disk.
///
/// This is a fairness lock, not a result cache: callers must re-check their
/// cache after acquiring, since the request they queued behind has very likely
/// just filled it.
pub struct SingleFlight {
    slots: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
}

impl SingleFlight {
    /// Process-wide registry. Deduplication only helps between concurrent
    /// requests, which all live in this one process.
    pub fn global() -> &'static Self {
        static INSTANCE: OnceLock<SingleFlight> = OnceLock::new();
        INSTANCE.get_or_init(|| {
            Self {
                slots: Mutex::new(HashMap::new()),
            }
        })
    }

    fn slot_for(&self, key: &str) -> Arc<AsyncMutex<()>> {
        let mut slots = self.slots.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = slots.get(key).and_then(Weak::upgrade) {
            return existing;
        }
        // Every slot whose holders have all finished is dead weight; drop them
        // in one pass rather than tracking liveness per release.
        if slots.len() >= PRUNE_THRESHOLD {
            slots.retain(|_, slot| slot.strong_count() > 0);
        }
        let slot = Arc::new(AsyncMutex::new(()));
        slots.insert(key.to_owned(), Arc::downgrade(&slot));
        slot
    }

    /// Wait for exclusive rights to fill `key`. The returned guard keeps the
    /// slot alive; drop it once the cache has been written.
    pub async fn acquire(&self, key: &str) -> OwnedMutexGuard<()> {
        self.slot_for(key).lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn same_key_runs_one_at_a_time() {
        let flight = SingleFlight::global();
        let key = format!("test-serial-{}", uuid::Uuid::new_v4());
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..8 {
            let key = key.clone();
            let concurrent = Arc::clone(&concurrent);
            let peak = Arc::clone(&peak);
            handles.push(tokio::spawn(async move {
                let _guard = flight.acquire(&key).await;
                let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::task::yield_now().await;
                concurrent.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_keys_do_not_block_each_other() {
        let flight = SingleFlight::global();
        let held = flight.acquire("test-key-a").await;
        // Would deadlock if distinct keys shared a slot.
        let _other = flight.acquire("test-key-b").await;
        drop(held);
    }

    #[tokio::test]
    async fn released_slots_are_reclaimed() {
        let flight = SingleFlight::global();
        let key = format!("test-reclaim-{}", uuid::Uuid::new_v4());
        drop(flight.acquire(&key).await);

        let live = {
            let slots = flight.slots.lock().unwrap();
            slots.get(&key).map(Weak::strong_count)
        };
        assert_eq!(live, Some(0));
    }
}
