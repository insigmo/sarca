use axum::http::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SarcaError {
    #[error("environment variable `{0}` is not set")]
    EnvConfigLoadingError(String),
    #[error("environment variable `{0}` cannot be parsed")]
    EnvVarParsingError(String),

    #[error("user was removed")]
    UserWasRemoved,

    #[error("{0} already exists")]
    AlreadyExists(String),
    #[error("{0} does not exist")]
    DoesNotExist(String),
    #[error("User already has a storage with such name")]
    StorageNameConflict,
    #[error("This chat is already used by another channel")]
    StorageChatIdConflict,
    #[error("User already has a storage worker with such name")]
    StorageWorkerNameConflict,
    #[error("Token must be unique")]
    StorageWorkerTokenConflict,
    #[error("not authenticated")]
    NotAuthenticated,
    #[error("[Telegram API] {0}")]
    TelegramAPIError(String),
    #[error("You need to add at least 1 storage worker")]
    NoStorageWorkers,
    #[error("Invalid path")]
    InvalidPath,
    #[error("Folder is larger than 10 GB. Download files in smaller pieces.")]
    FolderTooLargeForZip,
    #[error("Invalid folder name")]
    InvalidFolderName,
    #[error("You cannot manage access of yourself")]
    CannotManageAccessOfYourself,
    #[error("Storage does not have workers")]
    StorageDoesNotHaveWorkers,
    #[error("A storage can have at most 3 channels")]
    TooManyChannels,
    #[error("Cannot remove the last active channel")]
    LastActiveChannel,
    #[error("Storage has no active channel available")]
    NoActiveChannel,
    #[error("A file already exists at this path")]
    TrashPathConflict,
    #[error("Invalid trash retention days (must be 1–30)")]
    InvalidTrashRetention,
    #[error("Share expiry must be in the future")]
    InvalidShareExpiry,
    #[error("mail not configured")]
    MailNotConfigured,
    #[error("invalid or expired token")]
    InvalidToken,
    #[error("OAuth provider is not configured")]
    OAuthNotConfigured,
    #[error("OAuth failed")]
    OAuthFailed,
    #[error("unknown error")]
    Unknown,
    #[error("{0} header is required")]
    HeaderMissed(String),
    #[error("{0} header should be a valid {1}")]
    HeaderIsInvalid(String, String),
}

impl From<SarcaError> for (StatusCode, String) {
    fn from(e: SarcaError) -> Self {
        match &e {
            SarcaError::AlreadyExists(_)
            | SarcaError::StorageNameConflict
            | SarcaError::StorageChatIdConflict
            | SarcaError::StorageWorkerNameConflict
            | SarcaError::StorageWorkerTokenConflict
            | SarcaError::StorageDoesNotHaveWorkers
            | SarcaError::TooManyChannels
            | SarcaError::LastActiveChannel
            | SarcaError::CannotManageAccessOfYourself
            | SarcaError::TrashPathConflict => (StatusCode::CONFLICT, e.to_string()),
            SarcaError::NotAuthenticated => (StatusCode::UNAUTHORIZED, e.to_string()),
            SarcaError::DoesNotExist(_) => (StatusCode::NOT_FOUND, e.to_string()),
            SarcaError::FolderTooLargeForZip => {
                (StatusCode::PAYLOAD_TOO_LARGE, e.to_string())
            }
            SarcaError::HeaderMissed(_)
            | SarcaError::HeaderIsInvalid(..)
            | SarcaError::InvalidFolderName
            | SarcaError::InvalidPath
            | SarcaError::NoStorageWorkers
            | SarcaError::NoActiveChannel
            | SarcaError::InvalidTrashRetention
            | SarcaError::InvalidShareExpiry
            | SarcaError::InvalidToken
            | SarcaError::OAuthNotConfigured
            | SarcaError::OAuthFailed
            | SarcaError::TelegramAPIError(_) => (StatusCode::BAD_REQUEST, e.to_string()),
            SarcaError::MailNotConfigured => (StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
            _ => {
                tracing::error!("{e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong".to_owned(),
                )
            }
        }
    }
}

impl From<reqwest::Error> for SarcaError {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(e) if e.is_client_error() => SarcaError::TelegramAPIError(e.to_string()),
            Some(_) | None => {
                tracing::error!("{e}");
                SarcaError::Unknown
            }
        }
    }
}

pub type SarcaResult<T> = Result<T, SarcaError>;
