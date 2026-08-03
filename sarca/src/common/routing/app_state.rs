use sqlx::{Pool, Sqlite};

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
}

impl AppState {
    pub fn new(db: Pool<Sqlite>, config: Config, tx: ClientSender) -> Self {
        Self {
            db,
            config,
            tx,
            throttle: FailureThrottle::new(),
        }
    }
}
