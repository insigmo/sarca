use tokio::sync::{mpsc, oneshot};
use std::path::PathBuf;
use uuid::Uuid;

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
