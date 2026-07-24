use serde::Deserialize;

use crate::common::types::ChatId;

#[derive(Deserialize)]
pub struct UploadBodySchema {
    pub result: UploadResultSchema,
}

#[derive(Deserialize)]
pub struct UploadResultSchema {
    pub message_id: i64,
    pub document: UploadSchema,
}

#[derive(Deserialize)]
pub struct UploadSchema {
    pub file_id: String,
}

/// Result of a successful upload/copy: the Telegram file id plus the message id
/// that holds it in the target chat (needed later for `copyMessage`).
#[derive(Debug, Clone)]
pub struct UploadOutcome {
    pub file_id: String,
    pub message_id: i64,
}

#[derive(Deserialize)]
pub struct DownloadBodySchema {
    pub result: DownloadSchema,
}

#[derive(Deserialize)]
pub struct DownloadSchema {
    pub file_path: String,
    pub file_size: Option<u64>,
}

#[derive(Deserialize)]
pub struct GetChatBodySchema {
    pub result: GetChatResultSchema,
}

#[derive(Deserialize)]
pub struct GetChatResultSchema {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
}

#[derive(Deserialize)]
pub struct GetChatMemberBodySchema {
    pub result: ChatMemberResultSchema,
}

#[derive(Deserialize)]
pub struct ChatMemberResultSchema {
    pub status: String,
}

/// Minimal chat info resolved via `getChat`, used to auto-fill a channel's display name.
#[derive(Debug, Clone)]
pub struct ChatInfo {
    pub title: String,
}

#[derive(Deserialize)]
pub struct CopyMessageBodySchema {
    pub result: CopyMessageResultSchema,
}

#[derive(Deserialize)]
pub struct CopyMessageResultSchema {
    pub message_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetMeBodySchema {
    pub result: GetMeResultSchema,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GetMeResultSchema {
    pub id: i64,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BotMe {
    pub id: i64,
    pub username: String,
}

/// A Telegram chat discovered during setup (channel detect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedChat {
    pub chat_id: ChatId,
    pub title: String,
}

#[derive(Debug, Deserialize)]
pub struct GetUpdatesBodySchema {
    #[serde(default)]
    pub result: Vec<UpdateSchema>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSchema {
    #[serde(default)]
    pub channel_post: Option<MessageChatSchema>,
    #[serde(default)]
    pub my_chat_member: Option<ChatMemberUpdateSchema>,
    #[serde(default)]
    pub message: Option<MessageChatSchema>,
}

#[derive(Debug, Deserialize)]
pub struct MessageChatSchema {
    pub chat: UpdateChatSchema,
}

#[derive(Debug, Deserialize)]
pub struct ChatMemberUpdateSchema {
    pub chat: UpdateChatSchema,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChatSchema {
    pub id: ChatId,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    pub chat_type: Option<String>,
}

/// Extract distinct chats from a `getUpdates` JSON payload (for setup channel detect).
pub fn chats_from_updates(body: &GetUpdatesBodySchema) -> Vec<DetectedChat> {
    let mut out = Vec::new();
    for update in &body.result {
        if let Some(post) = &update.channel_post {
            push_detected(&mut out, &post.chat);
        }
        if let Some(member) = &update.my_chat_member {
            push_detected(&mut out, &member.chat);
        }
        if let Some(msg) = &update.message {
            // Groups/supergroups when bot is added; skip private DMs.
            let t = msg.chat.chat_type.as_deref().unwrap_or("");
            if t == "group" || t == "supergroup" || t == "channel" {
                push_detected(&mut out, &msg.chat);
            }
        }
    }
    out
}

fn push_detected(out: &mut Vec<DetectedChat>, chat: &UpdateChatSchema) {
    if out.iter().any(|c| c.chat_id == chat.id) {
        return;
    }
    let title = chat
        .title
        .clone()
        .or_else(|| chat.username.clone())
        .or_else(|| chat.first_name.clone())
        .unwrap_or_else(|| chat.id.to_string());
    out.push(DetectedChat {
        chat_id: chat.id,
        title,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chats_from_updates_reads_channel_post() {
        let json = r#"{
          "ok": true,
          "result": [{
            "update_id": 1,
            "channel_post": {
              "message_id": 1,
              "chat": { "id": -1001234567890, "title": "Sarca Data", "type": "channel" },
              "date": 1,
              "text": "hi"
            }
          }]
        }"#;
        let body: GetUpdatesBodySchema = serde_json::from_str(json).unwrap();
        let chats = chats_from_updates(&body);
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].chat_id, -1_001_234_567_890);
        assert_eq!(chats[0].title, "Sarca Data");
    }

    #[test]
    fn chats_from_updates_skips_private_messages() {
        let json = r#"{
          "result": [{
            "update_id": 2,
            "message": {
              "message_id": 1,
              "chat": { "id": 42, "first_name": "User", "type": "private" },
              "date": 1,
              "text": "hi"
            }
          }]
        }"#;
        let body: GetUpdatesBodySchema = serde_json::from_str(json).unwrap();
        assert!(chats_from_updates(&body).is_empty());
    }
}
