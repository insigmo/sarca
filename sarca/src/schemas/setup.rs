use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::common::types::ChatId;

#[derive(Debug, Serialize)]
pub struct SetupStatusSchema {
    pub has_storages: bool,
    pub conf_writable: bool,
}

#[derive(Debug, Deserialize)]
pub struct BotTokenSchema {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct BotValidateSchema {
    pub bot_id: i64,
    pub username: String,
    /// Free admin channels already visible in pending Telegram updates (capped at 3).
    /// Seeded into the wizard so the user does not wait for a second poll.
    #[serde(default)]
    pub channels: Vec<ChannelPollHitSchema>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelPollSchema {
    pub token: String,
    #[serde(default)]
    pub exclude_chat_ids: Vec<ChatId>,
    /// Optional chat ids to verify directly (when `my_chat_member` was missed).
    #[serde(default)]
    pub probe_chat_ids: Vec<ChatId>,
}

#[derive(Debug, Serialize)]
pub struct ChannelPollHitSchema {
    pub chat_id: ChatId,
    pub title: String,
}

#[derive(Debug, Serialize)]
pub struct ChannelPollResultSchema {
    /// All newly discovered admin chats in this poll (already excluding known ids).
    /// Capped at 3 so the wizard can add a full storage in one response.
    pub channels: Vec<ChannelPollHitSchema>,
    /// Present when a chat was seen but the bot is not an admin (or similar).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetupCreateStorageSchema {
    pub name: String,
    pub token: String,
    pub chat_ids: Vec<ChatId>,
}

#[derive(Debug, Serialize)]
pub struct SetupCreateStorageResultSchema {
    pub id: Uuid,
    pub name: String,
}
