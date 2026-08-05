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
    /// Bounds concurrent thumb/preview/download requests blocked on Telegram,
    /// independent of the rest of the API — see `config.media_concurrency`.
    pub media_semaphore: Arc<Semaphore>,
}

impl AppState {
    pub fn new(db: Pool<Sqlite>, config: Config, tx: ClientSender) -> Self {
        let media_semaphore = Arc::new(Semaphore::new(config.media_concurrency as usize));
        Self {
            db,
            config,
            tx,
            throttle: FailureThrottle::new(),
            media_semaphore,
        }
    }
}
