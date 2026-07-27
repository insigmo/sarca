//! Per-binding fair scheduler.
//!
//! Prevents a single global lock from serializing sync passes across bindings
//! (e.g. Camera auto-upload vs. a large folder sync). Each binding id may only
//! have one run in flight at a time (skip-when-busy), while a shared semaphore
//! caps overall concurrency across all bindings.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// RAII guard that removes `binding_id` from the shared in-flight map when
/// dropped. This runs on every exit path from [`BindingScheduler::run`] —
/// normal return, early return (e.g. semaphore acquire failure), and panic
/// unwinding inside `f().await` — so a binding can never get stuck "in
/// flight" forever and be skipped on every subsequent call.
struct InFlightGuard {
    in_flight: Arc<StdMutex<HashMap<String, ()>>>,
    binding_id: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.in_flight.lock() {
            guard.remove(&self.binding_id);
        }
    }
}

pub struct BindingScheduler {
    in_flight: Arc<StdMutex<HashMap<String, ()>>>,
    slots: Arc<Semaphore>,
}

impl BindingScheduler {
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            in_flight: Arc::new(StdMutex::new(HashMap::new())),
            slots: Arc::new(Semaphore::new(max_concurrent.max(1))),
        }
    }

    /// Runs `f` for `binding_id` unless a run for the same id is already in
    /// flight, in which case this returns `None` immediately without running
    /// `f`. Otherwise blocks until a concurrency permit is free, runs `f`,
    /// and returns `Some(result)`.
    ///
    /// The in-flight entry is released via an RAII guard rather than an
    /// explicit `remove` at the end of the function, so it is released on
    /// every exit path (early return via `?`, or a panic unwinding out of
    /// `f().await`) — not just the success path.
    pub async fn run<F, Fut, T>(&self, binding_id: &str, f: F) -> Option<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let _guard = {
            let mut guard = self.in_flight.lock().ok()?;
            if guard.contains_key(binding_id) {
                return None;
            }
            guard.insert(binding_id.to_string(), ());
            InFlightGuard {
                in_flight: self.in_flight.clone(),
                binding_id: binding_id.to_string(),
            }
        };
        let permit: OwnedSemaphorePermit = self.slots.clone().acquire_owned().await.ok()?;
        let result = f().await;
        drop(permit);
        Some(result)
        // `_guard` drops here (or during unwind on an early return/panic
        // above), releasing the in-flight entry exactly once.
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

    /// If `f` panics mid-run, the in-flight entry must still be released
    /// (via the RAII guard's `Drop`), so a later run for the *same* binding
    /// id is not skipped forever. The panic is driven through a spawned
    /// task so tokio's task boundary catches the unwind (mirrors how
    /// panics inside real binding sync work would surface today), and we
    /// assert on the resulting `JoinError` plus a successful follow-up run.
    #[tokio::test]
    async fn in_flight_released_after_panic() {
        let sched = Arc::new(BindingScheduler::new(2));

        let sched_for_panic = sched.clone();
        let join_result = tokio::spawn(async move {
            sched_for_panic
                .run("cam", || async { panic!("boom: simulated sync panic") })
                .await
        })
        .await;
        assert!(
            join_result.is_err(),
            "panic inside f() should propagate as a JoinError"
        );

        // The in-flight entry for "cam" must have been released by the
        // guard's Drop despite the panic, so this run is accepted (Some),
        // not skipped (None) as it would be with a leaked entry.
        let second = sched.run("cam", || async { 42 }).await;
        assert_eq!(
            second,
            Some(42),
            "binding must not be permanently stuck in-flight after a panic"
        );
    }

    /// Unit-tests `InFlightGuard`'s `Drop` directly, independent of tokio
    /// task/panic machinery: inserting then dropping the guard must remove
    /// the entry from the shared map.
    #[test]
    fn in_flight_guard_removes_entry_on_drop() {
        let map: Arc<StdMutex<HashMap<String, ()>>> = Arc::new(StdMutex::new(HashMap::new()));
        map.lock().unwrap().insert("cam".to_string(), ());
        assert!(map.lock().unwrap().contains_key("cam"));

        let guard = InFlightGuard {
            in_flight: map.clone(),
            binding_id: "cam".to_string(),
        };
        assert!(map.lock().unwrap().contains_key("cam"));
        drop(guard);
        assert!(
            !map.lock().unwrap().contains_key("cam"),
            "Drop must remove the binding id from the in-flight map"
        );
    }
}
