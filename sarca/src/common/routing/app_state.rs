use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use sqlx::{Pool, Postgres};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{common::channels::ClientSender, config::Config};

#[derive(Debug, Clone)]
pub struct OAuthStateEntry {
    pub provider: String,
    pub expires_at: Instant,
}

#[derive(Debug, Clone)]
pub struct OAuthExchangeEntry {
    pub user_id: Uuid,
    pub email: String,
    pub email_verified: bool,
    pub expires_at: Instant,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub db: Pool<Postgres>,
    pub config: Config,
    pub tx: ClientSender,
    /// CSRF `state` → provider (single-use, short TTL).
    pub oauth_states: Arc<Mutex<HashMap<String, OAuthStateEntry>>>,
    /// One-time exchange codes → user (single-use, short TTL).
    pub oauth_exchanges: Arc<Mutex<HashMap<String, OAuthExchangeEntry>>>,
}

impl AppState {
    pub fn new(db: Pool<Postgres>, config: Config, tx: ClientSender) -> Self {
        Self {
            db,
            config,
            tx,
            oauth_states: Arc::new(Mutex::new(HashMap::new())),
            oauth_exchanges: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn put_oauth_state(&self, state: String, provider: String) {
        let mut map = self.oauth_states.lock().await;
        Self::purge_expired_states(&mut map);
        map.insert(
            state,
            OAuthStateEntry {
                provider,
                expires_at: Instant::now() + Duration::from_mins(10),
            },
        );
    }

    pub async fn take_oauth_state(&self, state: &str) -> Option<OAuthStateEntry> {
        let mut map = self.oauth_states.lock().await;
        Self::purge_expired_states(&mut map);
        map.remove(state).filter(|e| e.expires_at > Instant::now())
    }

    pub async fn put_oauth_exchange(
        &self,
        code: String,
        user_id: Uuid,
        email: String,
        email_verified: bool,
    ) {
        let mut map = self.oauth_exchanges.lock().await;
        Self::purge_expired_exchanges(&mut map);
        map.insert(
            code,
            OAuthExchangeEntry {
                user_id,
                email,
                email_verified,
                expires_at: Instant::now() + Duration::from_mins(2),
            },
        );
    }

    pub async fn take_oauth_exchange(&self, code: &str) -> Option<OAuthExchangeEntry> {
        let mut map = self.oauth_exchanges.lock().await;
        Self::purge_expired_exchanges(&mut map);
        map.remove(code).filter(|e| e.expires_at > Instant::now())
    }

    fn purge_expired_states(map: &mut HashMap<String, OAuthStateEntry>) {
        let now = Instant::now();
        map.retain(|_, v| v.expires_at > now);
    }

    fn purge_expired_exchanges(map: &mut HashMap<String, OAuthExchangeEntry>) {
        let now = Instant::now();
        map.retain(|_, v| v.expires_at > now);
    }
}
