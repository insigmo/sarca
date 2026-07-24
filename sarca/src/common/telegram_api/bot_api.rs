use std::{path::Path, pin::Pin, time::Duration, time::Instant};

use futures::{Stream, StreamExt};
use reqwest::multipart;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::io::{AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    common::types::ChatId,
    errors::{SarcaError, SarcaResult},
    services::storage_workers_scheduler::StorageWorkersScheduler,
};

use super::schemas::{
    ChatInfo, CopyMessageBodySchema, DownloadBodySchema, GetChatBodySchema, UploadBodySchema,
    UploadOutcome,
};

const MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF_MS: u64 = 200;

pub struct TelegramBotApi<'t> {
    base_url: &'t str,
    scheduler: StorageWorkersScheduler<'t>,
}

impl<'t> TelegramBotApi<'t> {
    pub fn new(base_url: &'t str, scheduler: StorageWorkersScheduler<'t>) -> Self {
        Self {
            base_url,
            scheduler,
        }
    }

    /// Masks the bot token in URL for safe logging
    fn mask_url(&self, url: &str) -> String {
        if let Some(bot_idx) = url.find("/bot") {
            if let Some(slash_idx) = url[bot_idx + 4..].find('/') {
                return format!(
                    "{}/bot***{}",
                    &url[..bot_idx],
                    &url[bot_idx + 4 + slash_idx..]
                );
            }
        }
        url.to_string()
    }

    /// Retry network errors and HTTP 429/5xx with exponential backoff (3 attempts).
    async fn send_with_retries<F, Fut>(
        op: &str,
        mut send: F,
    ) -> SarcaResult<reqwest::Response>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        let mut last_network_err: Option<reqwest::Error> = None;

        for attempt in 0..MAX_ATTEMPTS {
            match send().await {
                Ok(response) => {
                    let status = response.status();
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if retryable && attempt + 1 < MAX_ATTEMPTS {
                        let body = response.text().await.unwrap_or_default();
                        let backoff = BASE_BACKOFF_MS * 2u64.pow(attempt);
                        tracing::warn!(
                            "[TELEGRAM API] {op} got {status}, retrying in {backoff}ms \
                             (attempt {}/{MAX_ATTEMPTS}): {body}",
                            attempt + 1
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        continue;
                    }
                    return Ok(response);
                }
                Err(e) => {
                    last_network_err = Some(e);
                    if attempt + 1 < MAX_ATTEMPTS {
                        let backoff = BASE_BACKOFF_MS * 2u64.pow(attempt);
                        tracing::warn!(
                            "[TELEGRAM API] {op} network error, retrying in {backoff}ms \
                             (attempt {}/{MAX_ATTEMPTS}): {}",
                            attempt + 1,
                            last_network_err.as_ref().unwrap()
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        continue;
                    }
                }
            }
        }

        Err(last_network_err
            .map(SarcaError::from)
            .unwrap_or(SarcaError::Unknown))
    }

    pub async fn upload(
        &self,
        file: &[u8],
        chat_id: ChatId,
        storage_id: Uuid,
    ) -> SarcaResult<UploadOutcome> {
        if chat_id < 0 && chat_id > -10000000000 {
            tracing::info!(
                "[TELEGRAM API] Using regular group (chat_id={}). If bot can't find the chat, \
                make sure the bot is added and has permissions.",
                chat_id
            );
        }

        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "sendDocument", token);
        let masked_url = self.mask_url(&url);
        let file_len = file.len();

        let start = Instant::now();
        let response = Self::send_with_retries("upload", || {
            let file_part =
                multipart::Part::bytes(file.to_vec()).file_name("sarca_chunk.bin");
            let form = multipart::Form::new()
                .text("chat_id", chat_id.to_string())
                .part("document", file_part);
            reqwest::Client::new().post(&url).multipart(form).send()
        })
        .await?;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({
                    "status": status.as_u16(),
                    "method": "POST",
                    "url": masked_url,
                    "body": {
                        "chat_id": chat_id,
                        "file_size_bytes": file_len,
                        "storage_id": storage_id.to_string()
                    },
                    "response": error_body,
                    "elapsed_ms": elapsed_ms
                })
            );
            return Err(SarcaError::TelegramAPIError(format!(
                "{}: {}",
                status, error_body
            )));
        }

        let result = response.json::<UploadBodySchema>().await.map_err(|e| {
            tracing::error!("[TELEGRAM API] Failed to parse response: {}", e);
            e
        })?;

        tracing::info!(
            target: "http_outbound",
            "{}",
            json!({
                "status": status.as_u16(),
                "method": "POST",
                "url": masked_url,
                "body": {
                    "chat_id": chat_id,
                    "file_size_bytes": file_len,
                    "storage_id": storage_id.to_string()
                },
                "response": {
                    "telegram_file_id": result.result.document.file_id
                },
                "elapsed_ms": elapsed_ms
            })
        );

        Ok(UploadOutcome {
            file_id: result.result.document.file_id,
            message_id: result.result.message_id,
        })
    }

    /// Upload a part of a file from disk without buffering it fully in RAM.
    ///
    /// `offset` and `len` define the slice of the file to upload.
    /// Optional `progress` reports bytes within the whole file (`file_base + sent`).
    pub async fn upload_file_part(
        &self,
        file_path: &Path,
        offset: u64,
        len: u64,
        chat_id: ChatId,
        storage_id: Uuid,
        file_total: u64,
        chunk_no: u32,
        total_chunks: u32,
        progress: Option<tokio::sync::mpsc::Sender<crate::common::channels::UploadProgressEvent>>,
    ) -> SarcaResult<UploadOutcome> {
        use crate::common::channels::UploadProgressEvent;
        use futures::StreamExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "sendDocument", token);
        let masked_url = self.mask_url(&url);

        // Custom retry loop because multipart stream must be rebuilt each attempt.
        let start = Instant::now();
        let mut last_err: Option<SarcaError> = None;
        let mut response_opt = None;

        for attempt in 0..MAX_ATTEMPTS {
            let mut file = match tokio::fs::File::open(file_path).await {
                Ok(f) => f,
                Err(_) => return Err(SarcaError::Unknown),
            };
            if file.seek(SeekFrom::Start(offset)).await.is_err() {
                return Err(SarcaError::Unknown);
            }
            let reader = file.take(len);
            let base_stream = ReaderStream::new(reader);
            let sent = Arc::new(AtomicU64::new(0));
            let last_emit = Arc::new(AtomicU64::new(0));
            let progress_tx = progress.clone();
            let stream = base_stream.map({
                let sent = sent.clone();
                let last_emit = last_emit.clone();
                move |item| {
                    if let Ok(ref bytes) = item {
                        let n = sent.fetch_add(bytes.len() as u64, Ordering::Relaxed) + bytes.len() as u64;
                        let prev = last_emit.load(Ordering::Relaxed);
                        // Emit about every 1 MiB (or on chunk completion).
                        if n == len || n.saturating_sub(prev) >= 1024 * 1024 {
                            last_emit.store(n, Ordering::Relaxed);
                            if let Some(tx) = progress_tx.as_ref() {
                                let _ = tx.try_send(UploadProgressEvent::telegram(
                                    offset.saturating_add(n).min(file_total),
                                    file_total,
                                    chunk_no,
                                    total_chunks,
                                ));
                            }
                        }
                    }
                    item
                }
            });
            let body = reqwest::Body::wrap_stream(stream);
            let part =
                multipart::Part::stream_with_length(body, len).file_name("sarca_chunk.bin");
            let form = multipart::Form::new()
                .text("chat_id", chat_id.to_string())
                .part("document", part);

            match reqwest::Client::new().post(&url).multipart(form).send().await {
                Ok(response) => {
                    let status = response.status();
                    let retryable = status.as_u16() == 429 || status.is_server_error();
                    if retryable && attempt + 1 < MAX_ATTEMPTS {
                        let body = response.text().await.unwrap_or_default();
                        let backoff = BASE_BACKOFF_MS * 2u64.pow(attempt);
                        tracing::warn!(
                            "[TELEGRAM API] upload_file_part got {status}, retrying in {backoff}ms \
                             (attempt {}/{MAX_ATTEMPTS}): {body}",
                            attempt + 1
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        continue;
                    }
                    response_opt = Some(response);
                    break;
                }
                Err(e) => {
                    last_err = Some(e.into());
                    if attempt + 1 < MAX_ATTEMPTS {
                        let backoff = BASE_BACKOFF_MS * 2u64.pow(attempt);
                        tracing::warn!(
                            "[TELEGRAM API] upload_file_part network error, retrying in {backoff}ms \
                             (attempt {}/{MAX_ATTEMPTS})",
                            attempt + 1
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        continue;
                    }
                }
            }
        }

        let response = match response_opt {
            Some(r) => r,
            None => return Err(last_err.unwrap_or(SarcaError::Unknown)),
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({
                    "status": status.as_u16(),
                    "method": "POST",
                    "url": masked_url,
                    "body": {
                        "chat_id": chat_id,
                        "offset": offset,
                        "len": len,
                        "storage_id": storage_id.to_string()
                    },
                    "response": error_body,
                    "elapsed_ms": elapsed_ms
                })
            );
            return Err(SarcaError::TelegramAPIError(format!(
                "{}: {}",
                status, error_body
            )));
        }

        let result = response.json::<UploadBodySchema>().await.map_err(|e| {
            tracing::error!("[TELEGRAM API] Failed to parse response: {}", e);
            e
        })?;

        tracing::info!(
            target: "http_outbound",
            "{}",
            json!({
                "status": status.as_u16(),
                "method": "POST",
                "url": masked_url,
                "body": {
                    "chat_id": chat_id,
                    "offset": offset,
                    "len": len,
                    "storage_id": storage_id.to_string()
                },
                "response": {
                    "telegram_file_id": result.result.document.file_id
                },
                "elapsed_ms": elapsed_ms
            })
        );

        Ok(UploadOutcome {
            file_id: result.result.document.file_id,
            message_id: result.result.message_id,
        })
    }

    /// Local Bot API writes files as owner-only briefly; our entrypoint chmod loop
    /// opens them for Sarca (`nobody`). Retry PermissionDenied / NotFound so downloads
    /// don't fail in that race window.
    async fn open_local_bot_api_file(path: &str) -> SarcaResult<tokio::fs::File> {
        const ATTEMPTS: u32 = 25;
        const DELAY_MS: u64 = 200;

        let mut last_err: Option<std::io::Error> = None;
        for attempt in 1..=ATTEMPTS {
            match tokio::fs::File::open(path).await {
                Ok(file) => return Ok(file),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                    ) =>
                {
                    tracing::warn!(
                        "[TELEGRAM API] local file open attempt {attempt}/{ATTEMPTS} \
                         path={} err={e}",
                        path
                    );
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(DELAY_MS)).await;
                }
                Err(e) => {
                    tracing::error!(
                        "[TELEGRAM API] local file open failed path={} err={e}",
                        path
                    );
                    return Err(SarcaError::TelegramAPIError(format!(
                        "Failed to open local bot api file: {e}"
                    )));
                }
            }
        }

        let e = last_err.expect("at least one permission/not-found error");
        tracing::error!(
            "[TELEGRAM API] local file open failed path={} err={e} after {ATTEMPTS} attempts. \
             Ensure telegram-bot-api-data is mounted and world-readable.",
            path
        );
        Err(SarcaError::TelegramAPIError(format!(
            "Failed to open local bot api file: {e}"
        )))
    }

    async fn read_local_bot_api_file(path: &str) -> SarcaResult<Vec<u8>> {
        let mut file = Self::open_local_bot_api_file(path).await?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await.map_err(|e| {
            SarcaError::TelegramAPIError(format!("Failed to read local bot api file: {e}"))
        })?;
        Ok(bytes)
    }

    pub async fn download(
        &self,
        telegram_file_id: &str,
        storage_id: Uuid,
    ) -> SarcaResult<Vec<u8>> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "getFile", token);
        let masked_url = self.mask_url(&url);

        let start = Instant::now();
        let response = Self::send_with_retries("download/getFile", || {
            reqwest::Client::new()
                .get(&url)
                .query(&[("file_id", telegram_file_id)])
                .send()
        })
        .await?;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({
                    "status": status.as_u16(),
                    "method": "GET",
                    "url": format!("{}?file_id={}", masked_url, telegram_file_id),
                    "body": null,
                    "response": error_body,
                    "elapsed_ms": elapsed_ms
                })
            );
            return Err(SarcaError::TelegramAPIError(format!(
                "{}: {}",
                status, error_body
            )));
        }

        let body: DownloadBodySchema = response.json().await?;

        tracing::info!(
            target: "http_outbound",
            "{}",
            json!({
                "status": status.as_u16(),
                "method": "GET",
                "url": format!("{}?file_id={}", masked_url, telegram_file_id),
                "body": null,
                "response": {
                    "file_path": body.result.file_path,
                    "file_size": body.result.file_size
                },
                "elapsed_ms": elapsed_ms
            })
        );

        // Local Bot API (`--local`) returns an absolute filesystem path. That path
        // lives on the telegram-bot-api data volume (must be mounted into Sarca).
        if body.result.file_path.starts_with('/') {
            if !body
                .result
                .file_path
                .starts_with("/var/lib/telegram-bot-api/")
            {
                return Err(SarcaError::TelegramAPIError(
                    "Unexpected local file_path from telegram-bot-api".to_string(),
                ));
            }

            return Self::read_local_bot_api_file(&body.result.file_path).await;
        }

        // downloading the file itself
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("file/", &body.result.file_path, token);
        let masked_url = self.mask_url(&url);

        let start = Instant::now();
        let response = Self::send_with_retries("download/file", || {
            reqwest::Client::new().get(&url).send()
        })
        .await?;
        let elapsed_ms = start.elapsed().as_millis() as u64;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({
                    "status": status.as_u16(),
                    "method": "GET",
                    "url": masked_url,
                    "body": null,
                    "response": error_body,
                    "elapsed_ms": elapsed_ms
                })
            );
            return Err(SarcaError::TelegramAPIError(format!(
                "{}: {}",
                status, error_body
            )));
        }

        let file = response.bytes().await.map(|file| file.to_vec())?;

        tracing::info!(
            target: "http_outbound",
            "{}",
            json!({
                "status": status.as_u16(),
                "method": "GET",
                "url": masked_url,
                "body": null,
                "response": {
                    "downloaded_bytes": file.len()
                },
                "elapsed_ms": elapsed_ms
            })
        );

        Ok(file)
    }

    /// Download file bytes as a stream (does not buffer whole chunk in RAM).
    pub async fn download_stream(
        &self,
        telegram_file_id: &str,
        storage_id: Uuid,
    ) -> SarcaResult<Pin<Box<dyn Stream<Item = Result<tokio_util::bytes::Bytes, SarcaError>> + Send>>>
    {
        // getting file path
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "getFile", token);

        let body: DownloadBodySchema = reqwest::Client::new()
            .get(url)
            .query(&[("file_id", telegram_file_id)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        // Local Bot API (`--local`) returns an absolute filesystem path.
        if body.result.file_path.starts_with('/') {
            if !body
                .result
                .file_path
                .starts_with("/var/lib/telegram-bot-api/")
            {
                return Err(SarcaError::TelegramAPIError(
                    "Unexpected local file_path from telegram-bot-api".to_string(),
                ));
            }

            let file = Self::open_local_bot_api_file(&body.result.file_path).await?;
            let stream = ReaderStream::new(file).map(|res| {
                res.map_err(|e| {
                    SarcaError::TelegramAPIError(format!(
                        "Failed to read local bot api file: {e}"
                    ))
                })
            });
            return Ok(Box::pin(stream));
        }

        // downloading the file itself
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("file/", &body.result.file_path, token);

        let response = reqwest::Client::new()
            .get(url)
            .send()
            .await?
            .error_for_status()?;

        let stream = response
            .bytes_stream()
            .map(|res| res.map_err(SarcaError::from));

        Ok(Box::pin(stream))
    }

    /// Resolve a chat's display name (title, else username, else first name, else the id).
    pub async fn get_chat(&self, chat_id: ChatId, storage_id: Uuid) -> SarcaResult<ChatInfo> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "getChat", token);
        let masked_url = self.mask_url(&url);

        let response = Self::send_with_retries("getChat", || {
            reqwest::Client::new()
                .get(&url)
                .query(&[("chat_id", chat_id.to_string())])
                .send()
        })
        .await?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({
                    "status": status.as_u16(),
                    "method": "GET",
                    "url": masked_url,
                    "body": { "chat_id": chat_id },
                    "response": error_body,
                })
            );
            return Err(SarcaError::TelegramAPIError(format!(
                "{}: {}",
                status, error_body
            )));
        }

        let body: GetChatBodySchema = response.json().await?;
        let title = body
            .result
            .title
            .or(body.result.username)
            .or(body.result.first_name)
            .unwrap_or_else(|| chat_id.to_string());

        Ok(ChatInfo { title })
    }

    /// Copy a message (with its document) from one chat to another without re-uploading.
    ///
    /// Telegram's `copyMessage` only returns the new `message_id`; the underlying file
    /// stays the same document, so the caller-supplied `source_file_id` remains valid for
    /// download via `getFile` as long as the bot can still reach any chat holding it.
    pub async fn copy_message(
        &self,
        from_chat_id: ChatId,
        message_id: i64,
        to_chat_id: ChatId,
        source_file_id: &str,
        storage_id: Uuid,
    ) -> SarcaResult<UploadOutcome> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "copyMessage", token);
        let masked_url = self.mask_url(&url);

        let response = Self::send_with_retries("copyMessage", || {
            reqwest::Client::new()
                .post(&url)
                .form(&[
                    ("chat_id", to_chat_id.to_string()),
                    ("from_chat_id", from_chat_id.to_string()),
                    ("message_id", message_id.to_string()),
                ])
                .send()
        })
        .await?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            tracing::error!(
                target: "http_outbound",
                "{}",
                json!({
                    "status": status.as_u16(),
                    "method": "POST",
                    "url": masked_url,
                    "body": {
                        "to_chat_id": to_chat_id,
                        "from_chat_id": from_chat_id,
                        "message_id": message_id
                    },
                    "response": error_body,
                })
            );
            return Err(SarcaError::TelegramAPIError(format!(
                "{}: {}",
                status, error_body
            )));
        }

        let body: CopyMessageBodySchema = response.json().await?;

        Ok(UploadOutcome {
            file_id: source_file_id.to_owned(),
            message_id: body.result.message_id,
        })
    }

    /// Best-effort Telegram `deleteMessage`. Missing/already-deleted messages are treated as success.
    pub async fn delete_message(
        &self,
        chat_id: ChatId,
        message_id: i64,
        storage_id: Uuid,
    ) -> SarcaResult<()> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "deleteMessage", token);
        let masked_url = self.mask_url(&url);

        let response = Self::send_with_retries("deleteMessage", || {
            reqwest::Client::new()
                .post(&url)
                .form(&[
                    ("chat_id", chat_id.to_string()),
                    ("message_id", message_id.to_string()),
                ])
                .send()
        })
        .await?;

        let status = response.status();
        if status.is_success() {
            return Ok(());
        }

        let error_body = response.text().await.unwrap_or_default();
        let lower = error_body.to_lowercase();
        if lower.contains("message to delete not found")
            || lower.contains("message can't be deleted")
            || lower.contains("message_id_invalid")
        {
            return Ok(());
        }

        tracing::warn!(
            target: "http_outbound",
            "{}",
            json!({
                "status": status.as_u16(),
                "method": "POST",
                "url": masked_url,
                "body": {
                    "chat_id": chat_id,
                    "message_id": message_id
                },
                "response": error_body,
            })
        );

        Err(SarcaError::TelegramAPIError(format!(
            "{}: {}",
            status, error_body
        )))
    }

    /// Taking token by a value to force dropping it so it can be used only once
    #[inline]
    fn build_url(&self, pre: &str, relative: &str, token: String) -> String {
        format!("{}/{pre}bot{token}/{relative}", self.base_url)
    }
}

/// Whether a Telegram API error indicates the chat is gone / unreachable for the bot
/// (as opposed to a transient network/rate-limit error).
pub fn is_chat_dead_error(err: &SarcaError) -> bool {
    let SarcaError::TelegramAPIError(msg) = err else {
        return false;
    };
    let msg = msg.to_lowercase();
    const DEAD_MARKERS: &[&str] = &[
        "chat not found",
        "bot was kicked",
        "bot is not a member",
        "user is deactivated",
        "have no rights",
        "not enough rights",
        "chat_id is empty",
        "peer_id_invalid",
        "chat_id_invalid",
        "forbidden",
        "group chat was upgraded",
        "member list is inaccessible",
    ];
    DEAD_MARKERS.iter().any(|marker| msg.contains(marker))
}
