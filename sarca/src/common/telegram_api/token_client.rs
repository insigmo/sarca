//! Token-scoped Telegram Bot API client for setup (no storage worker yet).

use serde_json::json;

use super::schemas::{
    BotMe,
    ChatInfo,
    DetectedChat,
    GetChatBodySchema,
    GetChatMemberBodySchema,
    GetMeBodySchema,
    GetUpdatesBodySchema,
    chats_from_updates,
};
use crate::{
    common::types::ChatId,
    errors::{SarcaError, SarcaResult},
};

pub struct TelegramTokenClient {
    base_url: String,
    token: String,
}

impl TelegramTokenClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            token: token.into().trim().to_owned(),
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
        let response = reqwest::Client::new().get(&url).send().await?;
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
        let response = reqwest::Client::new()
            .post(&url)
            .form(&[("drop_pending_updates", "false")])
            .send()
            .await?;
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
        let url = self.build_url("getUpdates");
        let masked = Self::mask_url(&url);
        let response = reqwest::Client::new()
            .get(&url)
            .query(&[("timeout", "0"), ("limit", "100")])
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if !status.is_success() {
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({ "status": status.as_u16(), "method": "GET", "url": masked, "response": text })
            );
            return Err(SarcaError::TelegramAPIError(format!(
                "getUpdates failed ({status}): {text}"
            )));
        }
        let body: GetUpdatesBodySchema = serde_json::from_str(&text)
            .map_err(|e| SarcaError::TelegramAPIError(format!("getUpdates parse error: {e}")))?;
        Ok(chats_from_updates(&body))
    }

    pub async fn get_chat(&self, chat_id: ChatId) -> SarcaResult<ChatInfo> {
        let url = self.build_url("getChat");
        let masked = Self::mask_url(&url);
        let response = reqwest::Client::new()
            .get(&url)
            .query(&[("chat_id", chat_id.to_string())])
            .send()
            .await?;
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
        let response = reqwest::Client::new()
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
}
