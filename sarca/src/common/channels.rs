use tokio::sync::{mpsc, oneshot};
use std::path::PathBuf;
use uuid::Uuid;
use serde::Serialize;

use crate::errors::SarcaResult;

//////////////////////////////////////
///     Client schemas
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
    /// Optional live progress toward Telegram (bytes within the whole file).
    pub progress: Option<mpsc::Sender<UploadProgressEvent>>,
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
}

impl UploadProgressEvent {
    pub fn telegram(uploaded: u64, total: u64, chunk: u32, chunks: u32) -> Self {
        Self {
            phase: "telegram",
            uploaded,
            total,
            chunk: Some(chunk),
            chunks: Some(chunks),
        }
    }
}

//////////////////////////////////////
///     Storage manager schemas
//////////////////////////////////////

pub struct StorageManagerMessage {
    pub data: StorageManagerData,
}

impl StorageManagerMessage {
    pub fn new(data: StorageManagerData) -> Self {
        Self { data }
    }
}

pub enum StorageManagerData {
    UploadFile(SarcaResult<()>),
}

//////////////////////////////////////
///     Channels
//////////////////////////////////////

pub type StorageManagerSender = oneshot::Sender<StorageManagerMessage>;
pub type ClientSender = mpsc::Sender<ClientMessage>;
pub type StorageManagerListener = mpsc::Receiver<ClientMessage>;
