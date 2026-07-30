use sqlx::{Pool, Sqlite};

use crate::{common::channels::ClientSender, config::Config};

#[derive(Debug, Clone)]
pub struct AppState {
    pub db: Pool<Sqlite>,
    pub config: Config,
    pub tx: ClientSender,
}

impl AppState {
    pub fn new(db: Pool<Sqlite>, config: Config, tx: ClientSender) -> Self {
        Self {
            db,
            config,
            tx,
        }
    }
}
