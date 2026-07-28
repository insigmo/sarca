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
    pub update_id: i64,
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
    /// Legacy forward (Bot API < 7.0 style).
    #[serde(default)]
    pub forward_from_chat: Option<UpdateChatSchema>,
    /// Modern forward origin (channel posts forwarded to the bot DM).
    #[serde(default)]
    pub forward_origin: Option<ForwardOriginSchema>,
}

#[derive(Debug, Deserialize)]
pub struct ForwardOriginSchema {
    #[serde(default)]
    #[serde(rename = "type")]
    pub origin_type: Option<String>,
    #[serde(default)]
    pub chat: Option<UpdateChatSchema>,
}

#[derive(Debug, Deserialize)]
pub struct ChatMemberUpdateSchema {
    pub chat: UpdateChatSchema,
    /// Present on real Bot API payloads; used to ignore kick/left noise.
    #[serde(default)]
    pub new_chat_member: Option<ChatMemberStatusSchema>,
}

#[derive(Debug, Deserialize)]
pub struct ChatMemberStatusSchema {
    #[serde(default)]
    pub status: Option<String>,
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
            // Only promote-to-admin is useful. Kick/left/member events used to
            // become candidates, then burn a getChatMember 403 and (worse) keep
            // the long-poll from waiting on a fresh admin-add in the same tick.
            match member.new_chat_member.as_ref().and_then(|m| m.status.as_deref()) {
                Some("administrator" | "creator") => {
                    push_detected(&mut out, &member.chat);
                },
                // Malformed / stripped payload — keep prior behavior (classify later).
                None => push_detected(&mut out, &member.chat),
                Some(_) => {},
            }
        }
        if let Some(msg) = &update.message {
            // Groups/supergroups when bot is added.
            let t = msg.chat.chat_type.as_deref().unwrap_or("");
            if t == "group" || t == "supergroup" || t == "channel" {
                push_detected(&mut out, &msg.chat);
            }
            // Private channel already admin: user forwards any post to the bot DM.
            if let Some(fwd) = &msg.forward_from_chat {
                if fwd.chat_type.as_deref() == Some("channel") || fwd.id < 0 {
                    push_detected(&mut out, fwd);
                }
            }
            if let Some(origin) = &msg.forward_origin {
                let is_channel = origin.origin_type.as_deref() == Some("channel");
                if let Some(chat) = &origin.chat {
                    if is_channel || chat.chat_type.as_deref() == Some("channel") || chat.id < 0 {
                        push_detected(&mut out, chat);
                    }
                }
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

    #[test]
    fn chats_from_updates_reads_my_chat_member() {
        let json = r#"{
          "result": [{
            "update_id": 3,
            "my_chat_member": {
              "chat": { "id": -100111, "title": "Admin Chan", "type": "channel" },
              "from": { "id": 1, "is_bot": false, "first_name": "U" },
              "date": 1,
              "old_chat_member": { "status": "left", "user": { "id": 2, "is_bot": true, "first_name": "B" } },
              "new_chat_member": { "status": "administrator", "user": { "id": 2, "is_bot": true, "first_name": "B" } }
            }
          }]
        }"#;
        let body: GetUpdatesBodySchema = serde_json::from_str(json).unwrap();
        let chats = chats_from_updates(&body);
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].chat_id, -100_111);
        assert_eq!(chats[0].title, "Admin Chan");
    }

    #[test]
    fn chats_from_updates_skips_my_chat_member_kick() {
        let json = r#"{
          "result": [{
            "update_id": 3,
            "my_chat_member": {
              "chat": { "id": -100111, "title": "Gone Chan", "type": "channel" },
              "from": { "id": 1, "is_bot": false, "first_name": "U" },
              "date": 1,
              "old_chat_member": { "status": "administrator", "user": { "id": 2, "is_bot": true, "first_name": "B" } },
              "new_chat_member": { "status": "kicked", "user": { "id": 2, "is_bot": true, "first_name": "B" } }
            }
          }]
        }"#;
        let body: GetUpdatesBodySchema = serde_json::from_str(json).unwrap();
        assert!(chats_from_updates(&body).is_empty());
    }

    #[test]
    fn chats_from_updates_reads_forward_from_chat_in_dm() {
        let json = r#"{
          "result": [{
            "update_id": 4,
            "message": {
              "message_id": 1,
              "chat": { "id": 42, "first_name": "User", "type": "private" },
              "date": 1,
              "forward_from_chat": {
                "id": -1004478634219,
                "title": "SarcaStorage1",
                "type": "channel"
              },
              "text": "hello"
            }
          }]
        }"#;
        let body: GetUpdatesBodySchema = serde_json::from_str(json).unwrap();
        let chats = chats_from_updates(&body);
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].chat_id, -1_004_478_634_219);
        assert_eq!(chats[0].title, "SarcaStorage1");
    }

    #[test]
    fn chats_from_updates_reads_forward_origin_channel() {
        let json = r#"{
          "result": [{
            "update_id": 5,
            "message": {
              "message_id": 2,
              "chat": { "id": 99, "type": "private" },
              "date": 1,
              "forward_origin": {
                "type": "channel",
                "chat": {
                  "id": -1004385550541,
                  "title": "SarcaStorage2",
                  "type": "channel"
                },
                "message_id": 10,
                "date": 1
              }
            }
          }]
        }"#;
        let body: GetUpdatesBodySchema = serde_json::from_str(json).unwrap();
        let chats = chats_from_updates(&body);
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].chat_id, -1_004_385_550_541);
        assert_eq!(chats[0].title, "SarcaStorage2");
    }

    #[test]
    fn chats_from_updates_reads_multiple_channels_in_one_payload() {
        let json = r#"{
          "result": [
            {
              "update_id": 10,
              "my_chat_member": {
                "chat": { "id": -1001, "title": "A", "type": "channel" },
                "from": { "id": 1, "is_bot": false, "first_name": "U" },
                "date": 1,
                "old_chat_member": { "status": "left", "user": { "id": 2, "is_bot": true, "first_name": "B" } },
                "new_chat_member": { "status": "administrator", "user": { "id": 2, "is_bot": true, "first_name": "B" } }
              }
            },
            {
              "update_id": 11,
              "channel_post": {
                "message_id": 1,
                "chat": { "id": -1002, "title": "B", "type": "channel" },
                "date": 1,
                "text": "hi"
              }
            },
            {
              "update_id": 12,
              "message": {
                "message_id": 2,
                "chat": { "id": 42, "type": "private" },
                "date": 1,
                "forward_from_chat": {
                  "id": -1003,
                  "title": "C",
                  "type": "channel"
                }
              }
            }
          ]
        }"#;
        let body: GetUpdatesBodySchema = serde_json::from_str(json).unwrap();
        let chats = chats_from_updates(&body);
        assert_eq!(chats.len(), 3);
        assert_eq!(chats[0].chat_id, -1001);
        assert_eq!(chats[1].chat_id, -1002);
        assert_eq!(chats[2].chat_id, -1003);
    }
}
