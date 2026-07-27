use sqlx::PgPool;

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
        app_settings::AppSettingsRepository,
        storage_channels::StorageChannelsRepository,
        storages::StoragesRepository,
    },
    schemas::{
        setup::{
            BotValidateSchema,
            ChannelPollResultSchema,
            LocalApiCredentialsSchema,
            LocalApiSaveResultSchema,
            LocalApiVerifySchema,
            SetupCreateStorageResultSchema,
            SetupCreateStorageSchema,
            SetupStatusSchema,
        },
        storages::{ChannelInput, InStorageSchema, SetStorageBotSchema},
    },
    services::storages::StoragesService,
};

pub struct SetupService<'d> {
    db: &'d PgPool,
    telegram_base_url: &'d str,
    rate_limit: u8,
    settings: AppSettingsRepository<'d>,
    storages_repo: StoragesRepository<'d>,
}

impl<'d> SetupService<'d> {
    pub fn new(db: &'d PgPool, telegram_base_url: &'d str, rate_limit: u8) -> Self {
        Self {
            db,
            telegram_base_url,
            rate_limit,
            settings: AppSettingsRepository::new(db),
            storages_repo: StoragesRepository::new(db),
        }
    }

    fn uses_local_api(base_url: &str) -> bool {
        !base_url.contains("api.telegram.org")
    }

    pub async fn status(&self, user: &AuthUser) -> SarcaResult<SetupStatusSchema> {
        let storages = self.storages_repo.list_by_user_id(user.id).await?;
        let uses_local_api = Self::uses_local_api(self.telegram_base_url);
        let local_api_skipped = self.settings.is_local_api_skipped().await?;
        let local_api_ready = if uses_local_api {
            self.ping_local_api().await.ok
        } else {
            // Official Bot API is always reachable; Phase A still offered until skipped.
            true
        };
        // Official API: encourage Local API until skipped.
        // Local API mode: show Phase A until reachable or skipped.
        let needs_local_api_phase = if uses_local_api {
            !local_api_skipped && !local_api_ready
        } else {
            !local_api_skipped
        };

        Ok(SetupStatusSchema {
            has_storages: !storages.is_empty(),
            uses_local_api,
            local_api_ready,
            local_api_skipped,
            needs_local_api_phase,
            conf_writable: conf::resolve_conf_path().is_some(),
        })
    }

    pub async fn save_local_api(
        &self,
        body: LocalApiCredentialsSchema,
    ) -> SarcaResult<LocalApiSaveResultSchema> {
        let api_id = body.api_id.trim().to_owned();
        let api_hash = body.api_hash.trim().to_owned();
        if api_id.is_empty() || api_hash.is_empty() {
            return Err(SarcaError::InvalidPath);
        }
        if !api_id.chars().all(|c| c.is_ascii_digit()) {
            return Err(SarcaError::TelegramAPIError(
                "api_id must be a number from my.telegram.org".into(),
            ));
        }

        self.settings.set_telegram_api_credentials(&api_id, &api_hash).await?;
        // Clear skip so Phase A can re-verify.
        self.settings.set_local_api_skipped(false).await?;

        let saved_to_conf = match conf::upsert_conf_keys(&[
            ("TELEGRAM_API_ID", &api_id),
            ("TELEGRAM_API_HASH", &api_hash),
        ]) {
            Ok(true) => true,
            Ok(false) => false,
            Err(e) => {
                tracing::warn!("could not write TELEGRAM_API_* to sarca.conf: {e}");
                false
            },
        };

        let restart_hint = if Self::uses_local_api(self.telegram_base_url) {
            Some(
                "If Local Bot API was already running, restart the telegram-bot-api \
                 container/process so it picks up the new credentials."
                    .to_owned(),
            )
        } else {
            Some(
                "Credentials saved. Set TELEGRAM_LOCAL_API=true (and TELEGRAM_API_BASE_URL) in \
                 sarca.conf, start Local Bot API, then restart Sarca."
                    .to_owned(),
            )
        };

        Ok(LocalApiSaveResultSchema {
            saved_to_settings: true,
            saved_to_conf,
            restart_hint,
        })
    }

    pub async fn skip_local_api(&self) -> SarcaResult<()> {
        self.settings.set_local_api_skipped(true).await
    }

    pub async fn verify_local_api(&self) -> SarcaResult<LocalApiVerifySchema> {
        Ok(self.ping_local_api().await)
    }

    async fn ping_local_api(&self) -> LocalApiVerifySchema {
        let uses_local_api = Self::uses_local_api(self.telegram_base_url);
        if !uses_local_api {
            return LocalApiVerifySchema {
                ok: true,
                uses_local_api: false,
                message: "Using official Telegram Bot API (files limited to ~20 MB).".into(),
            };
        }

        let url = self.telegram_base_url.trim_end_matches('/').to_owned();
        match reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
        {
            Ok(_) => {
                LocalApiVerifySchema {
                    ok: true,
                    uses_local_api: true,
                    message: format!("Reached Local Bot API at {url}"),
                }
            },
            Err(e) => {
                LocalApiVerifySchema {
                    ok: false,
                    uses_local_api: true,
                    message: format!("Cannot reach Local Bot API at {url}: {e}"),
                }
            },
        }
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
        // Reset sticky allowed_updates and open the Local Bot API session before
        // the user adds the bot as admin (my_chat_member is not retroactive).
        if let Err(e) = client.arm_updates().await {
            tracing::warn!("arm getUpdates during setup validate: {e}");
        }
        Ok(BotValidateSchema {
            bot_id: me.id,
            username: me.username,
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
        let (found, hint) = self.discover_admin_chats(token, exclude, probe).await?;
        if let Some((chat_id, title)) = found.into_iter().next() {
            return Ok(ChannelPollResultSchema {
                found: true,
                chat_id: Some(chat_id),
                title: Some(title),
                hint: None,
            });
        }
        Ok(ChannelPollResultSchema {
            found: false,
            chat_id: None,
            title: None,
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
        // Warm Local Bot API / validate token before mutating DB.
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
