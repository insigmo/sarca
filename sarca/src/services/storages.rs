use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    common::{access::check_access, jwt_manager::AuthUser, telegram_api::bot_api::TelegramBotApi, types::ChatId},
    errors::{SarcaError, SarcaResult},
    models::{
        access::{AccessType, UserWithAccess},
        chunk_replicas::ReplicationStats,
        storage_channels::{InStorageChannel, StorageChannel},
        storages::{InStorage, Storage, StorageWithInfo},
    },
    repositories::{
        access::AccessRepository, chunk_replicas::ChunkReplicasRepository,
        storage_channels::StorageChannelsRepository, storages::StoragesRepository,
    },
    schemas::{
        access::{GrantAccess, RestrictAccess},
        storages::{
            AddChannelSchema, InStorageSchema, StorageDetailSchema, UpdateChannelSchema,
            UpdateStorageSchema,
        },
    },
    services::channel_health::ChannelHealthService,
};

use super::storage_workers_scheduler::StorageWorkersScheduler;

const MAX_CHANNELS: usize = 3;

pub struct StoragesService<'d> {
    repo: StoragesRepository<'d>,
    access_repo: AccessRepository<'d>,
    channels_repo: StorageChannelsRepository<'d>,
    replicas_repo: ChunkReplicasRepository<'d>,
    db: &'d PgPool,
    telegram_baseurl: &'d str,
    rate_limit: u8,
}

impl<'d> StoragesService<'d> {
    pub fn new(db: &'d PgPool, telegram_baseurl: &'d str, rate_limit: u8) -> Self {
        Self {
            repo: StoragesRepository::new(db),
            access_repo: AccessRepository::new(db),
            channels_repo: StorageChannelsRepository::new(db),
            replicas_repo: ChunkReplicasRepository::new(db),
            db,
            telegram_baseurl,
            rate_limit,
        }
    }

    /// Best-effort `getChat` lookup; falls back to `fallback` when Telegram can't be reached.
    async fn resolve_channel_name(
        &self,
        chat_id: ChatId,
        given_name: Option<String>,
        storage_id: Uuid,
        fallback: impl FnOnce() -> String,
    ) -> String {
        if let Some(name) = given_name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()) {
            return name;
        }

        let scheduler = StorageWorkersScheduler::new(self.db, self.rate_limit);
        match TelegramBotApi::new(self.telegram_baseurl, scheduler)
            .get_chat(chat_id, storage_id)
            .await
        {
            Ok(chat) => chat.title,
            Err(e) => {
                tracing::debug!("[STORAGES SERVICE] getChat failed for {chat_id}: {e}");
                fallback()
            }
        }
    }

    pub async fn create(&self, in_schema: InStorageSchema, user: &AuthUser) -> SarcaResult<Storage> {
        if in_schema.channels.is_empty() {
            return Err(SarcaError::NoActiveChannel);
        }
        if in_schema.channels.len() > MAX_CHANNELS {
            return Err(SarcaError::TooManyChannels);
        }

        if self
            .repo
            .get_by_name_and_user_id(&in_schema.name, user.id)
            .await
            .is_ok()
        {
            return Err(SarcaError::StorageNameConflict);
        }

        let in_model = InStorage::new(in_schema.name);
        let storage = self.repo.create(in_model).await?;

        tracing::debug!(
            "[STORAGES SERVICE] Created storage id={}, name={}",
            storage.id,
            storage.name
        );

        for (idx, input) in in_schema.channels.into_iter().enumerate() {
            let position = (idx + 1) as i16;
            let chat_id = input.chat_id;
            let name = self
                .resolve_channel_name(chat_id, input.name, storage.id, || {
                    format!("Channel {position}")
                })
                .await;
            let in_channel = InStorageChannel::active(storage.id, position, chat_id, name);
            if let Err(e) = self.channels_repo.insert(in_channel).await {
                tracing::error!(
                    "[STORAGES SERVICE] failed to insert channel for storage {}: {e}. Rolling back.",
                    storage.id
                );
                let _ = self.repo.delete_storage(storage.id).await;
                return Err(e);
            }
        }

        let access_schema = GrantAccess::new(user.email.clone(), AccessType::A);
        let result = self.access_repo.create_or_update(storage.id, access_schema).await;

        match &result {
            Ok(_) => {
                tracing::debug!(
                    "[STORAGES SERVICE] Successfully granted access to user {} for storage {}",
                    user.email,
                    storage.id
                );
            }
            Err(e) => {
                tracing::error!(
                    "[STORAGES SERVICE] Failed to grant access to user {} for storage {}: {:?}. Rolling back storage creation.",
                    user.email,
                    storage.id,
                    e
                );
                let _ = self.repo.delete_storage(storage.id).await;
            }
        }
        result.map(|_| storage)
    }

    pub async fn list(&self, user: &AuthUser) -> SarcaResult<Vec<StorageWithInfo>> {
        let storages = self.repo.list_by_user_id(user.id).await?;
        tracing::debug!(
            "[STORAGES SERVICE] Listed {} storages for user_id={}",
            storages.len(),
            user.id
        );
        Ok(storages)
    }

    pub async fn get(&self, id: Uuid, user: &AuthUser) -> SarcaResult<Storage> {
        check_access(&self.access_repo, user.id, id, &AccessType::R).await?;

        self.repo.get_by_id(id).await
    }

    pub async fn get_detail(&self, id: Uuid, user: &AuthUser) -> SarcaResult<StorageDetailSchema> {
        check_access(&self.access_repo, user.id, id, &AccessType::R).await?;

        let storage = self.repo.get_by_id(id).await?;
        let channels = self.channels_repo.list_by_storage(id).await?;
        let has_dead_channel = channels.iter().any(|c| c.is_dead());
        let replication = self.replicas_repo.replication_stats(id).await?;

        Ok(StorageDetailSchema {
            id: storage.id,
            name: storage.name,
            primary_position: storage.primary_position,
            has_dead_channel,
            channels,
            replication,
        })
    }

    pub async fn update(
        &self,
        id: Uuid,
        in_schema: UpdateStorageSchema,
        user: &AuthUser,
    ) -> SarcaResult<Storage> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        let name = in_schema.name.trim();
        if let Ok(existing) = self.repo.get_by_name_and_user_id(name, user.id).await {
            if existing.id != id {
                return Err(SarcaError::StorageNameConflict);
            }
            return Ok(existing);
        }

        self.repo.update_name(id, name).await
    }

    pub async fn delete(&self, id: Uuid, user: &AuthUser) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        self.repo.delete_storage(id).await
    }

    async fn ensure_channel_of_storage(
        &self,
        storage_id: Uuid,
        channel_id: Uuid,
    ) -> SarcaResult<StorageChannel> {
        let channel = self.channels_repo.get_by_id(channel_id).await?;
        if channel.storage_id != storage_id {
            return Err(SarcaError::DoesNotExist("channel".to_owned()));
        }
        Ok(channel)
    }

    pub async fn add_channel(
        &self,
        storage_id: Uuid,
        in_schema: AddChannelSchema,
        user: &AuthUser,
    ) -> SarcaResult<StorageChannel> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::A).await?;

        let position = self
            .channels_repo
            .next_free_position(storage_id)
            .await?
            .ok_or(SarcaError::TooManyChannels)?;

        let chat_id = in_schema.chat_id;
        let name = self
            .resolve_channel_name(chat_id, in_schema.name, storage_id, || {
                format!("Channel {position}")
            })
            .await;

        let in_channel = InStorageChannel::active(storage_id, position, chat_id, name);
        let channel = self.channels_repo.insert(in_channel).await?;

        if let Err(e) = self.replicas_repo.enqueue_for_channel(storage_id, channel.id).await {
            tracing::warn!(
                "[STORAGES SERVICE] failed to enqueue catch-up replication for channel {}: {e}",
                channel.id
            );
        }

        Ok(channel)
    }

    pub async fn update_channel(
        &self,
        storage_id: Uuid,
        channel_id: Uuid,
        patch: UpdateChannelSchema,
        user: &AuthUser,
    ) -> SarcaResult<StorageChannel> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::A).await?;

        let channel = self.ensure_channel_of_storage(storage_id, channel_id).await?;

        if let Some(chat_id) = patch.chat_id {
            let name = self
                .resolve_channel_name(chat_id, patch.name, storage_id, || channel.name.clone())
                .await;
            let updated = self.channels_repo.update_chat(channel_id, chat_id, &name).await?;

            if let Err(e) = self
                .replicas_repo
                .enqueue_for_channel(storage_id, channel_id)
                .await
            {
                tracing::warn!(
                    "[STORAGES SERVICE] failed to enqueue catch-up replication for channel {}: {e}",
                    channel_id
                );
            }

            Ok(updated)
        } else if let Some(name) = patch.name {
            let name = name.trim();
            if name.is_empty() {
                return Ok(channel);
            }
            self.channels_repo.update_name(channel_id, name).await
        } else {
            Ok(channel)
        }
    }

    pub async fn remove_channel(
        &self,
        storage_id: Uuid,
        channel_id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::A).await?;

        let channel = self.ensure_channel_of_storage(storage_id, channel_id).await?;

        if channel.is_active() {
            let active_count = self.channels_repo.count_active(storage_id).await?;
            if active_count <= 1 {
                return Err(SarcaError::LastActiveChannel);
            }
        }

        let storage = self.repo.get_by_id(storage_id).await?;
        if storage.primary_position == channel.position {
            let siblings = self.channels_repo.list_by_storage(storage_id).await?;
            if let Some(next) = ChannelHealthService::next_active_position(&siblings, channel.position) {
                let _ = self.repo.set_primary_position(storage_id, next).await;
            }
        }

        // Telegram messages are intentionally left in place; only the DB slot is freed.
        self.channels_repo.delete(channel_id).await
    }

    pub async fn retry_replication(
        &self,
        storage_id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<ReplicationStats> {
        check_access(&self.access_repo, user.id, storage_id, &AccessType::A).await?;

        self.replicas_repo.retry_failed(storage_id).await?;
        self.replicas_repo.replication_stats(storage_id).await
    }

    pub async fn grant_access(
        &self,
        id: Uuid,
        in_schema: GrantAccess,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        if in_schema.user_email == user.email {
            return Err(SarcaError::CannotManageAccessOfYourself);
        }

        self.access_repo.create_or_update(id, in_schema).await
    }

    pub async fn list_users_with_access(
        &self,
        id: Uuid,
        user: &AuthUser,
    ) -> SarcaResult<Vec<UserWithAccess>> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        self.access_repo.list_users_with_access(id).await
    }

    pub async fn restrict_access(
        &self,
        id: Uuid,
        in_schema: RestrictAccess,
        user: &AuthUser,
    ) -> SarcaResult<()> {
        check_access(&self.access_repo, user.id, id, &AccessType::A).await?;

        if in_schema.user_id == user.id {
            return Err(SarcaError::CannotManageAccessOfYourself);
        }

        self.access_repo.delete_access(in_schema.user_id, id).await
    }
}
