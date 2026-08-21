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

use super::{
    http_client,
    schemas::{
        ChatInfo,
        CopyMessageBodySchema,
        DownloadBodySchema,
        GetChatBodySchema,
        UploadBodySchema,
        UploadOutcome,
    },
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
/// conservative so multi-chunk uploads don't trip flood control.
const MIN_SEND_GAP: Duration = Duration::from_millis(2200);
/// Elevated inter-send gap while a token is in a recent flood window.
const MIN_SEND_GAP_AFTER_FLOOD: Duration = Duration::from_secs(3);
/// How long after a flood wait we keep the elevated send gap.
const FLOOD_PACING_WINDOW: Duration = Duration::from_mins(5);
/// Extra cooldown after honoring `retry_after`, before the next attempt.
const POST_FLOOD_EXTRA_COOLDOWN: Duration = Duration::from_secs(7);

/// Test hook: `SARCA_TELEGRAM_PACING_MS` overrides the proactive send pacing so e2e
/// runs against a local fake Bot API don't sleep seconds between documents. Never set
/// this against api.telegram.org — the defaults exist to stay under flood control.
fn pacing_override() -> Option<Duration> {
    static OVERRIDE: OnceLock<Option<Duration>> = OnceLock::new();
    *OVERRIDE.get_or_init(|| {
        let ms = std::env::var("SARCA_TELEGRAM_PACING_MS").ok()?.parse::<u64>().ok()?;
        tracing::warn!("[TELEGRAM API] send pacing overridden to {ms}ms (test hook)");
        Some(Duration::from_millis(ms))
    })
}

fn min_send_gap() -> Duration {
    pacing_override().unwrap_or(MIN_SEND_GAP)
}

fn min_send_gap_after_flood() -> Duration {
    pacing_override().unwrap_or(MIN_SEND_GAP_AFTER_FLOOD)
}

fn post_flood_extra_cooldown() -> Duration {
    pacing_override().unwrap_or(POST_FLOOD_EXTRA_COOLDOWN)
}

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
                min_send_gap_after_flood()
            } else {
                min_send_gap()
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
        note_global_flood();
    }
}

/// Process-wide flood window, mirroring the per-token one. The storage manager
/// reads it to shrink file-level upload concurrency: per-token pacing alone does
/// not help when several tokens of the same storage are all being throttled.
fn global_flood_until() -> &'static std::sync::Mutex<Option<Instant>> {
    static UNTIL: OnceLock<std::sync::Mutex<Option<Instant>>> = OnceLock::new();
    UNTIL.get_or_init(|| std::sync::Mutex::new(None))
}

fn note_global_flood() {
    if let Ok(mut slot) = global_flood_until().lock() {
        *slot = Some(Instant::now() + FLOOD_PACING_WINDOW);
    }
}

/// True while any token flooded within the last `FLOOD_PACING_WINDOW`.
pub fn flood_active() -> bool {
    global_flood_until()
        .lock()
        .ok()
        .and_then(|slot| *slot)
        .is_some_and(|until| Instant::now() < until)
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
    /// Official API uses HTTP 429; some responses use HTTP 400 with
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
        tokio::time::sleep(post_flood_extra_cooldown()).await;
        if progress.is_some_and(tokio::sync::mpsc::Sender::is_closed) {
            return Err(SarcaError::TelegramAPIError("Upload canceled".to_owned()));
        }
        Ok(())
    }

    fn server_backoff_ms(attempt: u32) -> u64 {
        BASE_BACKOFF_MS.saturating_mul(2u64.saturating_pow(attempt)).min(30_000)
    }

    /// Retry network errors, HTTP 5xx, and Telegram flood waits.
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
                        // Expected path under rate limit — warn once, then retry quietly.
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
            http_client::client().post(&url).multipart(form).send()
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

        Ok(UploadOutcome {
            file_id: result.result.document.file_id,
            message_id: result.result.message_id,
        })
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
            // Shared pooled client: connect time is bounded (see http_client),
            // and cancel is still via progress.closed() when the NDJSON client
            // disconnects, below.
            let send_fut = http_client::client().post(url).multipart(form).send();
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
                        // Expected path under rate limit — warn once, then retry quietly.
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

        Ok(UploadOutcome {
            file_id: result.result.document.file_id,
            message_id: result.result.message_id,
        })
    }

    pub async fn download(&self, telegram_file_id: &str, storage_id: Uuid) -> SarcaResult<Vec<u8>> {
        self.download_impl(telegram_file_id, storage_id, None).await
    }

    /// Like `download`, but gives up and returns `StorageBusy` if no token is
    /// available before `deadline` — for interactive read paths that would
    /// rather 503 than hold a request open for minutes.
    pub async fn download_before(
        &self,
        telegram_file_id: &str,
        storage_id: Uuid,
        deadline: Instant,
    ) -> SarcaResult<Vec<u8>> {
        self.download_impl(telegram_file_id, storage_id, Some(deadline)).await
    }

    async fn download_impl(
        &self,
        telegram_file_id: &str,
        storage_id: Uuid,
        deadline: Option<Instant>,
    ) -> SarcaResult<Vec<u8>> {
        let token = match deadline {
            Some(deadline) => self.scheduler.get_token_before(storage_id, deadline).await?,
            None => self.scheduler.get_token(storage_id).await?,
        };
        let url = self.build_url("", "getFile", &token);
        let masked_url = Self::mask_url(&url);

        let start = Instant::now();
        let response = Self::send_with_retries("download/getFile", None, || {
            http_client::client().get(&url).query(&[("file_id", telegram_file_id)]).send()
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

        let url = self.build_url("file/", &body.result.file_path, &token);
        let masked_url = Self::mask_url(&url);

        let start = Instant::now();
        let response = Self::send_with_retries("download/file", None, || {
            http_client::client().get(&url).send()
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

        let body: DownloadBodySchema = http_client::client()
            .get(url)
            .query(&[("file_id", telegram_file_id)])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let url = self.build_url("file/", &body.result.file_path, &token);

        let response = http_client::client().get(url).send().await?.error_for_status()?;

        let stream = response.bytes_stream().map(|res| res.map_err(SarcaError::from));

        Ok(Box::pin(stream))
    }

    /// Resolve a chat's display name (title, else username, else first name, else the id).
    pub async fn get_chat(&self, chat_id: ChatId, storage_id: Uuid) -> SarcaResult<ChatInfo> {
        let token = self.scheduler.get_token(storage_id).await?;
        let url = self.build_url("", "getChat", &token);
        let masked_url = Self::mask_url(&url);

        let response = Self::send_with_retries("getChat", None, || {
            http_client::client().get(&url).query(&[("chat_id", chat_id.to_string())]).send()
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
            http_client::client()
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
        self.delete_message_with_token(chat_id, message_id, &token).await
    }

    /// Same as [`Self::delete_message`] but uses a caller-supplied bot token (e.g. after
    /// storage/worker rows are removed during durable purge).
    pub async fn delete_message_with_token(
        &self,
        chat_id: ChatId,
        message_id: i64,
        token: &str,
    ) -> SarcaResult<()> {
        let url = self.build_url("", "deleteMessage", token);

        let permit = SendPermit::acquire(token).await;
        let result = Self::send_with_retries("deleteMessage", Some(&permit), || {
            http_client::client()
                .post(&url)
                .form(&[("chat_id", chat_id.to_string()), ("message_id", message_id.to_string())])
                .send()
        })
        .await;
        if result.is_ok() {
            permit.mark_ok().await;
        }
        drop(permit);

        match result {
            Ok(_) => Ok(()),
            Err(error) if is_soft_delete_error(&error.to_string()) => Ok(()),
            Err(error) => Err(error),
        }
    }

    #[inline]
    fn build_url(&self, pre: &str, relative: &str, token: &str) -> String {
        format!("{}/{pre}bot{token}/{relative}", self.base_url)
    }
}

fn is_soft_delete_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    ["message to delete not found", "message can't be deleted", "message_id_invalid"]
        .iter()
        .any(|marker| lower.contains(marker))
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

#[cfg(test)]
mod delete_message_tests {
    use super::is_soft_delete_error;

    #[test]
    fn accepts_already_missing_delete_errors() {
        assert!(is_soft_delete_error("400 Bad Request: message to delete not found"));
        assert!(is_soft_delete_error(r#"{"description":"Bad Request: message can't be deleted"}"#));
        assert!(is_soft_delete_error("400: MESSAGE_ID_INVALID"));
        assert!(!is_soft_delete_error("400 Bad Request: chat not found"));
    }
}
