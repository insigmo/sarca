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

/// Push an upload progress event without ever blocking the Storage Manager.
///
/// A full channel means the HTTP client is slow/stuck draining NDJSON — drop the
/// event rather than stall Telegram (blocking here deadlocks the serial SM queue
/// and holds the per-token send permit forever). A closed channel means the
/// client canceled; return an error so the upload aborts promptly.
pub fn emit_upload_progress(
    tx: &mpsc::Sender<UploadProgressEvent>,
    ev: UploadProgressEvent,
) -> SarcaResult<()> {
    if tx.is_closed() {
        return Err(crate::errors::SarcaError::TelegramAPIError("Upload canceled".to_owned()));
    }
    match tx.try_send(ev) {
        Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
        Err(mpsc::error::TrySendError::Closed(_)) => {
            Err(crate::errors::SarcaError::TelegramAPIError("Upload canceled".to_owned()))
        },
    }
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
