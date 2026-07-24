use sqlx::PgPool;

use crate::{
    common::{
        jwt_manager::AuthUser,
        telegram_api::token_client::TelegramTokenClient,
        types::ChatId,
    },
    conf,
    errors::{SarcaError, SarcaResult},
    repositories::{app_settings::AppSettingsRepository, storages::StoragesRepository},
    schemas::{
        setup::{
            BotValidateSchema, ChannelPollResultSchema, LocalApiCredentialsSchema,
            LocalApiSaveResultSchema, LocalApiVerifySchema, SetupCreateStorageResultSchema,
            SetupCreateStorageSchema, SetupStatusSchema,
        },
        storage_workers::InStorageWorkerSchema,
        storages::{ChannelInput, InStorageSchema},
    },
    services::{storage_workers::StorageWorkersService, storages::StoragesService},
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

        self.settings
            .set_telegram_api_credentials(&api_id, &api_hash)
            .await?;
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
            }
        };

        let restart_hint = if Self::uses_local_api(self.telegram_base_url) {
            Some(
                "If Local Bot API was already running, restart the telegram-bot-api \
                 container/process so it picks up the new credentials."
                    .to_owned(),
            )
        } else {
            Some(
                "Credentials saved. Set TELEGRAM_LOCAL_API=true (and TELEGRAM_API_BASE_URL) \
                 in sarca.conf, start Local Bot API, then restart Sarca."
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
            Ok(_) => LocalApiVerifySchema {
                ok: true,
                uses_local_api: true,
                message: format!("Reached Local Bot API at {url}"),
            },
            Err(e) => LocalApiVerifySchema {
                ok: false,
                uses_local_api: true,
                message: format!("Cannot reach Local Bot API at {url}: {e}"),
            },
        }
    }

    pub async fn validate_bot(&self, token: &str) -> SarcaResult<BotValidateSchema> {
        let token = token.trim();
        if token.is_empty() || !token.contains(':') {
            return Err(SarcaError::TelegramAPIError(
                "Bot token looks invalid".into(),
            ));
        }
        let client = TelegramTokenClient::new(self.telegram_base_url, token);
        let me = client.get_me().await?;
        // Ensure getUpdates works during channel detect.
        if let Err(e) = client.delete_webhook().await {
            tracing::warn!("deleteWebhook during setup validate: {e}");
        }
        Ok(BotValidateSchema {
            bot_id: me.id,
            username: me.username,
        })
    }

    pub async fn poll_channel(
        &self,
        token: &str,
        exclude: &[ChatId],
    ) -> SarcaResult<ChannelPollResultSchema> {
        let client = TelegramTokenClient::new(self.telegram_base_url, token.trim());
        let chats = client.get_updates().await?;
        for chat in chats {
            if exclude.contains(&chat.chat_id) {
                continue;
            }
            // Prefer channel / negative ids (Telegram chats for storage).
            if chat.chat_id >= 0 {
                continue;
            }
            let title = match client.get_chat(chat.chat_id).await {
                Ok(info) => info.title,
                Err(_) => chat.title,
            };
            return Ok(ChannelPollResultSchema {
                found: true,
                chat_id: Some(chat.chat_id),
                title: Some(title),
            });
        }
        Ok(ChannelPollResultSchema {
            found: false,
            chat_id: None,
            title: None,
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

        let client = TelegramTokenClient::new(self.telegram_base_url, body.token.trim());
        let me = client.get_me().await?;

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

        let storages = StoragesService::new(self.db, self.telegram_base_url, self.rate_limit);
        let storage = storages
            .create(
                InStorageSchema {
                    name: name.clone(),
                    channels,
                },
                user,
            )
            .await?;

        let workers = StorageWorkersService::new(self.db);
        let worker_name = me.username;
        if let Err(e) = workers
            .create(
                InStorageWorkerSchema {
                    name: worker_name,
                    token: body.token.trim().to_owned(),
                    storage_id: Some(storage.id),
                },
                user,
            )
            .await
        {
            tracing::error!(
                "setup: storage {} created but worker failed: {e:?}; rolling back storage",
                storage.id
            );
            let _ = storages.delete(storage.id, user).await;
            return Err(e);
        }

        Ok(SetupCreateStorageResultSchema {
            id: storage.id,
            name: storage.name,
        })
    }
}
