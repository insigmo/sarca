//! Token-scoped Telegram Bot API client for setup (no storage worker yet).

use serde_json::json;

use super::{
    http_client,
    schemas::{
        BotMe,
        ChatInfo,
        DetectedChat,
        GetChatBodySchema,
        GetChatMemberBodySchema,
        GetMeBodySchema,
        GetUpdatesBodySchema,
        chats_from_updates,
    },
};
use crate::{
    common::types::ChatId,
    errors::{SarcaError, SarcaResult},
};

pub struct TelegramTokenClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl TelegramTokenClient {
    /// `allowed_updates` must be set explicitly — Telegram remembers the last filter;
    /// a sticky restrictive list silently drops `my_chat_member`.
    const ALLOWED_UPDATES: &'static str = r#"["message","edited_message","channel_post","edited_channel_post","my_chat_member","chat_member"]"#;
    /// Non-long-poll calls (getMe, deleteWebhook, getChat, getChatMember) must never
    /// hang forever: if the host can't reach Telegram (blocked/unroutable), a client
    /// with no total-request timeout waits indefinitely — the setup wizard's
    /// "Validate bot" then stays stuck disabled with no error.
    const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            token: token.into().trim().to_owned(),
            // Shared pooled client (also honors TELEGRAM_PROXY_URL); keeps this
            // client's own bounded timeout rather than the unbounded default.
            client: http_client::client_with_timeout(Self::DEFAULT_TIMEOUT),
        }
    }

    fn mask_url(url: &str) -> String {
        if let Some(bot_idx) = url.find("/bot") {
            if let Some(slash_idx) = url[bot_idx + 4..].find('/') {
                return format!("{}/bot***{}", &url[..bot_idx], &url[bot_idx + 4 + slash_idx..]);
            }
        }
        url.to_string()
    }

    fn build_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.base_url, self.token, method)
    }

    pub async fn get_me(&self) -> SarcaResult<BotMe> {
        let url = self.build_url("getMe");
        let masked = Self::mask_url(&url);
        let response = self.client.get(&url).send().await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({ "status": status.as_u16(), "method": "GET", "url": masked, "response": text })
            );
            return Err(SarcaError::TelegramAPIError(format!("Invalid bot token ({status})")));
        }
        let body: GetMeBodySchema = serde_json::from_str(&text)
            .map_err(|e| SarcaError::TelegramAPIError(format!("getMe parse error: {e}")))?;
        let username = body
            .result
            .username
            .filter(|u| !u.is_empty())
            .or(body.result.first_name)
            .unwrap_or_else(|| format!("bot_{}", body.result.id));
        Ok(BotMe {
            id: body.result.id,
            username,
        })
    }

    pub async fn delete_webhook(&self) -> SarcaResult<()> {
        let url = self.build_url("deleteWebhook");
        let masked = Self::mask_url(&url);
        let response =
            self.client.post(&url).form(&[("drop_pending_updates", "false")]).send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            tracing::warn!(
                target: "http_outbound",
                "{}",
                json!({ "status": status.as_u16(), "method": "POST", "url": masked, "response": text })
            );
            return Err(SarcaError::TelegramAPIError(format!("deleteWebhook failed ({status})")));
        }
        Ok(())
    }

    pub async fn get_updates(&self) -> SarcaResult<Vec<DetectedChat>> {
        // Pull the whole pending backlog first so a bot that is already admin in
        // several channels surfaces all of them in one setup response.
        let drained = self.drain_update_chats().await?;
        if !drained.is_empty() {
            return Ok(drained);
        }
        // Nothing pending — short long-poll, then drain any burst that arrived together.
        let (first, next) = self.get_updates_page(None, 2).await?;
        if first.is_empty() {
            return Ok(first);
        }
        let mut all = first;
        let mut seen: std::collections::HashSet<ChatId> = all.iter().map(|c| c.chat_id).collect();
        let mut offset = next;
        while let Some(off) = offset {
            let (more, next_off) = self.get_updates_page(Some(off), 0).await?;
            for chat in more {
                if seen.insert(chat.chat_id) {
                    all.push(chat);
                }
            }
            offset = next_off;
        }
        Ok(all)
    }

    /// Confirm and pull every pending update, collecting distinct chats.
    async fn drain_update_chats(&self) -> SarcaResult<Vec<DetectedChat>> {
        let mut all = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut offset: Option<i64> = None;
        // Hard cap: Telegram returns ≤100 per call; 20 pages is already generous.
        for _ in 0..20 {
            let (chats, next_offset) = self.get_updates_page(offset, 0).await?;
            for chat in chats {
                if seen.insert(chat.chat_id) {
                    all.push(chat);
                }
            }
            match next_offset {
                Some(next) => offset = Some(next),
                None => break,
            }
        }
        Ok(all)
    }

    /// One `getUpdates` page. Returns chats plus `Some(max_update_id + 1)` when the
    /// page was non-empty (caller must pass that as the next offset to confirm).
    async fn get_updates_page(
        &self,
        offset: Option<i64>,
        timeout_secs: u64,
    ) -> SarcaResult<(Vec<DetectedChat>, Option<i64>)> {
        let url = self.build_url("getUpdates");
        let masked = Self::mask_url(&url);
        // Retry once on 409 — Local Bot API / Telegram reject concurrent getUpdates
        // for the same bot ("only one bot instance").
        let mut attempt = 0u8;
        let req_timeout = std::time::Duration::from_secs(timeout_secs.saturating_add(8));
        loop {
            attempt += 1;
            let mut query: Vec<(&str, String)> = vec![
                ("timeout", timeout_secs.to_string()),
                ("limit", "100".to_string()),
                ("allowed_updates", Self::ALLOWED_UPDATES.to_string()),
            ];
            if let Some(off) = offset {
                query.push(("offset", off.to_string()));
            }
            let response = self.client.get(&url).query(&query).timeout(req_timeout).send().await?;
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            if status.as_u16() == 409 && attempt < 2 {
                tracing::warn!(
                    target: "http_outbound",
                    "{}",
                    json!({ "status": 409, "method": "GET", "url": masked, "retry": true })
                );
                tokio::time::sleep(std::time::Duration::from_millis(750)).await;
                continue;
            }
            if !status.is_success() {
                tracing::error!(
                    target: "http_outbound",
                    "{}",
                    json!({ "status": status.as_u16(), "method": "GET", "url": masked, "response": text })
                );
                let hint = if status.as_u16() == 409 {
                    " Another program (or a second Sarca / Bot API client) is already \
                     polling this bot with getUpdates — stop it and try again."
                } else {
                    ""
                };
                return Err(SarcaError::TelegramAPIError(format!(
                    "getUpdates failed ({status}): {text}{hint}"
                )));
            }
            let body: GetUpdatesBodySchema = serde_json::from_str(&text).map_err(|e| {
                SarcaError::TelegramAPIError(format!("getUpdates parse error: {e}"))
            })?;
            if body.result.is_empty() {
                return Ok((Vec::new(), None));
            }
            let next_offset = body.result.iter().map(|u| u.update_id).max().map(|id| id + 1);
            return Ok((chats_from_updates(&body), next_offset));
        }
    }

    pub async fn get_chat(&self, chat_id: ChatId) -> SarcaResult<ChatInfo> {
        let url = self.build_url("getChat");
        let masked = Self::mask_url(&url);
        let response =
            self.client.get(&url).query(&[("chat_id", chat_id.to_string())]).send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({ "status": status.as_u16(), "method": "GET", "url": masked, "body": { "chat_id": chat_id }, "response": text })
            );
            return Err(SarcaError::TelegramAPIError(format!("getChat failed ({status}): {text}")));
        }
        let body: GetChatBodySchema = response.json().await?;
        let title = body
            .result
            .title
            .or(body.result.username)
            .or(body.result.first_name)
            .unwrap_or_else(|| chat_id.to_string());
        Ok(ChatInfo {
            title,
        })
    }

    /// Returns Telegram member status (`creator`, `administrator`, `member`, …).
    pub async fn get_chat_member_status(
        &self,
        chat_id: ChatId,
        user_id: i64,
    ) -> SarcaResult<String> {
        let url = self.build_url("getChatMember");
        let masked = Self::mask_url(&url);
        let response = self
            .client
            .get(&url)
            .query(&[("chat_id", chat_id.to_string()), ("user_id", user_id.to_string())])
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({ "status": status.as_u16(), "method": "GET", "url": masked, "body": { "chat_id": chat_id, "user_id": user_id }, "response": text })
            );
            return Err(SarcaError::TelegramAPIError(format!(
                "getChatMember failed ({status}): {text}"
            )));
        }
        let body: GetChatMemberBodySchema = serde_json::from_str(&text)
            .map_err(|e| SarcaError::TelegramAPIError(format!("getChatMember parse error: {e}")))?;
        Ok(body.result.status)
    }

    /// Chats where this bot is admin/creator (negative chat ids only).
    /// `exclude` skips already-known chat ids. `probe` checks explicit ids first
    /// (recovery when `my_chat_member` was missed). Returns `(found, hint)` where
    /// hint explains non-admin sightings when nothing was found.
    pub async fn discover_admin_chats(
        &self,
        exclude: &[ChatId],
        probe: &[ChatId],
    ) -> SarcaResult<(Vec<(ChatId, String)>, Option<String>)> {
        let me = self.get_me().await?;
        let mut saw_non_admin = false;
        let mut found = Vec::new();
        let mut seen = std::collections::HashSet::new();

        let mut skipped_excluded: Vec<String> = Vec::new();

        for &chat_id in probe {
            if chat_id >= 0 || !seen.insert(chat_id) {
                continue;
            }
            if exclude.contains(&chat_id) {
                skipped_excluded.push(chat_id.to_string());
                continue;
            }
            match self.classify_admin_chat(chat_id, me.id, None).await {
                AdminClass::Admin(title) => found.push((chat_id, title)),
                AdminClass::NotAdmin => {
                    return Ok((
                        Vec::new(),
                        Some(format!(
                            "Bot is in chat {chat_id} but is not an admin. Give it admin rights \
                             with Post messages and Delete messages."
                        )),
                    ));
                },
                AdminClass::Unknown => {
                    return Ok((
                        Vec::new(),
                        Some(format!(
                            "Cannot access chat {chat_id}. Add the bot as an admin there, or check \
                             the id."
                        )),
                    ));
                },
            }
        }

        // If probes already confirmed chats, only scoop the pending backlog (no wait).
        // Otherwise long-poll so the wizard can catch a live admin-add.
        let chats = if found.is_empty() {
            self.get_updates().await?
        } else {
            self.drain_update_chats().await?
        };
        for chat in chats {
            if !seen.insert(chat.chat_id) {
                continue;
            }
            if exclude.contains(&chat.chat_id) {
                skipped_excluded.push(chat.title.clone());
                continue;
            }
            if chat.chat_id >= 0 {
                continue;
            }

            match self.classify_admin_chat(chat.chat_id, me.id, Some(chat.title.clone())).await {
                AdminClass::Admin(title) => found.push((chat.chat_id, title)),
                AdminClass::NotAdmin | AdminClass::Unknown => saw_non_admin = true,
            }
        }

        let hint = if !found.is_empty() {
            None
        } else if !skipped_excluded.is_empty() {
            let labels = skipped_excluded.join(", ");
            Some(format!(
                "Seen channel(s) already linked to another storage: {labels}. Use a different \
                 channel, or open the existing storage instead of creating a new one."
            ))
        } else if saw_non_admin {
            Some(
                "Bot was added to a channel but does not have admin rights. Make it an admin with \
                 Post messages and Delete messages."
                    .to_owned(),
            )
        } else {
            None
        };
        Ok((found, hint))
    }

    async fn classify_admin_chat(
        &self,
        chat_id: ChatId,
        bot_id: i64,
        fallback_title: Option<String>,
    ) -> AdminClass {
        let title = match self.get_chat(chat_id).await {
            Ok(info) => info.title,
            Err(_) => fallback_title.unwrap_or_else(|| chat_id.to_string()),
        };
        match self.get_chat_member_status(chat_id, bot_id).await {
            Ok(status) if status == "administrator" || status == "creator" => {
                AdminClass::Admin(title)
            },
            Ok(_) => AdminClass::NotAdmin,
            Err(e) => {
                tracing::warn!("discover: getChatMember for {chat_id} failed: {e}");
                AdminClass::Unknown
            },
        }
    }
}

enum AdminClass {
    Admin(String),
    NotAdmin,
    Unknown,
}
