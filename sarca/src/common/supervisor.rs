//! Restart wrapper for the long-lived background loops.
//!
//! Every loop used to be a bare `tokio::spawn(async move { loop { .. } })` with
//! no `JoinHandle` retained. A panic anywhere inside killed that loop silently:
//! the process kept serving HTTP while replication, trash purge or storage purge
//! were simply gone, and nothing in the logs said so. `spawn_supervised` catches
//! the panic, logs it with the loop's name, and restarts with a capped backoff.

use std::{future::Future, time::Duration};

/// Delay before the first restart attempt.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Ceiling for the exponential backoff between restarts.
const MAX_BACKOFF: Duration = Duration::from_mins(1);
/// A run lasting at least this long is treated as healthy: the backoff resets.
const HEALTHY_RUN: Duration = Duration::from_mins(5);

/// Spawn `make_task` under a supervisor that restarts it if it panics or returns.
///
/// `make_task` is called once per run, so it must build a fresh future each time
/// (clone whatever state the loop needs inside the closure).
pub fn spawn_supervised<F, Fut>(name: &'static str, make_task: F)
where
    F: Fn() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let mut backoff = INITIAL_BACKOFF;
        loop {
            let started = tokio::time::Instant::now();
            let outcome = tokio::spawn(make_task()).await;
            let ran_for = started.elapsed();

            match outcome {
                Ok(()) => {
                    tracing::error!(
                        "[SUPERVISOR] background loop `{name}` returned after {ran_for:?}; restarting in {backoff:?}"
                    );
                },
                Err(e) if e.is_cancelled() => {
                    tracing::info!(
                        "[SUPERVISOR] background loop `{name}` cancelled; not restarting"
                    );
                    return;
                },
                Err(e) => {
                    tracing::error!(
                        "[SUPERVISOR] background loop `{name}` panicked after {ran_for:?} ({e}); restarting in {backoff:?}"
                    );
                },
            }

            tokio::time::sleep(backoff).await;
            backoff = if ran_for >= HEALTHY_RUN {
                INITIAL_BACKOFF
            } else {
                (backoff * 2).min(MAX_BACKOFF)
            };
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use super::*;

    #[tokio::test]
    async fn restarts_a_panicking_loop() {
        let runs = Arc::new(AtomicU32::new(0));
        let counter = runs.clone();

        tokio::time::pause();
        spawn_supervised("test", move || {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                panic!("boom");
            }
        });

        // Let the first run and two backoff windows elapse.
        for _ in 0..3 {
            tokio::time::advance(Duration::from_secs(10)).await;
            tokio::task::yield_now().await;
        }

        assert!(runs.load(Ordering::SeqCst) >= 2, "supervisor should restart the loop");
    }
}
