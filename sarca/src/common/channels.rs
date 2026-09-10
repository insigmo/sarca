use std::path::PathBuf;

use serde::Serialize;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::errors::SarcaResult;

//////////////////////////////////////
//      Client schemas
//////////////////////////////////////
pub struct ClientMessage {
    pub tx: StorageManagerSender,
    pub data: ClientData,
}

pub enum ClientData {
    UploadFile(UploadFileData),
}

pub struct UploadFileData {
    pub file_id: Uuid,
    pub file_path: PathBuf,
    pub file_size: i64,
    /// Telegram document chunk size for this upload (bytes).
    pub chunk_size: usize,
    /// Optional live progress toward Telegram (bytes within the whole file).
    pub progress: Option<mpsc::Sender<UploadProgressEvent>>,
    /// Grid thumbnail (JPEG) built by the uploading client, if it sent one.
    pub client_thumb: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UploadProgressEvent {
    pub phase: &'static str,
    pub uploaded: u64,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks: Option<u32>,
    /// Seconds Telegram asked us to wait (flood control); present when `phase == "waiting"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

impl UploadProgressEvent {
    /// Spool + DB row ready; Telegram upload has not started yet.
    /// Clients may start the next file's client→Sarca transfer on this event.
    pub fn spooled(total: u64) -> Self {
        Self {
            phase: "spooled",
            uploaded: 0,
            total,
            chunk: None,
            chunks: None,
            retry_after: None,
        }
    }

    pub fn telegram(uploaded: u64, total: u64, chunk: u32, chunks: u32) -> Self {
        Self {
            phase: "telegram",
            uploaded,
            total,
            chunk: Some(chunk),
            chunks: Some(chunks),
            retry_after: None,
        }
    }

    pub fn waiting(uploaded: u64, total: u64, chunk: u32, chunks: u32, retry_after: u64) -> Self {
        Self {
            phase: "waiting",
            uploaded,
            total,
            chunk: Some(chunk),
            chunks: Some(chunks),
            retry_after: Some(retry_after),
        }
    }

    /// Keep-alive while Telegram is quiet (flood sleep / SM queue). Proxies and
    /// browsers often idle-timeout the NDJSON response without these.
    pub fn heartbeat() -> Self {
        Self {
            phase: "heartbeat",
            uploaded: 0,
            total: 0,
            chunk: None,
            chunks: None,
            retry_after: None,
        }
    }
}

/// Push an upload progress event, without ever blocking the Storage Manager and
/// without ever failing the upload.
///
/// Reporting only. A full channel means the HTTP client is slow at draining
/// NDJSON, and a closed one means nobody is reading any more — neither says
/// anything about whether the file should be stored, so both drop the event and
/// carry on.
///
/// This used to treat a closed channel as a cancellation, which made the
/// client's connection the upload's lifeline. That cannot hold: by the time any
/// progress is emitted the bytes are already spooled and the file row already
/// exists, and relaying them onward takes as long as the server's uplink needs —
/// hours for a large video, far longer than one HTTP request survives. So every
/// dropped connection abandoned a committed upload, left an unfinished row
/// behind, and sent the client back to re-upload the whole file, which is how a
/// big file could fail forever.
///
/// The trade-off is deliberate: an upload can no longer be called off by hanging
/// up on it. Once the server has the bytes it finishes with them, and getting
/// rid of the result means deleting the file.
pub fn emit_upload_progress(tx: &mpsc::Sender<UploadProgressEvent>, ev: UploadProgressEvent) {
    // Full or closed: either way there is nothing useful to do with the event.
    let _ = tx.try_send(ev);
}

//////////////////////////////////////
//      Storage manager schemas
//////////////////////////////////////
pub struct StorageManagerMessage {
    pub data: StorageManagerData,
}

impl StorageManagerMessage {
    pub fn new(data: StorageManagerData) -> Self {
        Self {
            data,
        }
    }
}

pub enum StorageManagerData {
    UploadFile(SarcaResult<()>),
}

//////////////////////////////////////
//      Channels
//////////////////////////////////////
pub type StorageManagerSender = oneshot::Sender<StorageManagerMessage>;
pub type ClientSender = mpsc::Sender<ClientMessage>;
pub type StorageManagerListener = mpsc::Receiver<ClientMessage>;
