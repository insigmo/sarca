use sqlx::PgPool;

use super::storage_workers_scheduler::StorageWorkersScheduler;
use crate::{
    common::telegram_api::bot_api::{TelegramBotApi, is_chat_dead_error},
    models::storage_channels::StorageChannel,
    repositories::{storage_channels::StorageChannelsRepository, storages::StoragesRepository},
};

pub struct ChannelHealthService;

impl ChannelHealthService {
    /// Next active channel position to promote to primary, cycling forward from `current`
    /// (exclusive). Returns `None` if there is no active channel left at all.
    pub fn next_active_position(channels: &[StorageChannel], current: i16) -> Option<i16> {
        let mut active: Vec<i16> =
            channels.iter().filter(|c| c.is_active()).map(|c| c.position).collect();
        active.sort_unstable();

        if active.is_empty() {
            return None;
        }

        match active.iter().position(|&p| p == current) {
            Some(idx) => Some(active[(idx + 1) % active.len()]),
            // `current` isn't active (already dead / doesn't exist) — pick the lowest active.
            None => active.into_iter().next(),
        }
    }

    /// Probe every active channel via `getChat`; mark dead ones and rotate away any
    /// storage's primary position that pointed at a channel that just died.
    pub async fn run_once(db: &PgPool, base_url: &str, rate_limit: u8) {
        let channels_repo = StorageChannelsRepository::new(db);
        let storages_repo = StoragesRepository::new(db);

        let channels = match channels_repo.list_all().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("[CHANNEL HEALTH] failed to list channels: {e}");
                return;
            },
        };

        for channel in channels.iter().filter(|c| c.is_active()) {
            let scheduler = StorageWorkersScheduler::new(db, rate_limit);
            let api = TelegramBotApi::new(base_url, scheduler);

            let Err(e) = api.get_chat(channel.chat_id, channel.storage_id).await else {
                continue;
            };

            if !is_chat_dead_error(&e) {
                tracing::debug!(
                    "[CHANNEL HEALTH] transient error probing channel {} (chat_id={}): {e}",
                    channel.id,
                    channel.chat_id
                );
                continue;
            }

            tracing::warn!(
                "[CHANNEL HEALTH] channel {} (chat_id={}) looks dead: {e}",
                channel.id,
                channel.chat_id
            );
            if let Err(e) = channels_repo.mark_dead(channel.id).await {
                tracing::error!("[CHANNEL HEALTH] failed to mark channel {} dead: {e}", channel.id);
                continue;
            }

            let storage = match storages_repo.get_by_id(channel.storage_id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(
                        "[CHANNEL HEALTH] failed to load storage {}: {e}",
                        channel.storage_id
                    );
                    continue;
                },
            };

            if storage.primary_position != channel.position {
                continue;
            }

            let siblings = match channels_repo.list_by_storage(storage.id).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(
                        "[CHANNEL HEALTH] failed to list channels for storage {}: {e}",
                        storage.id
                    );
                    continue;
                },
            };

            match Self::next_active_position(&siblings, channel.position) {
                Some(next) => {
                    let _ = storages_repo.set_primary_position(storage.id, next).await;
                    tracing::info!(
                        "[CHANNEL HEALTH] rotated storage {} primary channel to position {next}",
                        storage.id
                    );
                },
                None => {
                    tracing::error!(
                        "[CHANNEL HEALTH] storage {} has no active channel left",
                        storage.id
                    );
                },
            }
        }
    }

    /// Spawn a background loop that health-checks every channel on a fixed interval.
    pub fn spawn_loop(db: PgPool, base_url: String, rate_limit: u8, interval: std::time::Duration) {
        tokio::spawn(async move {
            loop {
                Self::run_once(&db, &base_url, rate_limit).await;
                tokio::time::sleep(interval).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::ChannelHealthService;
    use crate::models::storage_channels::StorageChannel;

    fn channel(position: i16, status: &str) -> StorageChannel {
        StorageChannel {
            id: Uuid::new_v4(),
            storage_id: Uuid::new_v4(),
            position,
            chat_id: -1_001_234_567_890,
            name: format!("channel-{position}"),
            status: status.to_owned(),
        }
    }

    #[test]
    fn cycles_forward_through_active_channels() {
        let channels = vec![channel(1, "active"), channel(2, "active"), channel(3, "active")];
        assert_eq!(ChannelHealthService::next_active_position(&channels, 1), Some(2));
        assert_eq!(ChannelHealthService::next_active_position(&channels, 2), Some(3));
        assert_eq!(ChannelHealthService::next_active_position(&channels, 3), Some(1));
    }

    #[test]
    fn skips_dead_channels_when_cycling() {
        let channels = vec![channel(1, "dead"), channel(2, "active"), channel(3, "active")];
        assert_eq!(ChannelHealthService::next_active_position(&channels, 2), Some(3));
        assert_eq!(ChannelHealthService::next_active_position(&channels, 3), Some(2));
    }

    #[test]
    fn falls_back_to_lowest_active_when_current_not_active() {
        let channels = vec![channel(1, "dead"), channel(2, "active"), channel(3, "active")];
        assert_eq!(ChannelHealthService::next_active_position(&channels, 1), Some(2));
    }

    #[test]
    fn returns_none_when_no_active_channel_remains() {
        let channels = vec![channel(1, "dead"), channel(2, "dead")];
        assert_eq!(ChannelHealthService::next_active_position(&channels, 1), None);
    }

    #[test]
    fn single_active_channel_cycles_to_itself() {
        let channels = vec![channel(1, "active")];
        assert_eq!(ChannelHealthService::next_active_position(&channels, 1), Some(1));
    }
}
