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

use super::schemas::{DownloadBodySchema, UploadBodySchema, UploadSchema};

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
    ) -> SarcaResult<UploadSchema> {
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

        Ok(result.result.document)
    }

    /// Upload a part of a file from disk without buffering it fully in RAM.
    ///
    /// `offset` and `len` define the slice of the file to upload.
    pub async fn upload_file_part(
        &self,
        file_path: &Path,
        offset: u64,
        len: u64,
        chat_id: ChatId,
        storage_id: Uuid,
    ) -> SarcaResult<UploadSchema> {
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
            let stream = ReaderStream::new(reader);
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

        Ok(result.result.document)
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

        // Local Bot API returns an absolute filesystem path.
        if body.result.file_path.starts_with('/') {
            // Security: only allow reading from the expected local-bot-api directory.
            if !body
                .result
                .file_path
                .starts_with("/var/lib/telegram-bot-api/")
            {
                return Err(SarcaError::TelegramAPIError(
                    "Unexpected local file_path from telegram-bot-api".to_string(),
                ));
            }

            let bytes = tokio::fs::read(&body.result.file_path).await.map_err(|e| {
                SarcaError::TelegramAPIError(format!(
                    "Failed to read local bot api file: {}",
                    e
                ))
            })?;
            return Ok(bytes);
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

        // Local Bot API returns an absolute filesystem path.
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

            let file = tokio::fs::File::open(&body.result.file_path)
                .await
                .map_err(|e| {
                    SarcaError::TelegramAPIError(format!(
                        "Failed to open local bot api file: {}",
                        e
                    ))
                })?;
            let stream = ReaderStream::new(file).map(|res| {
                res.map_err(|e| {
                    SarcaError::TelegramAPIError(format!(
                        "Failed to read local bot api file: {}",
                        e
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

    /// Taking token by a value to force dropping it so it can be used only once
    #[inline]
    fn build_url(&self, pre: &str, relative: &str, token: String) -> String {
        format!("{}/{pre}bot{token}/{relative}", self.base_url)
    }
}
