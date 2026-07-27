use std::{
    collections::HashMap,
    path::Path,
    pin::Pin,
    sync::{Arc, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures::{Stream, StreamExt};
use reqwest::multipart;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
    sync::{Mutex, OwnedSemaphorePermit, Semaphore},
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use super::schemas::{
    ChatInfo,
    CopyMessageBodySchema,
    DownloadBodySchema,
    GetChatBodySchema,
    UploadBodySchema,
    UploadOutcome,
};
use crate::{
    common::{
        channels::{UploadProgressEvent, emit_upload_progress},
        types::ChatId,
    },
    errors::{SarcaError, SarcaResult},
    services::storage_workers_scheduler::StorageWorkersScheduler,
};

/// Network / 5xx retries. Flood waits retry indefinitely (honor `retry_after`).
const MAX_ATTEMPTS: u32 = 5;
const BASE_BACKOFF_MS: u64 = 200;
/// Honor Telegram's `retry_after` up to this per wait (don't truncate short).
const MAX_FLOOD_WAIT_SECS: u64 = 900;
/// Soft pace between successful sends (~0.45 msg/s). Telegram FAQ is ~1/s; stay
/// conservative so Local Bot API / multi-chunk uploads don't trip flood control.
const MIN_SEND_GAP: Duration = Duration::from_millis(2200);
/// Elevated inter-send gap while a token is in a recent flood window.
const MIN_SEND_GAP_AFTER_FLOOD: Duration = Duration::from_secs(3);
/// How long after a flood wait we keep the elevated send gap.
const FLOOD_PACING_WINDOW: Duration = Duration::from_mins(5);
/// Extra cooldown after honoring `retry_after`, before the next attempt.
const POST_FLOOD_EXTRA_COOLDOWN: Duration = Duration::from_secs(7);

struct TokenSendGate {
    sem: Arc<Semaphore>,
    last_ok: Mutex<Option<Instant>>,
    /// While `Instant::now() < *flood_cooldown_until`, use slower send pacing.
    flood_cooldown_until: Mutex<Option<Instant>>,
}

/// Holds the per-token send lock for the duration of one mutating Telegram API call
/// (`sendDocument`, `copyMessage`, `deleteMessage`, including flood-wait sleeps), so
/// concurrent uploads / replication / purge cannot storm the same bot.
struct SendPermit {
    _permit: OwnedSemaphorePermit,
    gate: Arc<TokenSendGate>,
}

impl SendPermit {
    async fn acquire(token: &str) -> Self {
        let gate = {
            let mut map = send_gates().lock().await;
            map.entry(token.to_owned())
                .or_insert_with(|| {
                    Arc::new(TokenSendGate {
                        sem: Arc::new(Semaphore::new(1)),
                        last_ok: Mutex::new(None),
                        flood_cooldown_until: Mutex::new(None),
                    })
                })
                .clone()
        };
        let permit =
            gate.sem.clone().acquire_owned().await.expect("Telegram send semaphore closed");
        let sleep_for = {
            let last = gate.last_ok.lock().await;
            let flood_until = *gate.flood_cooldown_until.lock().await;
            let gap = if flood_until.is_some_and(|t| Instant::now() < t) {
                MIN_SEND_GAP_AFTER_FLOOD
            } else {
                MIN_SEND_GAP
            };
            last.and_then(|t| gap.checked_sub(t.elapsed())).unwrap_or(Duration::ZERO)
        };
        if !sleep_for.is_zero() {
            tokio::time::sleep(sleep_for).await;
        }
        Self {
            _permit: permit,
            gate,
        }
    }

    async fn mark_ok(&self) {
        *self.gate.last_ok.lock().await = Some(Instant::now());
    }

    /// Record a flood wait so subsequent acquires use a longer send gap.
    async fn note_flood(&self) {
        *self.gate.flood_cooldown_until.lock().await = Some(Instant::now() + FLOOD_PACING_WINDOW);
    }
}

fn send_gates() -> &'static Mutex<HashMap<String, Arc<TokenSendGate>>> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<TokenSendGate>>>> = OnceLock::new();
    GATES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Parameters for `TelegramBotApi::upload_file_part`.
pub struct UploadFilePartRequest {
    pub offset: u64,
    pub len: u64,
    pub chat_id: ChatId,
    pub storage_id: Uuid,
    pub file_total: u64,
    pub chunk_no: u32,
    pub total_chunks: u32,
    pub progress: Option<tokio::sync::mpsc::Sender<UploadProgressEvent>>,
}

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
    fn mask_url(url: &str) -> String {
        if let Some(bot_idx) = url.find("/bot") {
            if let Some(slash_idx) = url[bot_idx + 4..].find('/') {
                return format!("{}/bot***{}", &url[..bot_idx], &url[bot_idx + 4 + slash_idx..]);
            }
        }
        url.to_string()
    }

    /// Seconds to wait for a Telegram flood-control response, if any.
    ///
    /// Official API uses HTTP 429; Local Bot API often answers with HTTP 400 and
    /// `description: "Bad Request: too Many Requests: retry after N"`.
    fn flood_wait_secs(status: reqwest::StatusCode, body: &str) -> Option<u64> {
        let code = status.as_u16();
        let lower = body.to_ascii_lowercase();
        let looks_flood = code == 429
            || lower.contains("too many requests")
            || lower.contains("retry after")
            || lower.contains("\"retry_after\"");
        if !looks_flood {
            return None;
        }

        // JSON: "retry_after": 8  (top-level or under parameters)
        if let Some(idx) = lower.find("\"retry_after\"") {
            let after = &lower[idx + "\"retry_after\"".len()..];
            if let Ok(num) = after
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
            {
                return Some(num.clamp(1, MAX_FLOOD_WAIT_SECS));
            }
        }

        // Text: "retry after 8"
        if let Some(idx) = lower.find("retry after") {
            let after = &lower[idx + "retry after".len()..];
            if let Ok(num) = after
                .chars()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
            {
                return Some(num.clamp(1, MAX_FLOOD_WAIT_SECS));
            }
        }

        Some(8)
    }

    /// Sleep duration for a flood wait: `retry_after` plus a little jitter.
    fn flood_sleep_duration(wait_secs: u64) -> Duration {
        let jitter_cap_ms = (wait_secs.saturating_mul(100)).clamp(100, 2000);
        let jitter_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(100, |d| u64::from(d.subsec_nanos()) % jitter_cap_ms);
        Duration::from_secs(wait_secs) + Duration::from_millis(jitter_ms)
    }

    /// Honor a flood `retry_after`, then apply post-flood cooldown / adaptive pacing.
    ///
    /// Flood waits have no attempt or total-time budget: we keep retrying until the
    /// call succeeds, the client aborts (progress channel closed), or the process
    /// shuts down. Sleep is interruptible every second so cancel is prompt.
    ///
    /// Waiting events use non-blocking `try_send` so a stalled NDJSON client cannot
    /// deadlock the serial Storage Manager (and the per-token send permit).
    ///
    /// While sleeping, re-emit `waiting` every ~15s with remaining `retry_after` so the
    /// HTTP NDJSON stream stays alive (idle proxies otherwise drop the connection).
    async fn honor_flood_wait(
        wait_secs: u64,
        permit: Option<&SendPermit>,
        progress: Option<&tokio::sync::mpsc::Sender<UploadProgressEvent>>,
        uploaded: u64,
        total: u64,
        chunk: u32,
        chunks: u32,
    ) -> SarcaResult<()> {
        let sleep_for = Self::flood_sleep_duration(wait_secs);
        let deadline = Instant::now() + sleep_for;
        let mut last_emit = Instant::now();
        while Instant::now() < deadline {
            if progress.is_some_and(tokio::sync::mpsc::Sender::is_closed) {
                return Err(SarcaError::TelegramAPIError("Upload canceled".to_owned()));
            }
            if let Some(tx) = progress {
                if last_emit.elapsed() >= Duration::from_secs(15) {
                    let remaining =
                        deadline.saturating_duration_since(Instant::now()).as_secs().max(1);
                    // try_send: never block the flood-wait (holds the send permit).
                    emit_upload_progress(
                        tx,
                        UploadProgressEvent::waiting(uploaded, total, chunk, chunks, remaining),
                    )?;
                    last_emit = Instant::now();
                }
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            tokio::time::sleep(remaining.min(Duration::from_secs(1))).await;
        }
        if let Some(p) = permit {
            p.note_flood().await;
        }
        tokio::time::sleep(POST_FLOOD_EXTRA_COOLDOWN).await;
        if progress.is_some_and(tokio::sync::mpsc::Sender::is_closed) {
            return Err(SarcaError::TelegramAPIError("Upload canceled".to_owned()));
        }
        Ok(())
    }

    fn server_backoff_ms(attempt: u32) -> u64 {
        BASE_BACKOFF_MS.saturating_mul(2u64.saturating_pow(attempt)).min(30_000)
    }

    /// Retry network errors, HTTP 5xx, and Telegram flood waits (including HTTP 400
    /// "too Many Requests: retry after N" from Local Bot API).
    ///
    /// Flood waits retry indefinitely. Pass `permit` for upload paths so pacing
    /// adapts after floods.
    async fn send_with_retries<F, Fut>(
        op: &str,
        permit: Option<&SendPermit>,
        mut send: F,
    ) -> SarcaResult<reqwest::Response>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    {
        let mut flood_tries: u32 = 0;
        let mut flood_waited_secs: u64 = 0;
        let mut other_tries: u32 = 0;

        loop {
            match send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
                    }

                    let body = response.text().await.unwrap_or_default();
                    if let Some(wait_secs) = Self::flood_wait_secs(status, &body) {
                        flood_tries += 1;
                        flood_waited_secs = flood_waited_secs.saturating_add(wait_secs);
                        // Expected path under Local Bot API — warn once, then retry quietly.
                        if flood_tries == 1 {
                            tracing::warn!(
                                "[TELEGRAM API] {op} flood wait {wait_secs}s (will retry \
                                 indefinitely): {body}"
                            );
                        } else {
                            tracing::debug!(
                                "[TELEGRAM API] {op} flood wait {wait_secs}s (flood attempt \
                                 {flood_tries}, waited {flood_waited_secs}s total)"
                            );
                        }
                        Self::honor_flood_wait(wait_secs, permit, None, 0, 0, 0, 0).await?;
                        continue;
                    } else if status.is_server_error() {
                        other_tries += 1;
                        if other_tries < MAX_ATTEMPTS {
                            let backoff = Self::server_backoff_ms(other_tries.saturating_sub(1));
                            tracing::warn!(
                                "[TELEGRAM API] {op} got {status}, retrying in {backoff}ms \
                                 (attempt {other_tries}/{MAX_ATTEMPTS}): {body}"
                            );
                            tokio::time::sleep(Duration::from_millis(backoff)).await;
                            continue;
                        }
                    }

                    // Rebuild a synthetic failure response path: caller expects Response,
                    // but we already consumed the body. Return a clear error instead.
                    return Err(SarcaError::TelegramAPIError(format!("{status}: {body}")));
                },
                Err(e) => {
                    other_tries += 1;
                    if other_tries < MAX_ATTEMPTS {
                        let backoff = Self::server_backoff_ms(other_tries.saturating_sub(1));
                        tracing::warn!(
                            "[TELEGRAM API] {op} network error, retrying in {backoff}ms (attempt \
                             {other_tries}/{MAX_ATTEMPTS}): {e}"
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        continue;
                    }
                    return Err(SarcaError::from(e));
                },
            }
        }
    }

    pub async fn upload(
        &self,
        file: &[u8],
        chat_id: ChatId,
        storage_id: Uuid,
    ) -> SarcaResult<UploadOutcome> {
        if chat_id < 0 && chat_id > -10_000_000_000 {
            tracing::info!(
                "[TELEGRAM API] Using regular group (chat_id={}). If bot can't find the chat, \
                 make sure the bot is added and has permissions.",
                chat_id
            );
        }

        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "sendDocument", &token);
        let masked_url = Self::mask_url(&url);
        let file_len = file.len();

        let start = Instant::now();
        let permit = SendPermit::acquire(&token).await;
        let response = Self::send_with_retries("upload", Some(&permit), || {
            let file_part = multipart::Part::bytes(file.to_vec()).file_name("sarca_chunk.bin");
            let form = multipart::Form::new()
                .text("chat_id", chat_id.to_string())
                .part("document", file_part);
            reqwest::Client::new().post(&url).multipart(form).send()
        })
        .await?;
        permit.mark_ok().await;
        drop(permit);
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
            return Err(SarcaError::TelegramAPIError(format!("{status}: {error_body}")));
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

        let outcome = UploadOutcome {
            file_id: result.result.document.file_id,
            message_id: result.result.message_id,
        };
        self.cleanup_local_bot_api_copy(&outcome.file_id, storage_id).await;
        Ok(outcome)
    }

    /// Build the streaming multipart form for one upload attempt of `upload_file_part`.
    ///
    /// Rebuilt per attempt because the underlying file stream can't be replayed.
    async fn build_upload_part_form(
        file_path: &Path,
        req: &UploadFilePartRequest,
    ) -> SarcaResult<multipart::Form> {
        use std::sync::atomic::{AtomicU64, Ordering};

        use futures::StreamExt;

        let mut file = tokio::fs::File::open(file_path).await.map_err(|_| SarcaError::Unknown)?;
        file.seek(SeekFrom::Start(req.offset)).await.map_err(|_| SarcaError::Unknown)?;
        let reader = file.take(req.len);
        let base_stream = ReaderStream::new(reader);

        let sent = AtomicU64::new(0);
        let last_emit = AtomicU64::new(0);
        let progress_tx = req.progress.clone();
        let (offset, len, file_total, chunk_no, total_chunks) =
            (req.offset, req.len, req.file_total, req.chunk_no, req.total_chunks);
        let stream = base_stream.map(move |item| {
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
        });
        let body = reqwest::Body::wrap_stream(stream);
        let part = multipart::Part::stream_with_length(body, req.len).file_name("sarca_chunk.bin");
        Ok(multipart::Form::new().text("chat_id", req.chat_id.to_string()).part("document", part))
    }

    /// Custom retry loop for `upload_file_part` because the multipart stream must be
    /// rebuilt each attempt (unlike `send_with_retries`, which reuses a closure).
    ///
    /// Flood waits retry indefinitely per chunk (no attempt / total-time budget).
    async fn send_upload_part_with_retries(
        url: &str,
        file_path: &Path,
        req: &UploadFilePartRequest,
        permit: &SendPermit,
    ) -> SarcaResult<reqwest::Response> {
        let mut flood_tries: u32 = 0;
        let mut flood_waited_secs: u64 = 0;
        let mut other_tries: u32 = 0;

        loop {
            if req.progress.as_ref().is_some_and(tokio::sync::mpsc::Sender::is_closed) {
                return Err(SarcaError::TelegramAPIError("Upload canceled".to_owned()));
            }
            let form = Self::build_upload_part_form(file_path, req).await?;
            // Bound connect time; large local-API chunks have no total timeout — cancel
            // via progress.closed() when the NDJSON client disconnects.
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let send_fut = client.post(url).multipart(form).send();
            let result = if let Some(tx) = req.progress.as_ref() {
                tokio::select! {
                    r = send_fut => r,
                    () = tx.closed() => {
                        return Err(SarcaError::TelegramAPIError("Upload canceled".to_owned()));
                    }
                }
            } else {
                send_fut.await
            };
            match result {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
                    }

                    let body = response.text().await.unwrap_or_default();
                    if let Some(wait_secs) = Self::flood_wait_secs(status, &body) {
                        flood_tries += 1;
                        flood_waited_secs = flood_waited_secs.saturating_add(wait_secs);
                        // Expected path under Local Bot API — warn once, then retry quietly.
                        if flood_tries == 1 {
                            tracing::warn!(
                                "[TELEGRAM API] upload_file_part flood wait {wait_secs}s (will \
                                 retry indefinitely): {body}"
                            );
                        } else {
                            tracing::debug!(
                                "[TELEGRAM API] upload_file_part flood wait {wait_secs}s (flood \
                                 attempt {flood_tries}, waited {flood_waited_secs}s total)"
                            );
                        }
                        if let Some(tx) = req.progress.as_ref() {
                            // Never await: a full NDJSON buffer must not hold the send permit.
                            emit_upload_progress(
                                tx,
                                UploadProgressEvent::waiting(
                                    req.offset,
                                    req.file_total,
                                    req.chunk_no,
                                    req.total_chunks,
                                    wait_secs,
                                ),
                            )?;
                        }
                        Self::honor_flood_wait(
                            wait_secs,
                            Some(permit),
                            req.progress.as_ref(),
                            req.offset,
                            req.file_total,
                            req.chunk_no,
                            req.total_chunks,
                        )
                        .await?;
                        continue;
                    } else if status.is_server_error() {
                        other_tries += 1;
                        if other_tries < MAX_ATTEMPTS {
                            let backoff = Self::server_backoff_ms(other_tries.saturating_sub(1));
                            tracing::warn!(
                                "[TELEGRAM API] upload_file_part got {status}, retrying in \
                                 {backoff}ms (attempt {other_tries}/{MAX_ATTEMPTS}): {body}"
                            );
                            tokio::time::sleep(Duration::from_millis(backoff)).await;
                            continue;
                        }
                    }

                    return Err(SarcaError::TelegramAPIError(format!("{status}: {body}")));
                },
                Err(e) => {
                    other_tries += 1;
                    if other_tries < MAX_ATTEMPTS {
                        let backoff = Self::server_backoff_ms(other_tries.saturating_sub(1));
                        tracing::warn!(
                            "[TELEGRAM API] upload_file_part network error, retrying in \
                             {backoff}ms (attempt {other_tries}/{MAX_ATTEMPTS})"
                        );
                        tokio::time::sleep(Duration::from_millis(backoff)).await;
                        continue;
                    }
                    return Err(e.into());
                },
            }
        }
    }

    /// Upload a part of a file from disk without buffering it fully in RAM.
    ///
    /// `req.offset` and `req.len` define the slice of the file to upload.
    /// Optional `req.progress` reports bytes within the whole file (`file_base + sent`).
    pub async fn upload_file_part(
        &self,
        file_path: &Path,
        req: UploadFilePartRequest,
    ) -> SarcaResult<UploadOutcome> {
        let token = self.scheduler.get_token(req.storage_id).await?;
        let url = self.build_url("", "sendDocument", &token);
        let masked_url = Self::mask_url(&url);

        let start = Instant::now();
        let permit = SendPermit::acquire(&token).await;
        let response = Self::send_upload_part_with_retries(&url, file_path, &req, &permit).await?;
        permit.mark_ok().await;
        drop(permit);
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

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
                        "chat_id": req.chat_id,
                        "offset": req.offset,
                        "len": req.len,
                        "storage_id": req.storage_id.to_string()
                    },
                    "response": error_body,
                    "elapsed_ms": elapsed_ms
                })
            );
            return Err(SarcaError::TelegramAPIError(format!("{status}: {error_body}")));
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
                    "chat_id": req.chat_id,
                    "offset": req.offset,
                    "len": req.len,
                    "storage_id": req.storage_id.to_string()
                },
                "response": {
                    "telegram_file_id": result.result.document.file_id
                },
                "elapsed_ms": elapsed_ms
            })
        );

        let outcome = UploadOutcome {
            file_id: result.result.document.file_id,
            message_id: result.result.message_id,
        };
        self.cleanup_local_bot_api_copy(&outcome.file_id, req.storage_id).await;
        Ok(outcome)
    }

    /// After Local Bot API accepts an upload it keeps a disk copy under `documents/`.
    /// Resolve that path via `getFile` and delete it — durable bytes already live in Telegram.
    async fn cleanup_local_bot_api_copy(&self, telegram_file_id: &str, storage_id: Uuid) {
        let Ok(token) = self.scheduler.get_token(storage_id).await else {
            return;
        };
        let url = self.build_url("", "getFile", &token);
        let body = match reqwest::Client::new()
            .get(&url)
            .query(&[("file_id", telegram_file_id)])
            .send()
            .await
        {
            Ok(resp) => {
                match resp.error_for_status() {
                    Ok(ok) => {
                        match ok.json::<DownloadBodySchema>().await {
                            Ok(body) => body,
                            Err(_) => return,
                        }
                    },
                    Err(_) => return,
                }
            },
            Err(_) => return,
        };
        if body.result.file_path.starts_with('/') {
            maybe_remove_local_bot_api_file(&body.result.file_path).await;
        }
    }

    /// Local Bot API writes files as owner-only briefly; our entrypoint chmod loop
    /// opens them for Sarca (`nobody`). Retry `PermissionDenied` / `NotFound` so downloads
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
                        "[TELEGRAM API] local file open attempt {attempt}/{ATTEMPTS} path={} \
                         err={e}",
                        path
                    );
                    last_err = Some(e);
                    tokio::time::sleep(Duration::from_millis(DELAY_MS)).await;
                },
                Err(e) => {
                    tracing::error!("[TELEGRAM API] local file open failed path={} err={e}", path);
                    return Err(SarcaError::TelegramAPIError(format!(
                        "Failed to open local bot api file: {e}"
                    )));
                },
            }
        }

        let e = last_err.expect("at least one permission/not-found error");
        tracing::error!(
            "[TELEGRAM API] local file open failed path={} err={e} after {ATTEMPTS} attempts. \
             Ensure telegram-bot-api-data is mounted and world-readable.",
            path
        );
        Err(SarcaError::TelegramAPIError(format!("Failed to open local bot api file: {e}")))
    }

    async fn read_local_bot_api_file(path: &str) -> SarcaResult<Vec<u8>> {
        let mut file = Self::open_local_bot_api_file(path).await?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).await.map_err(|e| {
            SarcaError::TelegramAPIError(format!("Failed to read local bot api file: {e}"))
        })?;
        Ok(bytes)
    }

    pub async fn download(&self, telegram_file_id: &str, storage_id: Uuid) -> SarcaResult<Vec<u8>> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "getFile", &token);
        let masked_url = Self::mask_url(&url);

        let start = Instant::now();
        let response = Self::send_with_retries("download/getFile", None, || {
            reqwest::Client::new().get(&url).query(&[("file_id", telegram_file_id)]).send()
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
            return Err(SarcaError::TelegramAPIError(format!("{status}: {error_body}")));
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
            if !body.result.file_path.starts_with(LOCAL_BOT_API_DATA_PREFIX) {
                return Err(SarcaError::TelegramAPIError(
                    "Unexpected local file_path from telegram-bot-api".to_string(),
                ));
            }

            let path = body.result.file_path;
            let bytes = Self::read_local_bot_api_file(&path).await?;
            maybe_remove_local_bot_api_file(&path).await;
            return Ok(bytes);
        }

        // downloading the file itself
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("file/", &body.result.file_path, &token);
        let masked_url = Self::mask_url(&url);

        let start = Instant::now();
        let response = Self::send_with_retries("download/file", None, || {
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
            return Err(SarcaError::TelegramAPIError(format!("{status}: {error_body}")));
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
        let url = self.build_url("", "getFile", &token);

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
            if !body.result.file_path.starts_with(LOCAL_BOT_API_DATA_PREFIX) {
                return Err(SarcaError::TelegramAPIError(
                    "Unexpected local file_path from telegram-bot-api".to_string(),
                ));
            }

            let path = body.result.file_path;
            let file = Self::open_local_bot_api_file(&path).await?;
            let stream = ReaderStream::new(file).map(|res| {
                res.map_err(|e| {
                    SarcaError::TelegramAPIError(format!("Failed to read local bot api file: {e}"))
                })
            });
            return Ok(Box::pin(CleanupLocalFileStream {
                inner: stream,
                path: Some(path),
            }));
        }

        // downloading the file itself
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("file/", &body.result.file_path, &token);

        let response = reqwest::Client::new().get(url).send().await?.error_for_status()?;

        let stream = response.bytes_stream().map(|res| res.map_err(SarcaError::from));

        Ok(Box::pin(stream))
    }

    /// Resolve a chat's display name (title, else username, else first name, else the id).
    pub async fn get_chat(&self, chat_id: ChatId, storage_id: Uuid) -> SarcaResult<ChatInfo> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "getChat", &token);
        let masked_url = Self::mask_url(&url);

        let response = Self::send_with_retries("getChat", None, || {
            reqwest::Client::new().get(&url).query(&[("chat_id", chat_id.to_string())]).send()
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
            return Err(SarcaError::TelegramAPIError(format!("{status}: {error_body}")));
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

    /// Copy a message (with its document) from one chat to another without re-uploading.
    ///
    /// Telegram's `copyMessage` only returns the new `message_id`; the underlying file
    /// stays the same document, so the caller-supplied `source_file_id` remains valid for
    /// download via `getFile` as long as the bot can still reach any chat holding it.
    ///
    /// Uses the same per-token send gate as `sendDocument` so replication cannot race
    /// uploads and trip flood control.
    pub async fn copy_message(
        &self,
        from_chat_id: ChatId,
        message_id: i64,
        to_chat_id: ChatId,
        source_file_id: &str,
        storage_id: Uuid,
    ) -> SarcaResult<UploadOutcome> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "copyMessage", &token);
        let masked_url = Self::mask_url(&url);

        let permit = SendPermit::acquire(&token).await;
        let response = Self::send_with_retries("copyMessage", Some(&permit), || {
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
        permit.mark_ok().await;
        drop(permit);

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
            return Err(SarcaError::TelegramAPIError(format!("{status}: {error_body}")));
        }

        let body: CopyMessageBodySchema = response.json().await?;

        Ok(UploadOutcome {
            file_id: source_file_id.to_owned(),
            message_id: body.result.message_id,
        })
    }

    /// Best-effort Telegram `deleteMessage`. Missing/already-deleted messages are treated as
    /// success. Shares the per-token send gate so purge cannot race uploads.
    pub async fn delete_message(
        &self,
        chat_id: ChatId,
        message_id: i64,
        storage_id: Uuid,
    ) -> SarcaResult<()> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "deleteMessage", &token);
        let masked_url = Self::mask_url(&url);

        let permit = SendPermit::acquire(&token).await;
        let response = Self::send_with_retries("deleteMessage", Some(&permit), || {
            reqwest::Client::new()
                .post(&url)
                .form(&[("chat_id", chat_id.to_string()), ("message_id", message_id.to_string())])
                .send()
        })
        .await?;
        permit.mark_ok().await;
        drop(permit);

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

        Err(SarcaError::TelegramAPIError(format!("{status}: {error_body}")))
    }

    #[inline]
    fn build_url(&self, pre: &str, relative: &str, token: &str) -> String {
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

const LOCAL_BOT_API_DATA_PREFIX: &str = "/var/lib/telegram-bot-api/";

/// Absolute path under Local Bot API `documents/` that is safe for Sarca to delete
/// after upload/download. Rejects `/temp/`, path traversal, and anything outside the
/// standard data dir (see tdlib/telegram-bot-api#303).
fn is_deletable_local_bot_api_file(path: &str) -> bool {
    use std::path::{Component, Path};

    let path = Path::new(path);
    if !path.is_absolute() {
        return false;
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }

    // Prefer `components()` over `iter()`: on Unix, `iter()` yields a leading empty
    // OsStr for absolute paths which shifts every index.
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| {
            match c {
                Component::Normal(s) => s.to_str(),
                _ => None,
            }
        })
        .collect();
    // var, lib, telegram-bot-api, <bot>, documents, <file>
    if parts.len() < 6 {
        return false;
    }
    if parts[0] != "var" || parts[1] != "lib" || parts[2] != "telegram-bot-api" {
        return false;
    }
    if parts[3].is_empty() || parts[4] != "documents" {
        return false;
    }
    // Require at least one filename under documents/ (not the directory itself).
    parts[5..].iter().any(|p| !p.is_empty())
}

/// Best-effort delete of a Local Bot API `documents/` file after Sarca is done with it.
async fn maybe_remove_local_bot_api_file(path: &str) {
    if !is_deletable_local_bot_api_file(path) {
        return;
    }
    match tokio::fs::remove_file(path).await {
        Ok(()) => {
            tracing::debug!("[TELEGRAM API] removed local bot-api document {path}");
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => {
            tracing::warn!("[TELEGRAM API] failed to remove local bot-api document {path}: {e}");
        },
    }
}

/// Stream wrapper that unlinks a Local Bot API documents file when dropped (after the
/// reader finishes or the client cancels).
struct CleanupLocalFileStream<S> {
    inner: S,
    path: Option<String>,
}

impl<S: Stream + Unpin> Stream for CleanupLocalFileStream<S> {
    type Item = S::Item;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Drop for CleanupLocalFileStream<S> {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if !is_deletable_local_bot_api_file(&path) {
            return;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => {
                tracing::debug!("[TELEGRAM API] removed local bot-api document {path}");
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => {
                tracing::warn!(
                    "[TELEGRAM API] failed to remove local bot-api document {path}: {e}"
                );
            },
        }
    }
}

#[cfg(test)]
mod local_bot_api_cleanup_tests {
    use super::is_deletable_local_bot_api_file;

    #[test]
    fn accepts_documents_file() {
        assert!(is_deletable_local_bot_api_file(
            "/var/lib/telegram-bot-api/123:AAtoken/documents/file_23"
        ));
    }

    #[test]
    fn rejects_temp_file() {
        assert!(!is_deletable_local_bot_api_file("/var/lib/telegram-bot-api/123:AAtoken/temp/38"));
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(!is_deletable_local_bot_api_file(
            "/var/lib/telegram-bot-api/123:AAtoken/documents/../../temp/x"
        ));
        assert!(!is_deletable_local_bot_api_file("/var/lib/telegram-bot-api/../etc/passwd"));
    }

    #[test]
    fn rejects_outside_prefix() {
        assert!(!is_deletable_local_bot_api_file("/tmp/file_23"));
        assert!(!is_deletable_local_bot_api_file("documents/file_23"));
    }

    #[test]
    fn rejects_documents_directory_itself() {
        assert!(!is_deletable_local_bot_api_file(
            "/var/lib/telegram-bot-api/123:AAtoken/documents"
        ));
        assert!(!is_deletable_local_bot_api_file(
            "/var/lib/telegram-bot-api/123:AAtoken/documents/"
        ));
    }
}

#[cfg(test)]
mod flood_wait_tests {
    use super::{MAX_FLOOD_WAIT_SECS, TelegramBotApi};

    #[test]
    fn parses_local_bot_api_400_flood() {
        let body = r#"{"ok":false,"error_code":400,"description":"Bad Request: too Many Requests: retry after 8"}"#;
        let secs = TelegramBotApi::flood_wait_secs(reqwest::StatusCode::BAD_REQUEST, body);
        assert_eq!(secs, Some(8));
    }

    #[test]
    fn parses_retry_after_json() {
        let body =
            r#"{"ok":false,"error_code":429,"description":"Too Many Requests","retry_after":42}"#;
        let secs = TelegramBotApi::flood_wait_secs(reqwest::StatusCode::TOO_MANY_REQUESTS, body);
        assert_eq!(secs, Some(42));
    }

    #[test]
    fn parses_parameters_retry_after() {
        let body = r#"{"ok":false,"error_code":429,"description":"Too Many Requests: retry after 35","parameters":{"retry_after":35}}"#;
        let secs = TelegramBotApi::flood_wait_secs(reqwest::StatusCode::TOO_MANY_REQUESTS, body);
        assert_eq!(secs, Some(35));
    }

    #[test]
    fn ignores_ordinary_bad_request() {
        let body = r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#;
        assert_eq!(TelegramBotApi::flood_wait_secs(reqwest::StatusCode::BAD_REQUEST, body), None);
    }

    #[test]
    fn caps_single_flood_wait_at_max() {
        let body = format!(
            r#"{{"ok":false,"error_code":429,"description":"Too Many Requests","retry_after":{}}}"#,
            MAX_FLOOD_WAIT_SECS + 500
        );
        let secs = TelegramBotApi::flood_wait_secs(reqwest::StatusCode::TOO_MANY_REQUESTS, &body);
        assert_eq!(secs, Some(MAX_FLOOD_WAIT_SECS));
    }

    #[test]
    fn flood_sleep_includes_retry_after() {
        let d = TelegramBotApi::flood_sleep_duration(10);
        assert!(d.as_secs() >= 10);
        assert!(d.as_secs() <= 12);
    }

    #[test]
    fn pacing_defaults_are_conservative() {
        // Keep proactive gaps well under Telegram's ~1 msg/s FAQ guideline.
        assert!(super::MIN_SEND_GAP.as_millis() >= 2000);
        assert!(super::MIN_SEND_GAP_AFTER_FLOOD.as_millis() >= 3000);
        assert!(super::POST_FLOOD_EXTRA_COOLDOWN.as_secs() >= 5);
        assert!(super::FLOOD_PACING_WINDOW.as_secs() >= 180);
    }
}
