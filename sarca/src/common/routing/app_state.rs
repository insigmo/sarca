use std::sync::Arc;

use sqlx::{Pool, Sqlite};
use tokio::sync::Semaphore;

use crate::{
    common::{channels::ClientSender, throttle::FailureThrottle},
    config::Config,
};

#[derive(Debug, Clone)]
pub struct AppState {
    pub db: Pool<Sqlite>,
    pub config: Config,
    pub tx: ClientSender,
    /// Shared brake for every unauthenticated secret comparison (login, share
    /// unlock, reset mail). Cloning `AppState` keeps the same counters.
    pub throttle: FailureThrottle,
    /// Bounds concurrent preview/inline/download requests blocked on Telegram,
    /// independent of the rest of the API — see `config.media_concurrency`.
    pub media_semaphore: Arc<Semaphore>,
    /// Bounds concurrent grid-thumbnail reads and chunk prefetch only, carved
    /// out of `media_concurrency` so scrolling a large folder can never fill
    /// every permit: an interactive preview open always finds at least a few
    /// spare on `media_semaphore`.
    pub thumb_semaphore: Arc<Semaphore>,
}

impl AppState {
    pub fn new(db: Pool<Sqlite>, config: Config, tx: ClientSender) -> Self {
        let media_semaphore = Arc::new(Semaphore::new(config.media_concurrency as usize));
        let thumb_semaphore = Arc::new(Semaphore::new(thumb_concurrency(config.media_concurrency)));
        Self {
            db,
            config,
            tx,
            throttle: FailureThrottle::new(),
            media_semaphore,
            thumb_semaphore,
        }
    }
}

/// Reserve 4 permits off the top for interactive reads; never drop below 1
/// so a small `MEDIA_CONCURRENCY` still leaves thumbs able to make progress.
fn thumb_concurrency(media_concurrency: u16) -> usize {
    (media_concurrency as usize).saturating_sub(4).max(1)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::Semaphore;

    use super::thumb_concurrency;

    #[test]
    fn reserves_four_permits_for_interactive_reads() {
        assert_eq!(thumb_concurrency(16), 12);
    }

    #[test]
    fn never_drops_below_one_permit() {
        assert_eq!(thumb_concurrency(1), 1);
        assert_eq!(thumb_concurrency(4), 1);
    }

    /// The whole point of the split: a folder scroll that fills every thumb
    /// permit must still leave `media_semaphore` acquirable for the preview
    /// the user is actually waiting on. See `preview_for_path` /
    /// `thumb_for_path` in `routers::files`, which acquire these two
    /// semaphores respectively.
    #[tokio::test]
    async fn saturated_thumb_semaphore_does_not_starve_media_semaphore() {
        let media_semaphore = Arc::new(Semaphore::new(16));
        let thumb_semaphore = Arc::new(Semaphore::new(thumb_concurrency(16)));

        // Simulate a folder scroll: fill every thumb permit and hold them.
        let _thumb_permits: Vec<_> =
            std::iter::from_fn(|| thumb_semaphore.clone().try_acquire_owned().ok()).collect();
        assert_eq!(thumb_semaphore.available_permits(), 0);

        // The interactive preview request must still get a permit immediately.
        assert!(
            media_semaphore.try_acquire().is_ok(),
            "preview_for_path must not wait behind thumb reads"
        );
    }
}
