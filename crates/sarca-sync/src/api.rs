use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use futures::{StreamExt, TryStreamExt};
use reqwest::{
    header::{CONTENT_RANGE, RANGE},
    multipart::{Form, Part},
    Client, Response, StatusCode, Version,
};
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::types::{ChangelogResponse, SnapshotResponse};

/// Whether the sync client was built to prefer HTTP/3 (reqwest `http3` + `reqwest_unstable`).
pub const HTTP3_PREFERRED: bool = cfg!(all(feature = "http3-client", reqwest_unstable));

/// Largest single file we will pull down. The body used to be buffered whole in
/// memory, so a server answering a small `snapshot` entry with a multi-gigabyte
/// response could OOM the client. 16 GiB is far above any real media file and
/// still bounds the damage.
const MAX_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;

/// Budget for a whole control-plane call (login, snapshot, changelog, delete).
/// These answer immediately or not at all, so a short deadline is right.
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(20);

/// Handshake budget. Unreachable servers must still fail fast even though the
/// transfer deadlines below are measured in minutes.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longest silence tolerated between two frames of a *response body*.
///
/// An upload's NDJSON progress stream heartbeats every 15s (see the server's
/// `HEARTBEAT_SECS`), so a body that goes quiet for this long is dead however
/// long the transfer as a whole is allowed to take.
///
/// Applied by [`drain_upload_progress`] and [`write_response_body`], *not* by
/// `reqwest::ClientBuilder::read_timeout`. That setting reads like this one and
/// is not: reqwest arms a single sleep when the request is dispatched, never
/// resets it, and fails the whole request if the response *head* has not
/// arrived when it expires. The upload endpoint answers only once the entire
/// multipart body is on the server's disk, so a client-wide `read_timeout` was
/// really a hard 45-second ceiling on time-to-first-byte — every file too big
/// to push in that window died with "error sending request for url (…):
/// operation timed out" on a perfectly healthy connection. The send side is
/// bounded by [`UPLOAD_STALL_TIMEOUT`] and the per-request `transfer_timeout`
/// instead.
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

/// Longest an upload may hand *zero* bytes to the connection before we call the
/// socket wedged.
///
/// [`READ_IDLE_TIMEOUT`] cannot cover this: while the body is going up there is
/// no response to read yet. `transfer_timeout` is sized for a whole
/// multi-gigabyte file, so on its own it would let a dead connection hold a
/// sync slot for hours. Deliberately generous — a congested uplink may go quiet
/// for a while, but not for two minutes with nothing accepted at all.
const UPLOAD_STALL_TIMEOUT: Duration = Duration::from_secs(120);

/// How often the stall watchdog wakes to compare against the last progress.
const UPLOAD_STALL_POLL: Duration = Duration::from_secs(5);

/// Floor for one file transfer, whatever its size.
const TRANSFER_MIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Ceiling for one file transfer — a stop so a wedged connection that keeps
/// trickling bytes cannot occupy a sync slot forever.
const TRANSFER_MAX_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);

/// Worst-case sustained throughput assumed when sizing a transfer deadline.
const TRANSFER_MIN_BYTES_PER_SEC: u64 = 32 * 1024;

/// `download_to` only fans out into parallel Range requests above this size —
/// small files aren't worth the extra round trips.
const PARALLEL_DOWNLOAD_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

/// How many concurrent `Range` requests one file download fans out to. The
/// server caches Telegram chunks independently and keyed per-chunk (see
/// `ChunkCache`/`SingleFlight`), so distinct Range windows let it pull several
/// Telegram chunks at once instead of the single-chunk-ahead prefetch a lone
/// sequential GET gets.
const PARALLEL_DOWNLOAD_PARTS: u64 = 4;

/// Total deadline for transferring `bytes`, clamped to
/// `[TRANSFER_MIN_TIMEOUT, TRANSFER_MAX_TIMEOUT]`.
///
/// A single flat deadline cannot work here: the server answers an upload only
/// after it has pushed the file to Telegram, which routinely takes longer than
/// a control call ever should. Sizing the deadline by payload keeps small
/// files from hanging around forever while giving large ones room to finish.
pub fn transfer_timeout(bytes: u64) -> Duration {
    let by_size = Duration::from_secs(bytes / TRANSFER_MIN_BYTES_PER_SEC);
    by_size.clamp(TRANSFER_MIN_TIMEOUT, TRANSFER_MAX_TIMEOUT)
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub email_verified: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageSummary {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StoragesResponse {
    pub storages: Vec<StorageSummary>,
}

#[derive(Clone)]
pub struct SarcaApi {
    /// Lazily built HTTP/3 client (`http3_prior_knowledge` needs a tokio runtime).
    h3_client: Arc<OnceLock<Client>>,
    /// Client used for TCP HTTPS fallback (ALPN `h2`/`http/1.1`).
    tcp_client: Client,
    timeout: Duration,
    base_url: String,
    access_token: String,
}

impl SarcaApi {
    pub fn new(base_url: impl Into<String>, access_token: impl Into<String>) -> Self {
        let timeout = DEFAULT_HTTP_TIMEOUT;
        let tcp_client = build_tcp_client(timeout).expect("failed to create HTTP client");
        Self {
            h3_client: Arc::new(OnceLock::new()),
            tcp_client,
            timeout,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            access_token: access_token.into(),
        }
    }

    fn h3_client(&self) -> &Client {
        ensure_h3_client(&self.h3_client, self.timeout).unwrap_or(&self.tcp_client)
    }

    fn clients(&self) -> HttpClients {
        HttpClients {
            h3: self.h3_client().clone(),
            tcp: self.tcp_client.clone(),
        }
    }

    pub fn set_token(&mut self, token: impl Into<String>) {
        self.access_token = token.into();
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// `Authorization` header value (`Bearer …`) when an access token is present.
    pub fn authorization_header(&self) -> Option<String> {
        authorization_header_value(&self.access_token)
    }

    fn require_access_token(&self) -> Result<()> {
        if self.access_token.trim().is_empty() {
            bail!(
                "Not authenticated — missing access token. Sign in again so Sync can use your session."
            );
        }
        Ok(())
    }

    /// Password login against `{base}/api/auth/login` (no prior token required).
    pub async fn login(
        base_url: impl AsRef<str>,
        email: impl AsRef<str>,
        password: impl AsRef<str>,
    ) -> Result<LoginResponse> {
        let base = normalize_server_url(base_url.as_ref())?;
        let clients = build_http_clients(DEFAULT_HTTP_TIMEOUT)?;
        let url = format!("{base}/api/auth/login");
        let resp = match send_preferring_h3(&clients, "POST", &url, |client, version| {
            client.post(&url).version(version).json(&serde_json::json!({
                "email": email.as_ref(),
                "password": password.as_ref(),
            }))
        })
        .await
        {
            Ok(resp) => resp,
            Err(err) => {
                if err.is_timeout() {
                    bail!("Cannot reach server — connection timed out. Check the URL and network.");
                }
                if err.is_connect()
                    || err.is_request()
                    || err.to_string().to_ascii_lowercase().contains("dns")
                {
                    bail!(
                        "Cannot reach server — no connection. Check the URL and that the server is running."
                    );
                }
                bail!("Cannot reach server: {err}");
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                bail!("Invalid email or password");
            }
            if status.is_server_error() {
                bail!("Server error ({status}). Try again later.");
            }
            bail!("Login failed ({status}): {body}");
        }
        resp.json().await.context("invalid login response")
    }

    /// Exchange a refresh token for a new access/refresh pair.
    pub async fn refresh(
        base_url: impl AsRef<str>,
        refresh_token: impl AsRef<str>,
    ) -> Result<LoginResponse> {
        let base = normalize_server_url(base_url.as_ref())?;
        let refresh = refresh_token.as_ref().trim();
        if refresh.is_empty() {
            bail!("Missing refresh token — sign in again");
        }
        let clients = build_http_clients(DEFAULT_HTTP_TIMEOUT)?;
        let url = format!("{base}/api/auth/refresh");
        let resp = send_preferring_h3(&clients, "POST", &url, |client, version| {
            client
                .post(&url)
                .version(version)
                .json(&serde_json::json!({ "refresh_token": refresh }))
        })
        .await
        .context("token refresh request failed")?;
        if !resp.status().is_success() {
            let status = resp.status();
            bail!("token refresh failed: {status}");
        }
        resp.json().await.context("invalid refresh response")
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        req.bearer_auth(&self.access_token)
    }

    async fn send_authed(
        &self,
        method: &'static str,
        url: &str,
        build: impl Fn(&Client, Version) -> reqwest::RequestBuilder,
    ) -> Result<Response> {
        let clients = self.clients();
        send_preferring_h3(&clients, method, url, |client, version| {
            self.auth(build(client, version))
        })
        .await
        .map_err(Into::into)
    }

    pub async fn list_storages(&self) -> Result<Vec<StorageSummary>> {
        self.require_access_token()?;
        let url = format!("{}/api/storages", self.base_url);
        let resp = self
            .send_authed("GET", &url, |client, version| {
                client.get(&url).version(version)
            })
            .await?
            .error_for_status()?;
        let body: StoragesResponse = resp.json().await.context("invalid storages response")?;
        Ok(body.storages)
    }

    pub async fn snapshot(&self, storage_id: Uuid) -> Result<SnapshotResponse> {
        let url = format!("{}/api/storages/{storage_id}/sync/snapshot", self.base_url);
        let resp = self
            .send_authed("GET", &url, |client, version| {
                client.get(&url).version(version)
            })
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn changelog(
        &self,
        storage_id: Uuid,
        cursor: i64,
        limit: i64,
    ) -> Result<ChangelogResponse> {
        let url = format!(
            "{}/api/storages/{storage_id}/sync/changelog?cursor={cursor}&limit={limit}",
            self.base_url
        );
        let resp = self
            .send_authed("GET", &url, |client, version| {
                client.get(&url).version(version)
            })
            .await?
            .error_for_status()?;
        Ok(resp.json().await?)
    }

    pub async fn download_to(
        &self,
        storage_id: Uuid,
        remote_path: &str,
        dest: &Path,
    ) -> Result<()> {
        let encoded = remote_path
            .split('/')
            .map(urlencoding_encode)
            .collect::<Vec<_>>()
            .join("/");
        let url = format!(
            "{}/api/storages/{storage_id}/files/download/{encoded}",
            self.base_url
        );
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // A 1-byte probe tells us the file size via `Content-Range` and
        // whether the server actually honors `Range`, without committing to
        // a full-body GET first. `download_file` on the server always
        // answers `206` for a non-empty file, but a proxy in front of it
        // might strip the header — treat a non-`206` reply as the whole
        // body having arrived already and write it straight through.
        let probe = self
            .send_authed("GET", &url, |client, version| {
                client
                    .get(&url)
                    .version(version)
                    .header(RANGE, "bytes=0-0")
                    .timeout(TRANSFER_MAX_TIMEOUT)
            })
            .await?
            .error_for_status()?;

        if probe.status() != StatusCode::PARTIAL_CONTENT {
            return self.write_response_atomically(probe, dest).await;
        }

        let total_len = probe
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|v| v.parse::<u64>().ok())
            .with_context(|| format!("server omitted Content-Range for {}", dest.display()))?;
        drop(probe);

        if total_len > MAX_DOWNLOAD_BYTES {
            bail!(
                "refusing download of {total_len} bytes for {}: over the {MAX_DOWNLOAD_BYTES}-byte limit",
                dest.display()
            );
        }

        if total_len < PARALLEL_DOWNLOAD_THRESHOLD_BYTES {
            // Size is known and small — one more plain GET is cheaper than
            // fanning out. `write_response_body`'s per-frame idle guard is what
            // catches a dead connection during the long Telegram round trip.
            let resp = self
                .send_authed("GET", &url, |client, version| {
                    client
                        .get(&url)
                        .version(version)
                        .timeout(TRANSFER_MAX_TIMEOUT)
                })
                .await?
                .error_for_status()?;
            return self.write_response_atomically(resp, dest).await;
        }

        self.download_parallel(&url, total_len, dest).await
    }

    /// Fans a large download out into `PARALLEL_DOWNLOAD_PARTS` concurrent
    /// `Range` GETs, each written to its own scratch file, then concatenates
    /// the parts in order into `dest`. The server's per-chunk cache
    /// deduplicates overlapping requests by Telegram chunk, not by byte
    /// range, so distinct Range windows land on distinct chunks and are
    /// fetched from Telegram at the same time instead of one-ahead.
    async fn download_parallel(&self, url: &str, total_len: u64, dest: &Path) -> Result<()> {
        let parts = PARALLEL_DOWNLOAD_PARTS.min(total_len.div_ceil(PARALLEL_DOWNLOAD_THRESHOLD_BYTES));
        let part_size = total_len.div_ceil(parts);
        let ranges: Vec<(u64, u64)> = (0..parts)
            .map(|i| {
                let start = i * part_size;
                let end = (start + part_size).min(total_len).saturating_sub(1);
                (start, end)
            })
            .filter(|&(start, end)| start <= end)
            .collect();

        let part_paths: Vec<PathBuf> = (0..ranges.len())
            .map(|idx| {
                dest.with_extension(format!("sarca-part-{idx}-{}", Uuid::new_v4().simple()))
            })
            .collect();

        let fetch_parts = ranges
            .iter()
            .copied()
            .zip(part_paths.iter().cloned())
            .map(|((start, end), part_path)| {
                let url = url.to_owned();
                async move {
                    let resp = self
                        .send_authed("GET", &url, |client, version| {
                            client
                                .get(&url)
                                .version(version)
                                .header(RANGE, format!("bytes={start}-{end}"))
                                .timeout(TRANSFER_MAX_TIMEOUT)
                        })
                        .await?
                        .error_for_status()?;
                    self.write_response_body(resp, &part_path).await
                }
            });

        let fetched = futures::stream::iter(fetch_parts)
            .buffer_unordered(part_paths.len().max(1))
            .try_collect::<Vec<()>>()
            .await;

        if let Err(e) = fetched {
            for p in &part_paths {
                let _ = tokio::fs::remove_file(p).await;
            }
            return Err(e);
        }

        // Byte ranges concatenate in order to reproduce the original file —
        // no container/codec is involved, so a plain streamed copy suffices.
        let tmp = dest.with_extension(format!("sarca-part-{}", Uuid::new_v4().simple()));
        let assemble = async {
            let mut out = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .await
                .with_context(|| format!("create {}", tmp.display()))?;
            for part_path in &part_paths {
                let mut part_file = tokio::fs::File::open(part_path)
                    .await
                    .with_context(|| format!("open {}", part_path.display()))?;
                tokio::io::copy(&mut part_file, &mut out).await?;
            }
            out.flush().await?;
            out.sync_all().await?;
            drop(out);
            // `rename` replaces a symlink at `dest` rather than following it.
            tokio::fs::rename(&tmp, dest)
                .await
                .with_context(|| format!("write {}", dest.display()))
        }
        .await;

        for p in &part_paths {
            let _ = tokio::fs::remove_file(p).await;
        }
        if assemble.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        assemble
    }

    /// Writes `resp`'s body straight into a freshly created file at `path`
    /// (no rename — callers wanting an atomic swap into a final destination
    /// use [`Self::write_response_atomically`]). Rejects an oversized body;
    /// `Content-Length` is only advisory, so the stream below re-checks as
    /// bytes arrive.
    async fn write_response_body(&self, resp: Response, path: &Path) -> Result<()> {
        if let Some(len) = resp.content_length() {
            if len > MAX_DOWNLOAD_BYTES {
                bail!(
                    "refusing download of {len} bytes for {}: over the {MAX_DOWNLOAD_BYTES}-byte limit",
                    path.display()
                );
            }
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await
            .with_context(|| format!("create {}", path.display()))?;
        let mut written: u64 = 0;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = next_frame_before_idle(&mut stream, "download").await? {
            written = written.saturating_add(chunk.len() as u64);
            if written > MAX_DOWNLOAD_BYTES {
                bail!(
                    "download of {} exceeded the {MAX_DOWNLOAD_BYTES}-byte limit",
                    path.display()
                );
            }
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok(())
    }

    /// Writes `resp`'s body to a sibling temp file and renames it into place
    /// at `dest`. Writing straight to `dest` follows a symlink that happens
    /// to sit there, which lets a compromised server overwrite any file the
    /// user can write (`~/.bashrc`, `~/.ssh/authorized_keys`) simply by
    /// naming a path whose local counterpart is a link. `create_new` (inside
    /// `write_response_body`) also means we never write through a link
    /// planted at the temp path itself.
    async fn write_response_atomically(&self, resp: Response, dest: &Path) -> Result<()> {
        let tmp = dest.with_extension(format!("sarca-part-{}", Uuid::new_v4().simple()));
        let result = match self.write_response_body(resp, &tmp).await {
            Ok(()) => tokio::fs::rename(&tmp, dest)
                .await
                .with_context(|| format!("write {}", dest.display())),
            Err(e) => Err(e),
        };
        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }

    pub async fn delete_remote(&self, storage_id: Uuid, remote_path: &str) -> Result<()> {
        let encoded = remote_path
            .split('/')
            .map(urlencoding_encode)
            .collect::<Vec<_>>()
            .join("/");
        let url = format!(
            "{}/api/storages/{storage_id}/files/{encoded}",
            self.base_url
        );
        let resp = self
            .send_authed("DELETE", &url, |client, version| {
                client.delete(&url).version(version)
            })
            .await?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            bail!("delete failed: {}", resp.status());
        }
        Ok(())
    }

    pub async fn upload_file(
        &self,
        storage_id: Uuid,
        parent_path: &str,
        filename: &str,
        local_path: &Path,
        mtime_ms: Option<i64>,
        content_hash: Option<&str>,
    ) -> Result<()> {
        self.require_access_token()?;
        let url = format!("{}/api/storages/{storage_id}/files/upload", self.base_url);
        let h3_version = preferred_request_version(&url);

        struct UploadParams<'a> {
            parent_path: &'a str,
            filename: &'a str,
            local_path: &'a Path,
            mtime_ms: Option<i64>,
            content_hash: Option<&'a str>,
        }

        async fn build_upload(
            api: &SarcaApi,
            client: &Client,
            url: &str,
            version: Version,
            params: &UploadParams<'_>,
        ) -> Result<(reqwest::RequestBuilder, Arc<UploadProgress>)> {
            let file = File::open(params.local_path).await?;
            let meta = file.metadata().await?;
            let progress = Arc::new(UploadProgress::new());
            let stream = instrumented_upload_body(ReaderStream::new(file), progress.clone());
            let body = reqwest::Body::wrap_stream(stream);
            let part = Part::stream_with_length(body, meta.len())
                .file_name(params.filename.to_owned())
                .mime_str("application/octet-stream")?;
            let mut form = Form::new()
                .text("path", params.parent_path.to_owned())
                .text("filename", params.filename.to_owned())
                .part("file", part);
            if let Some(ms) = params.mtime_ms {
                form = form.text("mtime", ms.to_string());
            }
            if let Some(hash) = params.content_hash {
                form = form.text("content_hash", hash.to_owned());
            }
            // Override the client-wide control-plane deadline: the server only
            // answers once the file is through Telegram, which is minutes, not
            // seconds. Without this the response body read died mid-stream with
            // "error decoding response body: operation timed out" and the file
            // was reported as failed even though the server kept going.
            let req = api
                .auth(client.post(url).version(version).multipart(form))
                .timeout(transfer_timeout(meta.len()));
            Ok((req, progress))
        }

        let h3_client = if h3_version == Version::HTTP_3 {
            self.h3_client()
        } else {
            &self.tcp_client
        };
        let params = UploadParams {
            parent_path,
            filename,
            local_path,
            mtime_ms,
            content_hash,
        };
        let (req, progress) = build_upload(self, h3_client, &url, h3_version, &params).await?;
        let resp = match send_upload(req, progress.clone()).await {
            Ok(resp) => {
                log_response_protocol("POST", &url, resp.version());
                resp
            }
            // Falling back re-sends the whole file, so only do it when HTTP/3
            // gave up before a single byte left the machine. Otherwise the
            // "fallback" is a silent second upload of a multi-gigabyte file —
            // and when the first attempt hit its deadline, a second one against
            // an already-expired budget.
            Err(UploadSendError::Transport(err))
                if should_fallback_from_h3(&url, h3_version, &err) && !progress.sent_any() =>
            {
                tracing::info!(
                    method = "POST",
                    url = %url,
                    error = %err,
                    "HTTP/3 upload failed before sending, falling back to TCP HTTPS"
                );
                log::info!("HTTP/3 upload failed, falling back to TCP HTTPS url={url} error={err}");
                let (req, progress) =
                    build_upload(self, &self.tcp_client, &url, Version::HTTP_11, &params).await?;
                let resp = send_upload(req, progress).await.map_err(|e| e.into_report())?;
                log_response_protocol("POST", &url, resp.version());
                resp
            }
            Err(e) => return Err(e.into_report()),
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(
                anyhow::Error::new(HttpStatusError { status, body }).context("upload failed")
            );
        }
        // Status is sent before Telegram delivery even starts — the real
        // outcome is a `phase` line in the streamed NDJSON body. That one stays
        // untyped, and so classifies as `FailureScope::File`: the server took
        // the bytes and reached a verdict about *this* file.
        if let Some(msg) = drain_upload_progress(resp).await? {
            bail!("upload failed: {msg}");
        }
        Ok(())
    }

    pub async fn create_folder(
        &self,
        storage_id: Uuid,
        parent: &str,
        folder_name: &str,
    ) -> Result<()> {
        self.require_access_token()?;
        let url = format!(
            "{}/api/storages/{storage_id}/files/create_folder",
            self.base_url
        );
        let body = serde_json::json!({
            "path": parent,
            "folder_name": folder_name,
        });
        let resp = self
            .send_authed("POST", &url, |client, version| {
                client.post(&url).version(version).json(&body)
            })
            .await?;
        if !resp.status().is_success() && resp.status().as_u16() != 409 {
            let status = resp.status();
            let detail = resp.text().await.unwrap_or_default();
            if detail.trim().is_empty() {
                bail!("create_folder failed: {status}");
            }
            bail!("create_folder failed: {status} {detail}");
        }
        Ok(())
    }
}

/// How far an upload's request body has got.
///
/// Lets the stall watchdog tell "the connection is not draining" from "the body
/// is up and the server is busy pushing it to Telegram", and lets the HTTP/3
/// fallback tell "nothing was sent, retrying is free" from "half a gigabyte is
/// already on the wire".
#[derive(Debug)]
struct UploadProgress {
    /// Bytes handed to the connection so far.
    sent: AtomicU64,
    /// Milliseconds since `started` at the last handover.
    last_ms: AtomicU64,
    /// Set once the final chunk of the file has been handed over.
    body_complete: AtomicBool,
    started: Instant,
    /// Silence that counts as wedged, and how often to check for it. Fields
    /// rather than constants so tests can watch a real stall resolve without
    /// waiting out the production budget.
    stall_after: Duration,
    stall_poll: Duration,
}

impl UploadProgress {
    fn new() -> Self {
        Self::with_stall_budget(UPLOAD_STALL_TIMEOUT, UPLOAD_STALL_POLL)
    }

    fn with_stall_budget(stall_after: Duration, stall_poll: Duration) -> Self {
        Self {
            sent: AtomicU64::new(0),
            last_ms: AtomicU64::new(0),
            body_complete: AtomicBool::new(false),
            started: Instant::now(),
            stall_after,
            stall_poll,
        }
    }

    fn note_sent(&self, bytes: usize) {
        self.sent.fetch_add(bytes as u64, Ordering::Relaxed);
        self.last_ms.store(self.elapsed_ms(), Ordering::Relaxed);
    }

    fn note_body_complete(&self) {
        self.body_complete.store(true, Ordering::Relaxed);
    }

    fn sent_any(&self) -> bool {
        self.sent.load(Ordering::Relaxed) > 0
    }

    fn body_complete(&self) -> bool {
        self.body_complete.load(Ordering::Relaxed)
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// How long the body has been silent. Before the first chunk this is the
    /// age of the request, which is what we want: a connection that never
    /// accepts a byte is as wedged as one that stops halfway.
    fn idle(&self) -> Duration {
        Duration::from_millis(
            self.elapsed_ms()
                .saturating_sub(self.last_ms.load(Ordering::Relaxed)),
        )
    }
}

/// Wraps an upload body so every chunk the connection takes is recorded, and
/// the end of the file is marked.
///
/// The trailing empty chunk exists only to give the stream somewhere to run
/// `note_body_complete` — it is polled exactly when the file hits EOF, which is
/// the moment silence stops meaning "wedged" and starts meaning "the server is
/// working". An empty frame is a no-op in the body.
fn instrumented_upload_body<S>(
    stream: S,
    progress: Arc<UploadProgress>,
) -> impl futures::Stream<Item = std::io::Result<bytes::Bytes>> + Send
where
    S: futures::Stream<Item = std::io::Result<bytes::Bytes>> + Send,
{
    let on_chunk = progress.clone();
    stream
        .inspect(move |chunk| {
            if let Ok(chunk) = chunk {
                on_chunk.note_sent(chunk.len());
            }
        })
        .chain(futures::stream::once(async move {
            progress.note_body_complete();
            Ok(bytes::Bytes::new())
        }))
}

/// Why an upload never got a response.
enum UploadSendError {
    Transport(reqwest::Error),
    /// The connection stopped accepting body bytes for this long.
    Stalled(Duration),
}

impl UploadSendError {
    fn into_report(self) -> anyhow::Error {
        match self {
            Self::Transport(e) => describe_transfer_error("upload", e),
            Self::Stalled(after) => anyhow::Error::new(UploadStalled(after)),
        }
    }
}

/// Resolves only once the request body has been silent for
/// [`UPLOAD_STALL_TIMEOUT`]; never resolves after the body is fully sent.
async fn upload_stalled(progress: Arc<UploadProgress>) -> Duration {
    loop {
        tokio::time::sleep(progress.stall_poll).await;
        if progress.body_complete() {
            // Everything is on the wire. What remains is the server pushing to
            // Telegram, bounded by the request's own `transfer_timeout`, and
            // then the NDJSON body, bounded by `READ_IDLE_TIMEOUT`.
            std::future::pending::<()>().await;
        }
        if progress.idle() >= progress.stall_after {
            return progress.stall_after;
        }
    }
}

/// Sends an upload, failing fast if the socket wedges mid-body.
async fn send_upload(
    req: reqwest::RequestBuilder,
    progress: Arc<UploadProgress>,
) -> std::result::Result<Response, UploadSendError> {
    tokio::select! {
        biased;
        sent = req.send() => sent.map_err(UploadSendError::Transport),
        after = upload_stalled(progress) => Err(UploadSendError::Stalled(after)),
    }
}

/// Pulls the next body frame, failing if the stream goes [`READ_IDLE_TIMEOUT`]
/// without one.
///
/// `Ok(None)` is a clean end of body.
async fn next_frame_before_idle<S>(stream: &mut S, action: &str) -> Result<Option<bytes::Bytes>>
where
    S: futures::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin,
{
    match tokio::time::timeout(READ_IDLE_TIMEOUT, stream.next()).await {
        Ok(Some(chunk)) => Ok(Some(chunk.map_err(|e| describe_transfer_error(action, e))?)),
        Ok(None) => Ok(None),
        Err(_) => Err(anyhow::anyhow!(
            "{action} stalled — the server sent nothing for {}s. It will be retried.",
            READ_IDLE_TIMEOUT.as_secs()
        )),
    }
}

/// Reads an upload's NDJSON progress stream to the end, returning the message
/// of the first `phase: "error"` line.
///
/// Scanned line by line rather than buffered whole: a long upload heartbeats
/// every 15s for as long as Telegram takes, and there is no reason to hold all
/// of that in memory to find one field.
async fn drain_upload_progress(resp: Response) -> Result<Option<String>> {
    let mut stream = resp.bytes_stream();
    let mut pending: Vec<u8> = Vec::new();
    let mut failure: Option<String> = None;
    while let Some(chunk) = next_frame_before_idle(&mut stream, "upload").await? {
        pending.extend_from_slice(&chunk);
        while let Some(nl) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=nl).collect();
            if failure.is_none() {
                failure = ndjson_error_message(&line);
            }
        }
        // A `phase: "error"` line is terminal, but keep draining so the
        // connection closes cleanly rather than being reset mid-response.
    }
    if failure.is_none() {
        failure = ndjson_error_message(&pending);
    }
    Ok(failure)
}

/// Turns a transport failure during a file transfer into something the Sync
/// panel can show. reqwest's own wording ("error decoding response body:
/// request or response body error: operation timed out") names the layer that
/// noticed, not what went wrong.
///
/// Both branches stay classifiable by [`failure_scope`]: the non-timeout one
/// keeps the `reqwest::Error` in the chain, and the timeout one — whose whole
/// point is to replace reqwest's wording — carries [`TransferTimedOut`] instead.
fn describe_transfer_error(action: &str, err: reqwest::Error) -> anyhow::Error {
    if err.is_timeout() {
        return anyhow::Error::new(TransferTimedOut(action.to_owned()));
    }
    anyhow::Error::from(err).context(format!("{action} failed"))
}

/// What a failed transfer says about the *next* file in the batch.
///
/// The upload retry ladder in `engine.rs` defers a file that fails, on the
/// theory that the file is the problem. That theory only holds when the server
/// actually formed an opinion about it. When the server is simply not there,
/// every file in the backlog "fails" in milliseconds, and a ladder applied to
/// all of them buries a whole queue for hours over an outage that lasted a
/// minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureScope {
    /// This file: the server took the request and refused the content, or the
    /// bytes could not be read off local disk. Nothing else is implicated.
    File,
    /// The link to the server: nothing arrived, the connection wedged, a
    /// gateway answered for a backend that is not there, or the session
    /// expired. Every other file is about to fail exactly the same way.
    Link,
}

/// A non-2xx answer, kept as a typed error so callers can classify by status
/// instead of re-parsing the message they just formatted. `Display` is bare
/// (`"502 Bad Gateway "`) because every construction site adds its own
/// `.context(...)` prefix.
#[derive(Debug)]
pub struct HttpStatusError {
    pub status: StatusCode,
    pub body: String,
}

impl std::fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.status, self.body)
    }
}

impl std::error::Error for HttpStatusError {}

/// The send side went silent for [`UPLOAD_STALL_TIMEOUT`]. Typed for the same
/// reason as [`HttpStatusError`]: a wedged socket is a link problem, and the
/// only way to know that from an `anyhow::Error` is to leave a type behind.
#[derive(Debug)]
struct UploadStalled(Duration);

impl std::fmt::Display for UploadStalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "upload stalled — the connection stopped accepting data for {}s. It will be retried.",
            self.0.as_secs()
        )
    }
}

impl std::error::Error for UploadStalled {}

/// A transfer that ran out its deadline. Typed for the same reason as
/// [`UploadStalled`]: [`describe_transfer_error`] deliberately drops reqwest's
/// own wording, and dropping the error with it would leave nothing to classify.
#[derive(Debug)]
struct TransferTimedOut(String);

impl std::fmt::Display for TransferTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} timed out — the server stopped responding. It will be retried.",
            self.0
        )
    }
}

impl std::error::Error for TransferTimedOut {}

/// Statuses that say nothing about the file that happened to be in flight.
fn scope_for_status(status: StatusCode) -> FailureScope {
    match status.as_u16() {
        // The session, not the file. Self-heals: the client refreshes the
        // access token before every tick.
        401 | 403 => FailureScope::Link,
        // Congestion and throttling — "come back later", for any file.
        408 | 425 | 429 => FailureScope::Link,
        // Sarca answers 4xx for everything it blames on the request itself, so
        // a 5xx is the server or the gateway in front of it: overloaded,
        // restarting, or gone. Caddy's empty-bodied 502 for a backend that is
        // not listening is the shape this whole classification exists for.
        s if s >= 500 => FailureScope::Link,
        _ => FailureScope::File,
    }
}

/// Classifies a failed transfer by walking the error chain for something that
/// positively identifies the link — a status code, a transport error, a wedged
/// socket.
///
/// Anything else is [`FailureScope::File`], deliberately: that is the
/// conservative answer, because it preserves head-of-line isolation, which is
/// the property the retry ladder exists for.
pub fn failure_scope(err: &anyhow::Error) -> FailureScope {
    for cause in err.chain() {
        if let Some(e) = cause.downcast_ref::<HttpStatusError>() {
            return scope_for_status(e.status);
        }
        if let Some(e) = cause.downcast_ref::<reqwest::Error>() {
            return match e.status() {
                Some(status) => scope_for_status(status),
                // No status means no answer: connection refused, TLS failure,
                // DNS, timeout, a reset mid-body.
                None => FailureScope::Link,
            };
        }
        if cause.is::<UploadStalled>() || cause.is::<TransferTimedOut>() {
            return FailureScope::Link;
        }
    }
    FailureScope::File
}

/// Pair of HTTP clients: QUIC/HTTP/3 (ALPN `h3`) and TCP HTTPS fallback.
#[derive(Clone)]
pub struct HttpClients {
    pub h3: Client,
    pub tcp: Client,
}

/// Common builder settings, including TOFU pinning when a pin store is installed.
///
/// reqwest routes a preconfigured rustls config to both the TCP connector and
/// the HTTP/3 connector, so the same verifier covers QUIC.
fn client_builder(timeout: Duration) -> reqwest::ClientBuilder {
    // No `.read_timeout()`: see [`READ_IDLE_TIMEOUT`]. reqwest applies it as a
    // deadline on the response head, which the upload endpoint cannot meet for
    // any file that takes longer than it to send. Body-idle detection is done
    // by the callers that actually read a body.
    let builder = Client::builder()
        .timeout(timeout)
        .connect_timeout(CONNECT_TIMEOUT);
    match crate::pinning::pinned_tls_config() {
        Some(config) => builder.use_preconfigured_tls(config),
        None => builder,
    }
}

fn build_tcp_client(timeout: Duration) -> Result<Client> {
    client_builder(timeout)
        .build()
        .context("failed to create TCP HTTP client")
}

/// How many same-host redirects the webview proxy resolves before giving up.
const MAX_PROXY_REDIRECTS: usize = 5;

/// Redirect policy for the webview proxy: follow a redirect that stays on the
/// upstream host, hand anything else to the webview.
///
/// A Sarca server fronted by Caddy or nginx answers plain HTTP with a 308 to
/// its HTTPS origin. Passing that straight through moved the webview off the
/// loopback origin the whole native bridge is keyed on — the Settings ACL, the
/// `sarca-ipc` protocol handler and the navigation fallback all check that one
/// origin — so Sync settings failed with "not allowed by ACL | Load failed |
/// Native bridge timeout". Resolving it here keeps the page on a single origin,
/// and the pinned TLS this client exists for covers the HTTPS leg the webview
/// could not validate itself.
///
/// A redirect to another host is still returned untouched (following it would
/// route third-party traffic through us), and HTTPS is never downgraded.
fn proxy_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        let Some(previous) = attempt.previous().last() else {
            return attempt.stop();
        };
        if attempt.previous().len() >= MAX_PROXY_REDIRECTS {
            return attempt.stop();
        }
        let same_host = attempt.url().host_str() == previous.host_str();
        let downgraded = previous.scheme() == "https" && attempt.url().scheme() != "https";
        if same_host && !downgraded {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

/// Pinned client for one-shot probes against the configured server.
///
/// Same trust and redirect handling as the webview proxy, plus an overall
/// timeout: a probe that hangs must not hold up connecting.
pub(crate) fn probe_http_client(timeout: Duration) -> Result<Client> {
    let builder = Client::builder()
        .timeout(timeout)
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(proxy_redirect_policy());
    let builder = match crate::pinning::pinned_tls_config() {
        Some(config) => builder.use_preconfigured_tls(config),
        None => builder,
    };
    builder
        .build()
        .context("failed to create server probe HTTP client")
}

/// Pinned client for the loopback webview proxy.
///
/// No overall timeout (a request may be a multi-gigabyte transfer). Redirects
/// that leave the upstream host are left for the proxy to rewrite `Location`
/// on; see [`proxy_redirect_policy`] for the ones resolved here.
pub(crate) fn proxy_http_client() -> Result<Client> {
    let builder = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .redirect(proxy_redirect_policy());
    let builder = match crate::pinning::pinned_tls_config() {
        Some(config) => builder.use_preconfigured_tls(config),
        None => builder,
    };
    builder
        .build()
        .context("failed to create webview proxy HTTP client")
}

fn build_h3_prior_client(timeout: Duration) -> Result<Client> {
    #[cfg(feature = "http3-client")]
    {
        client_builder(timeout)
            .http3_prior_knowledge()
            .build()
            .context("failed to create HTTP/3 client")
    }
    #[cfg(not(feature = "http3-client"))]
    {
        build_tcp_client(timeout)
    }
}

fn ensure_h3_client(slot: &OnceLock<Client>, timeout: Duration) -> Option<&Client> {
    if let Some(c) = slot.get() {
        return Some(c);
    }
    let client = build_h3_prior_client(timeout).ok()?;
    let _ = slot.set(client);
    slot.get()
}

/// Build HTTP clients used by sync API calls.
///
/// With the `http3-client` feature, the H3 client uses `http3_prior_knowledge()` so the
/// QUIC ClientHello advertises ALPN `h3`. A separate TCP client keeps `h2`/`http/1.1` ALPN
/// for fallback. H3 construction requires a tokio runtime (lazy in [`SarcaApi`]).
pub fn build_http_clients(timeout: Duration) -> Result<HttpClients> {
    let tcp = build_tcp_client(timeout)?;
    let h3 = build_h3_prior_client(timeout).unwrap_or_else(|_| tcp.clone());
    Ok(HttpClients { h3, tcp })
}

/// Request HTTP version for a URL when HTTP/3 preference is enabled.
pub fn preferred_request_version(url: &str) -> Version {
    if HTTP3_PREFERRED && url.starts_with("https://") {
        Version::HTTP_3
    } else {
        Version::HTTP_11
    }
}

fn should_fallback_from_h3(url: &str, attempted: Version, err: &reqwest::Error) -> bool {
    HTTP3_PREFERRED
        && url.starts_with("https://")
        && attempted == Version::HTTP_3
        && (err.is_connect() || err.is_timeout() || err.is_request())
}

fn format_http_version(version: Version) -> &'static str {
    match version {
        Version::HTTP_3 => "HTTP/3",
        Version::HTTP_2 => "HTTP/2",
        Version::HTTP_11 => "HTTP/1.1",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_09 => "HTTP/0.9",
        _ => "HTTP/?",
    }
}

fn log_response_protocol(method: &str, url: &str, version: Version) {
    let protocol = format_http_version(version);
    // info so Android logcat can prove HTTP/3 without enabling debug.
    tracing::info!(
        method = method,
        url = url,
        protocol = protocol,
        "sarca-sync HTTP response"
    );
    // `log` + android_logger bridge (client init) reaches adb logcat.
    log::info!("sarca-sync HTTP response method={method} url={url} protocol={protocol}");
}

async fn send_preferring_h3(
    clients: &HttpClients,
    method: &'static str,
    url: &str,
    build: impl Fn(&Client, Version) -> reqwest::RequestBuilder,
) -> Result<Response, reqwest::Error> {
    let preferred = preferred_request_version(url);
    if preferred == Version::HTTP_3 {
        match build(&clients.h3, Version::HTTP_3).send().await {
            Ok(resp) => {
                log_response_protocol(method, url, resp.version());
                return Ok(resp);
            }
            Err(err) if should_fallback_from_h3(url, Version::HTTP_3, &err) => {
                tracing::info!(
                    method = method,
                    url = url,
                    error = %err,
                    "HTTP/3 unavailable, falling back to TCP HTTPS"
                );
                log::info!(
                    "HTTP/3 unavailable, falling back to TCP HTTPS method={method} url={url} error={err}"
                );
            }
            Err(err) => return Err(err),
        }
    }

    let resp = build(&clients.tcp, Version::HTTP_11).send().await?;
    log_response_protocol(method, url, resp.version());
    Ok(resp)
}

/// Build `Authorization: Bearer …` value, or `None` when the token is empty.
pub fn authorization_header_value(access_token: &str) -> Option<String> {
    let token = access_token.trim();
    if token.is_empty() {
        None
    } else {
        Some(format!("Bearer {token}"))
    }
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

/// Scan an upload's streamed NDJSON body for a `phase: "error"` line and
/// return its message. Mirrors `handleUploadNdjsonLine` in `ui/src/api/request.js` —
/// the HTTP status is sent before Telegram delivery starts, so a `phase: error`
/// line mid-stream is the only signal the upload actually failed.
fn ndjson_error_message(body: &[u8]) -> Option<String> {
    for line in body.split(|&b| b == b'\n') {
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if ev.get("phase").and_then(|p| p.as_str()) == Some("error") {
            return Some(
                ev.get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Upload failed")
                    .to_owned(),
            );
        }
    }
    None
}

/// True when `host` can only be reached from the local machine or the local
/// network, so plaintext HTTP to it does not cross an untrusted path.
///
/// Everything else — a public hostname, a routable address — is assumed to be
/// reachable over the internet, where an implied `http://` would put the
/// session's bearer tokens on the wire in the clear for any on-path attacker.
fn is_local_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.eq_ignore_ascii_case("localhost")
        || bare.to_ascii_lowercase().ends_with(".localhost")
        || bare.to_ascii_lowercase().ends_with(".local")
        || bare.to_ascii_lowercase().ends_with(".internal")
        || bare.to_ascii_lowercase().ends_with(".home.arpa")
    {
        return true;
    }
    match bare.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        Ok(std::net::IpAddr::V6(v6)) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // fc00::/7 unique-local and fe80::/10 link-local.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// Normalize a Sarca server base URL.
///
/// Accepts `http://…`, `https://…`, or a host/IP without a scheme. A missing
/// scheme resolves to `https://` for anything routable and only falls back to
/// `http://` for loopback / LAN hosts, where self-hosted Sarca commonly runs
/// without TLS. Typing `sarca.example.com` must not silently downgrade the
/// connection that carries the access and refresh tokens; an explicit
/// `http://sarca.example.com` still works for users who mean it.
pub fn normalize_server_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("Server URL is required");
    }
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        // Parse once against a placeholder scheme just to isolate the host.
        let host_only = reqwest::Url::parse(&format!("http://{trimmed}"))
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        if is_local_host(&host_only) {
            format!("http://{trimmed}")
        } else {
            format!("https://{trimmed}")
        }
    };
    let parsed = reqwest::Url::parse(&with_scheme).map_err(|_| {
        anyhow::anyhow!("Invalid server URL. Use http:// or https:// and a valid host.")
    })?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => bail!("Unsupported URL scheme '{other}'. Use http:// or https://."),
    }
    if parsed.host_str().is_none() {
        bail!("Invalid server URL — missing host.");
    }
    // Drop path/query; API client always appends `/api/...`.
    let mut out = format!(
        "{}://{}",
        parsed.scheme(),
        parsed.host_str().unwrap_or_default()
    );
    if let Some(port) = parsed.port() {
        out.push(':');
        out.push_str(&port.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn transfer_timeout_never_drops_to_the_control_plane_deadline() {
        // The bug this guards: a 200 KB photo failed with "operation timed
        // out" because the whole request shared the 20s control-plane budget
        // while the server was still handing the file to Telegram.
        assert_eq!(transfer_timeout(0), TRANSFER_MIN_TIMEOUT);
        assert_eq!(transfer_timeout(209_800), TRANSFER_MIN_TIMEOUT);
        assert!(transfer_timeout(u64::MAX) <= TRANSFER_MAX_TIMEOUT);
    }

    #[test]
    fn transfer_timeout_grows_with_payload() {
        // 512 MiB at the assumed floor throughput sits between the two bounds.
        let bytes = 512 * 1024 * 1024;
        assert!(transfer_timeout(bytes) > TRANSFER_MIN_TIMEOUT);
        assert_eq!(
            transfer_timeout(bytes),
            Duration::from_secs(bytes / TRANSFER_MIN_BYTES_PER_SEC)
        );
        // Past the cap it stops growing — a wedged transfer cannot hold a sync
        // slot indefinitely.
        assert_eq!(transfer_timeout(64 * bytes), TRANSFER_MAX_TIMEOUT);
    }

    /// Answers a POST only after the whole request body has arrived and
    /// `head_delay` has passed — the shape of the real upload endpoint, which
    /// spools the multipart to disk and only then starts the NDJSON stream.
    async fn slow_upload_endpoint(head_delay: Duration, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    // Drain the request until the client stops sending. The
                    // multipart bodies in these tests are small enough to
                    // arrive in a couple of reads.
                    let mut buf = vec![0u8; 64 * 1024];
                    loop {
                        match tokio::time::timeout(
                            Duration::from_millis(150),
                            sock.read(&mut buf),
                        )
                        .await
                        {
                            Ok(Ok(0)) | Err(_) => break,
                            Ok(Ok(_)) => continue,
                            Ok(Err(_)) => return,
                        }
                    }
                    tokio::time::sleep(head_delay).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(body.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    /// Why `client_builder` must not set `read_timeout`.
    ///
    /// It reads like "longest silence between two body reads" and is not:
    /// reqwest arms one sleep at dispatch, never resets it, and fails the whole
    /// request if the response *head* has not arrived. Against an endpoint that
    /// answers only after the body is spooled, that is a hard ceiling on
    /// upload duration — which is what killed every large file with
    /// "error sending request for url (…): operation timed out".
    #[tokio::test]
    async fn reqwest_read_timeout_is_a_deadline_on_the_response_head() {
        let base = slow_upload_endpoint(Duration::from_millis(900), "{}\n").await;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_millis(200))
            .build()
            .unwrap();

        let err = client
            .post(format!("{base}/api/upload"))
            .body("x".repeat(4096))
            .send()
            .await
            .expect_err("read_timeout fires before the head arrives");

        assert!(err.is_timeout(), "expected a timeout, got: {err}");
        // Same wording the failing uploads recorded in `upload_failures`.
        assert!(
            err.to_string().contains("error sending request"),
            "got: {err}"
        );
    }

    /// The fix: transfer clients carry no head-phase read deadline, so an
    /// upload is bounded only by its own `transfer_timeout`.
    #[tokio::test]
    async fn transfer_clients_have_no_response_head_deadline() {
        let client = build_tcp_client(DEFAULT_HTTP_TIMEOUT).unwrap();
        assert!(
            !format!("{client:?}").contains("read_timeout"),
            "a client-wide read_timeout caps every upload at that many seconds \
             regardless of file size: {client:?}"
        );

        // And it really does outlive one: same endpoint as the test above,
        // answering well after the body is in.
        let base = slow_upload_endpoint(Duration::from_millis(900), "{}\n").await;
        let resp = client
            .post(format!("{base}/api/upload"))
            .body("x".repeat(4096))
            .send()
            .await
            .expect("a slow head must not fail a healthy upload");
        assert!(resp.status().is_success());
    }

    #[tokio::test]
    async fn upload_body_records_progress_and_marks_completion() {
        let progress = Arc::new(UploadProgress::new());
        assert!(!progress.sent_any());
        assert!(!progress.body_complete());

        let chunks = futures::stream::iter(vec![
            Ok(bytes::Bytes::from_static(b"hello")),
            Ok(bytes::Bytes::from_static(b" world")),
        ]);
        let mut body = Box::pin(instrumented_upload_body(chunks, progress.clone()));

        assert_eq!(body.next().await.unwrap().unwrap().len(), 5);
        assert!(progress.sent_any(), "first chunk must count as progress");
        assert!(
            !progress.body_complete(),
            "the file is not finished after one chunk"
        );

        body.next().await.unwrap().unwrap();
        // The trailing empty frame is what marks the end of the file.
        assert!(body.next().await.unwrap().unwrap().is_empty());
        assert!(
            progress.body_complete(),
            "EOF must flip the watchdog off so a slow Telegram push is not \
             mistaken for a wedged socket"
        );
    }

    #[tokio::test]
    async fn a_stalled_upload_fails_instead_of_holding_the_slot() {
        // A server that accepts the connection and then never reads or
        // answers. `send_upload` must give up on the stall watchdog rather
        // than sit on the request's multi-hour transfer deadline.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let held = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
            drop(sock);
        });

        // Same watchdog as production, on a budget a test can wait out.
        let progress = Arc::new(UploadProgress::with_stall_budget(
            Duration::from_millis(300),
            Duration::from_millis(50),
        ));

        let client = Client::builder().build().unwrap();
        let req = client
            .post(format!("http://{addr}/api/upload"))
            .timeout(TRANSFER_MAX_TIMEOUT)
            .body("x".repeat(4096));

        let Err(err) = tokio::time::timeout(Duration::from_secs(10), send_upload(req, progress))
            .await
            .expect("the watchdog must resolve, not wait out transfer_timeout")
        else {
            panic!("a wedged socket is not a successful upload");
        };

        assert!(
            matches!(err, UploadSendError::Stalled(_)),
            "expected a stall, not a transport error"
        );
        assert!(err.into_report().to_string().contains("upload stalled"));
        held.abort();
    }

    #[tokio::test]
    async fn ndjson_error_is_found_across_chunk_boundaries() {
        // The progress stream arrives in arbitrary TCP-sized pieces, so a
        // `phase: "error"` line is routinely split. Buffering the whole body
        // hid that; line reassembly must not.
        let body = "{\"phase\":\"heartbeat\"}\n\
                    {\"phase\":\"error\",\"message\":\"Telegram flood wait\"}\n";
        let base = slow_upload_endpoint(Duration::from_millis(10), body).await;
        let resp = Client::builder()
            .build()
            .unwrap()
            .post(format!("{base}/api/upload"))
            .body("x")
            .send()
            .await
            .unwrap();

        assert_eq!(
            drain_upload_progress(resp).await.unwrap().as_deref(),
            Some("Telegram flood wait")
        );
    }

    #[tokio::test]
    async fn a_clean_progress_stream_reports_no_failure() {
        let body = "{\"phase\":\"spooled\"}\n{\"phase\":\"done\"}\n";
        let base = slow_upload_endpoint(Duration::from_millis(10), body).await;
        let resp = Client::builder()
            .build()
            .unwrap()
            .post(format!("{base}/api/upload"))
            .body("x")
            .send()
            .await
            .unwrap();

        assert_eq!(drain_upload_progress(resp).await.unwrap(), None);
    }

    #[tokio::test]
    async fn timeout_errors_are_described_in_plain_words() {
        // A server that accepts the connection and then says nothing — the
        // shape of the failure users hit while Telegram delivery hangs.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accepted = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
            drop(sock);
        });

        let client = Client::builder()
            .timeout(Duration::from_millis(150))
            .build()
            .unwrap();
        let err = client
            .get(format!("http://{addr}/"))
            .send()
            .await
            .expect_err("server never answers");
        assert!(err.is_timeout(), "expected a timeout, got: {err}");

        let described = describe_transfer_error("upload", err).to_string();
        assert!(described.contains("upload timed out"), "got: {described}");
        assert!(
            !described.contains("decoding response body"),
            "reqwest's wording leaked into the UI message: {described}"
        );
        accepted.abort();
    }

    #[test]
    fn http3_preference_enabled_in_default_build() {
        const {
            assert!(
                HTTP3_PREFERRED,
                "default build should compile HTTP/3 preference (http3-client + reqwest_unstable)"
            );
        }
    }

    #[test]
    fn preferred_request_version_selects_h3_for_https() {
        assert_eq!(
            preferred_request_version("https://sarca.example.com"),
            Version::HTTP_3
        );
        assert_eq!(
            preferred_request_version("http://127.0.0.1:8001"),
            Version::HTTP_11
        );
    }

    #[test]
    fn build_http_client_succeeds_with_h3_config() {
        build_http_clients(Duration::from_secs(5)).expect("HTTP client builder should succeed");
    }

    #[test]
    fn normalizes_bare_lan_host_to_http() {
        for raw in [
            "192.168.1.40:8001",
            "10.0.0.5",
            "172.16.4.4:8001",
            "127.0.0.1:8001",
            "localhost:8001",
            "sarca.local",
        ] {
            let got = normalize_server_url(raw).unwrap();
            assert!(got.starts_with("http://"), "{raw} -> {got}");
        }
    }

    #[test]
    fn bare_public_host_defaults_to_https_not_http() {
        // A missing scheme must never downgrade a routable host: the base URL
        // carries the access and refresh tokens on every request.
        assert_eq!(
            normalize_server_url("sarca.example.com").unwrap(),
            "https://sarca.example.com"
        );
        assert_eq!(
            normalize_server_url("sarca.example.com:8443").unwrap(),
            "https://sarca.example.com:8443"
        );
        assert_eq!(
            normalize_server_url("203.0.113.10").unwrap(),
            "https://203.0.113.10"
        );
    }

    #[test]
    fn explicit_http_is_still_honoured() {
        assert_eq!(
            normalize_server_url("http://sarca.example.com").unwrap(),
            "http://sarca.example.com"
        );
    }

    #[test]
    fn keeps_https() {
        assert_eq!(
            normalize_server_url("https://sarca.example.com/").unwrap(),
            "https://sarca.example.com"
        );
    }

    #[test]
    fn rejects_bad_scheme() {
        assert!(normalize_server_url("ftp://x").is_err());
    }

    #[test]
    fn authorization_header_present_when_token_set() {
        assert_eq!(
            authorization_header_value("abc.def.ghi").as_deref(),
            Some("Bearer abc.def.ghi")
        );
        let api = SarcaApi::new("http://127.0.0.1:9", "tok-123");
        assert_eq!(
            api.authorization_header().as_deref(),
            Some("Bearer tok-123")
        );
    }

    #[test]
    fn authorization_header_none_when_token_missing() {
        assert_eq!(authorization_header_value(""), None);
        assert_eq!(authorization_header_value("   "), None);
        let api = SarcaApi::new("http://127.0.0.1:9", "");
        assert_eq!(api.authorization_header(), None);
    }

    #[tokio::test]
    async fn create_folder_fails_clearly_without_access_token() {
        let api = SarcaApi::new("http://127.0.0.1:9", "  ");
        let err = api
            .create_folder(Uuid::nil(), "", "Camera")
            .await
            .expect_err("empty token must fail before HTTP");
        let msg = err.to_string();
        assert!(
            msg.contains("access token") || msg.contains("Not authenticated"),
            "unexpected error: {msg}"
        );
    }

    #[tokio::test]
    async fn create_folder_sends_authorization_bearer_header() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = tx.send(req);
            let _ = sock
                .write_all(
                    b"HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        });

        let api = SarcaApi::new(format!("http://{addr}"), "test-access-token");
        api.create_folder(Uuid::nil(), "", "Camera")
            .await
            .expect("409 Conflict is treated as success (folder exists)");

        let req = rx.await.expect("server must receive request");
        assert!(
            req.to_ascii_lowercase()
                .contains("authorization: bearer test-access-token"),
            "Authorization header missing in request:\n{req}"
        );
        assert!(
            req.contains("/files/create_folder"),
            "wrong path in request:\n{req}"
        );
    }

    /// A compromised server names a file whose local counterpart happens to be
    /// a symlink pointing outside the sync root. Writing straight to that path
    /// would overwrite the link target (`~/.bashrc`, `~/.ssh/authorized_keys`).
    #[tokio::test]
    #[cfg(unix)]
    async fn download_to_replaces_a_symlink_instead_of_writing_through_it() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = sock.read(&mut buf).await;
            let _ = sock
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nOWNED",
                )
                .await;
        });

        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("bashrc");
        std::fs::write(&outside, b"ORIGINAL").unwrap();
        let dest = dir.path().join("root").join("innocent.jpg");
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&outside, &dest).unwrap();

        let api = SarcaApi::new(format!("http://{addr}"), "t");
        api.download_to(Uuid::nil(), "innocent.jpg", &dest)
            .await
            .expect("download must succeed");

        assert_eq!(
            std::fs::read(&outside).unwrap(),
            b"ORIGINAL",
            "the symlink target outside the root must be untouched"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), b"OWNED");
        assert!(
            !std::fs::symlink_metadata(&dest).unwrap().is_symlink(),
            "the link must have been replaced by a real file"
        );
    }

    /// A file at (or above) `PARALLEL_DOWNLOAD_THRESHOLD_BYTES` must fan out
    /// into concurrent `Range` GETs and reassemble them in byte order, with
    /// no scratch part files left behind.
    #[tokio::test]
    async fn download_to_parallel_range_fetch_reassembles_bytes_in_order() {
        const TOTAL: usize = PARALLEL_DOWNLOAD_THRESHOLD_BYTES as usize;
        let expected: Vec<u8> = (0..TOTAL).map(|i| (i % 251) as u8).collect();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_data = expected.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let data = server_data.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let range = req
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("range:"))
                        .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_owned()));
                    let (start, end) = range
                        .as_deref()
                        .and_then(|v| v.strip_prefix("bytes="))
                        .and_then(|v| v.split_once('-'))
                        .map(|(s, e)| (s.parse::<usize>().unwrap(), e.parse::<usize>().unwrap()))
                        .unwrap_or((0, data.len() - 1));
                    let end = end.min(data.len() - 1);
                    let slice = &data[start..=end];
                    let header = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len(),
                        slice.len()
                    );
                    let _ = sock.write_all(header.as_bytes()).await;
                    let _ = sock.write_all(slice).await;
                });
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("big.bin");

        let api = SarcaApi::new(format!("http://{addr}"), "t");
        api.download_to(Uuid::nil(), "big.bin", &dest)
            .await
            .expect("parallel download must succeed");

        let got = tokio::fs::read(&dest).await.unwrap();
        assert_eq!(
            got, expected,
            "parts must reassemble in the original byte order"
        );

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != "big.bin")
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp part files must be cleaned up: {leftovers:?}"
        );
    }

    #[test]
    fn failure_scope_blames_the_link_for_a_gateway_that_has_no_backend() {
        // The 502 storm: Caddy answering for a sarca that was restarting. The
        // file in flight had nothing to do with it, and 130 files behind it
        // even less.
        for status in [502u16, 503, 504, 500, 507] {
            let err = anyhow::Error::new(HttpStatusError {
                status: StatusCode::from_u16(status).unwrap(),
                body: String::new(),
            })
            .context("upload failed");
            assert_eq!(failure_scope(&err), FailureScope::Link, "status {status}");
        }
    }

    #[test]
    fn failure_scope_blames_the_link_for_an_expired_session() {
        // An access token lives 30 minutes; a batch of multi-gigabyte videos
        // does not finish in 30 minutes. Deferring files over it would punish
        // the backlog for the clock.
        for status in [401u16, 403, 408, 429] {
            let err = anyhow::Error::new(HttpStatusError {
                status: StatusCode::from_u16(status).unwrap(),
                body: String::new(),
            })
            .context("upload failed");
            assert_eq!(failure_scope(&err), FailureScope::Link, "status {status}");
        }
    }

    #[test]
    fn failure_scope_blames_the_file_when_the_server_refused_it() {
        for status in [400u16, 404, 409, 413, 422] {
            let err = anyhow::Error::new(HttpStatusError {
                status: StatusCode::from_u16(status).unwrap(),
                body: "no".into(),
            })
            .context("upload failed");
            assert_eq!(failure_scope(&err), FailureScope::File, "status {status}");
        }
        // Nothing typed in the chain at all — e.g. the NDJSON `error` phase, or
        // a local read failure. Unknown has to mean "the file", so head-of-line
        // isolation still holds.
        assert_eq!(
            failure_scope(&anyhow::anyhow!("upload failed: [Telegram API] file is too big")),
            FailureScope::File
        );
    }

    #[test]
    fn failure_scope_blames_the_link_for_a_wedged_socket() {
        assert_eq!(
            failure_scope(&UploadSendError::Stalled(Duration::from_secs(120)).into_report()),
            FailureScope::Link
        );
    }

    #[tokio::test]
    async fn failure_scope_blames_the_link_when_the_request_never_landed() {
        // Port 9 ("discard") is never bound in test environments, so this is a
        // connection refused: no status, no answer, no verdict about the file.
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, b"test").await.unwrap();
        let api = SarcaApi::new("http://127.0.0.1:9", "t");
        let err = api
            .upload_file(Uuid::nil(), "", "test.txt", &file_path, None, None)
            .await
            .expect_err("an unreachable server must fail the upload");
        assert_eq!(failure_scope(&err), FailureScope::Link);
    }

    #[test]
    fn a_transfer_timeout_is_a_link_failure_and_keeps_its_wording() {
        let err = anyhow::Error::new(TransferTimedOut("upload".to_owned()));
        assert_eq!(
            format!("{err:#}"),
            "upload timed out — the server stopped responding. It will be retried."
        );
        assert_eq!(failure_scope(&err), FailureScope::Link);
    }

    #[test]
    fn upload_stall_message_is_unchanged_by_being_typed() {
        // The Sync panel shows this string; typing the error was for the retry
        // logic, not a rewording.
        assert_eq!(
            format!(
                "{:#}",
                UploadSendError::Stalled(Duration::from_secs(120)).into_report()
            ),
            "upload stalled — the connection stopped accepting data for 120s. It will be retried."
        );
    }

    /// Server sends `201` before Telegram delivery even starts, then streams
    /// NDJSON progress; a mid-stream `phase: error` line means the upload
    /// never actually landed even though the HTTP status was success.
    #[tokio::test]
    async fn upload_file_fails_on_ndjson_error_phase_despite_201_status() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 65536];
            // Drain the (chunked-encoded, multi-write) multipart request body
            // before responding: keep reading until the stream goes idle
            // rather than assuming one short read means "done".
            loop {
                match tokio::time::timeout(
                    std::time::Duration::from_millis(200),
                    sock.read(&mut buf),
                )
                .await
                {
                    Ok(Ok(0) | Err(_)) | Err(_) => break,
                    Ok(Ok(_)) => {}
                }
            }
            let body = concat!(
                "{\"phase\":\"spooled\",\"uploaded\":0,\"total\":4}\n",
                "{\"phase\":\"telegram\",\"uploaded\":0,\"total\":4}\n",
                "{\"message\":\"[Telegram API] 401 Unauthorized\",\"phase\":\"error\"}\n",
            );
            let resp = format!(
                "HTTP/1.1 201 Created\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        tokio::fs::write(&file_path, b"test").await.unwrap();

        let api = SarcaApi::new(format!("http://{addr}"), "test-access-token");
        let err = api
            .upload_file(Uuid::nil(), "", "test.txt", &file_path, None, None)
            .await
            .expect_err("NDJSON error phase must surface as Err, not silent Ok");
        assert!(
            err.to_string().contains("401 Unauthorized"),
            "unexpected error: {err}"
        );
    }
}
