use sqlx::SqlitePool;

use crate::{
    common::{
        access::check_access,
        jwt_manager::AuthUser,
        telegram_api::token_client::TelegramTokenClient,
        types::ChatId,
    },
    conf,
    errors::{SarcaError, SarcaResult},
    models::{access::AccessType, storages::Storage},
    repositories::{
        access::AccessRepository,
        storage_channels::StorageChannelsRepository,
        storages::StoragesRepository,
    },
    schemas::{
        setup::{
            BotValidateSchema,
            ChannelPollHitSchema,
            ChannelPollResultSchema,
            SetupCreateStorageResultSchema,
            SetupCreateStorageSchema,
            SetupStatusSchema,
        },
        storages::{ChannelInput, InStorageSchema, SetStorageBotSchema},
    },
    services::storages::StoragesService,
};

pub struct SetupService<'d> {
    db: &'d SqlitePool,
    telegram_base_url: &'d str,
    rate_limit: u16,
    storages_repo: StoragesRepository<'d>,
}

impl<'d> SetupService<'d> {
    pub fn new(db: &'d SqlitePool, telegram_base_url: &'d str, rate_limit: u16) -> Self {
        Self {
            db,
            telegram_base_url,
            rate_limit,
            storages_repo: StoragesRepository::new(db),
        }
    }

    pub async fn status(&self, user: &AuthUser) -> SarcaResult<SetupStatusSchema> {
        let storages = self.storages_repo.list_by_user_id(user.id).await?;
        Ok(SetupStatusSchema {
            has_storages: !storages.is_empty(),
            conf_writable: conf::resolve_conf_path().is_some(),
        })
    }

    pub async fn validate_bot(&self, token: &str) -> SarcaResult<BotValidateSchema> {
        let token = token.trim();
        if token.is_empty() || !token.contains(':') {
            return Err(SarcaError::TelegramAPIError("Bot token looks invalid".into()));
        }
        let client = TelegramTokenClient::new(self.telegram_base_url, token);
        let me = client.get_me().await?;
        // Ensure getUpdates works during channel detect.
        if let Err(e) = client.delete_webhook().await {
            tracing::warn!("deleteWebhook during setup validate: {e}");
        }

        // Chat ids already bound to any storage are not free for a new one.
        let occupied = StorageChannelsRepository::new(self.db).list_all_chat_ids().await?;
        // Discover now (sets allowed_updates + drains pending updates) so Continue
        // lands on the channel step with every free admin chat already listed.
        let (found, _) = client.discover_admin_chats(&occupied, &[]).await?;
        let channels = found
            .into_iter()
            .take(3)
            .map(|(chat_id, title)| {
                ChannelPollHitSchema {
                    chat_id,
                    title,
                }
            })
            .collect::<Vec<_>>();

        Ok(BotValidateSchema {
            bot_id: me.id,
            username: me.username,
            channels,
        })
    }

    /// Discover chats where the bot is admin/creator (negative chat ids only).
    /// `exclude` skips chat ids already known (e.g. on this storage).
    pub async fn discover_admin_chats(
        &self,
        token: &str,
        exclude: &[ChatId],
        probe: &[ChatId],
    ) -> SarcaResult<(Vec<(ChatId, String)>, Option<String>)> {
        let client = TelegramTokenClient::new(self.telegram_base_url, token.trim());
        client.discover_admin_chats(exclude, probe).await
    }

    pub async fn poll_channel(
        &self,
        token: &str,
        exclude: &[ChatId],
        probe: &[ChatId],
    ) -> SarcaResult<ChannelPollResultSchema> {
        // Also skip chat ids already owned by any storage (unique globally).
        let mut skip = StorageChannelsRepository::new(self.db).list_all_chat_ids().await?;
        for id in exclude {
            if !skip.contains(id) {
                skip.push(*id);
            }
        }
        let (found, hint) = self.discover_admin_chats(token, &skip, probe).await?;
        // Cap to remaining slots (max 3 channels per storage).
        let room = 3usize.saturating_sub(exclude.len());
        let channels = found
            .into_iter()
            .take(room)
            .map(|(chat_id, title)| {
                ChannelPollHitSchema {
                    chat_id,
                    title,
                }
            })
            .collect::<Vec<_>>();
        let hint = if channels.is_empty() { hint } else { None };
        Ok(ChannelPollResultSchema {
            channels,
            hint,
        })
    }

    pub async fn create_storage(
        &self,
        body: SetupCreateStorageSchema,
        user: &AuthUser,
    ) -> SarcaResult<SetupCreateStorageResultSchema> {
        let name = body.name.trim().to_owned();
        if name.is_empty() {
            return Err(SarcaError::InvalidFolderName);
        }
        if body.chat_ids.is_empty() || body.chat_ids.len() > 3 {
            return Err(SarcaError::NoActiveChannel);
        }
        for id in &body.chat_ids {
            if *id >= 0 {
                return Err(SarcaError::TelegramAPIError(
                    "chat_id must be a negative Telegram channel/group id".into(),
                ));
            }
        }

        let token = body.token.trim().to_owned();
        let client = TelegramTokenClient::new(self.telegram_base_url, &token);
        let _me = client.get_me().await?;

        let storages = StoragesService::new(self.db, self.telegram_base_url, self.rate_limit);

        // Retry path: previous Finish may have left channels without a worker.
        if let Some(existing) = self.storage_owned_by_chats(&body.chat_ids, user).await? {
            storages
                .set_bot(
                    existing.id,
                    SetStorageBotSchema {
                        token,
                    },
                    user,
                )
                .await?;
            return Ok(SetupCreateStorageResultSchema {
                id: existing.id,
                name: existing.name,
            });
        }

        let mut channels = Vec::with_capacity(body.chat_ids.len());
        for (i, chat_id) in body.chat_ids.iter().copied().enumerate() {
            let title = match client.get_chat(chat_id).await {
                Ok(info) => Some(info.title),
                Err(_) => Some(format!("Channel {}", i + 1)),
            };
            channels.push(ChannelInput {
                chat_id,
                name: title,
            });
        }

        let storage = match storages
            .create(
                InStorageSchema {
                    name: name.clone(),
                    channels,
                },
                user,
            )
            .await
        {
            Ok(s) => s,
            Err(SarcaError::StorageChatIdConflict) => {
                // Lost race / partial prior attempt — recover if we own the chats.
                if let Some(existing) = self.storage_owned_by_chats(&body.chat_ids, user).await? {
                    storages
                        .set_bot(
                            existing.id,
                            SetStorageBotSchema {
                                token,
                            },
                            user,
                        )
                        .await?;
                    return Ok(SetupCreateStorageResultSchema {
                        id: existing.id,
                        name: existing.name,
                    });
                }
                return Err(SarcaError::StorageChatIdConflict);
            },
            Err(e) => return Err(e),
        };

        if let Err(e) = storages
            .set_bot(
                storage.id,
                SetStorageBotSchema {
                    token,
                },
                user,
            )
            .await
        {
            tracing::error!(
                "setup: storage {} created but set_bot failed: {e:?}; force-deleting storage",
                storage.id
            );
            // Bypass access check — we just created this row; delete must not leave orphans.
            if let Err(del_e) = self.storages_repo.delete_storage(storage.id).await {
                tracing::error!(
                    "setup: force-delete of orphan storage {} failed: {del_e:?}",
                    storage.id
                );
            }
            return Err(e);
        }

        Ok(SetupCreateStorageResultSchema {
            id: storage.id,
            name: storage.name,
        })
    }

    /// If every `chat_id` belongs to the same storage and this user is admin, return it.
    async fn storage_owned_by_chats(
        &self,
        chat_ids: &[ChatId],
        user: &AuthUser,
    ) -> SarcaResult<Option<Storage>> {
        if chat_ids.is_empty() {
            return Ok(None);
        }
        let channels = StorageChannelsRepository::new(self.db);
        let mut storage_id = None;
        for &chat_id in chat_ids {
            let ch = match channels.get_by_chat_id(chat_id).await {
                Ok(c) => c,
                Err(SarcaError::DoesNotExist(_)) => return Ok(None),
                Err(e) => return Err(e),
            };
            match storage_id {
                None => storage_id = Some(ch.storage_id),
                Some(id) if id != ch.storage_id => return Ok(None),
                Some(_) => {},
            }
        }
        let Some(storage_id) = storage_id else {
            return Ok(None);
        };
        if check_access(&AccessRepository::new(self.db), user.id, storage_id, &AccessType::A)
            .await
            .is_err()
        {
            return Ok(None);
        }
        match self.storages_repo.get_by_id(storage_id).await {
            Ok(s) => Ok(Some(s)),
            Err(SarcaError::DoesNotExist(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
