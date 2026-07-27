//! Per-binding fair scheduler.
//!
//! Prevents a single global lock from serializing sync passes across bindings
//! (e.g. Camera auto-upload vs. a large folder sync). Each binding id may only
//! have one run in flight at a time (skip-when-busy), while a shared semaphore
//! caps overall concurrency across all bindings.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

pub struct BindingScheduler {
    in_flight: Mutex<HashMap<String, ()>>,
    slots: Arc<Semaphore>,
}

impl BindingScheduler {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            in_flight: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(max_concurrent.max(1))),
        }
    }

    /// Runs `f` for `binding_id` unless a run for the same id is already in
    /// flight, in which case this returns `None` immediately without running
    /// `f`. Otherwise blocks until a concurrency permit is free, runs `f`,
    /// and returns `Some(result)`.
    pub async fn run<F, Fut, T>(&self, binding_id: &str, f: F) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        {
            let mut guard = self.in_flight.lock().await;
            if guard.contains_key(binding_id) {
                return None;
            }
            guard.insert(binding_id.to_string(), ());
        }
        let permit: OwnedSemaphorePermit = self.slots.clone().acquire_owned().await.ok()?;
        let result = f().await;
        drop(permit);
        self.in_flight.lock().await.remove(binding_id);
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn two_bindings_overlap_in_time() {
        let sched = BindingScheduler::new(2);
        let live = Arc::new(AtomicUsize::new(0));
        let max_live = Arc::new(AtomicUsize::new(0));

        let mk = |id: &'static str| {
            let sched = &sched;
            let live = live.clone();
            let max_live = max_live.clone();
            async move {
                sched
                    .run(id, || {
                        let live = live.clone();
                        let max_live = max_live.clone();
                        async move {
                            let n = live.fetch_add(1, Ordering::SeqCst) + 1;
                            max_live.fetch_max(n, Ordering::SeqCst);
                            sleep(Duration::from_millis(80)).await;
                            live.fetch_sub(1, Ordering::SeqCst);
                        }
                    })
                    .await
            }
        };

        let (a, b) = tokio::join!(mk("camera"), mk("folder"));
        assert!(a.is_some() && b.is_some());
        assert!(
            max_live.load(Ordering::SeqCst) >= 2,
            "folder must start while camera still running"
        );
    }

    #[tokio::test]
    async fn same_binding_skips_when_busy() {
        let sched = BindingScheduler::new(2);
        let (first, second) = tokio::join!(
            sched.run("cam", || async {
                sleep(Duration::from_millis(100)).await;
                1
            }),
            async {
                sleep(Duration::from_millis(10)).await;
                sched.run("cam", || async { 2 }).await
            }
        );
        assert_eq!(first, Some(1));
        assert_eq!(second, None);
    }
}
