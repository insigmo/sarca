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
    #[error("User already has a storage with such chat id")]
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
    #[error("Invalid folder name")]
    InvalidFolderName,
    #[error("You cannot manage access of yourself")]
    CannotManageAccessOfYourself,
    #[error("Storage does not have workers")]
    StorageDoesNotHaveWorkers,
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
            | SarcaError::CannotManageAccessOfYourself => (StatusCode::CONFLICT, e.to_string()),
            SarcaError::NotAuthenticated => (StatusCode::UNAUTHORIZED, e.to_string()),
            SarcaError::DoesNotExist(_) => (StatusCode::NOT_FOUND, e.to_string()),
            SarcaError::HeaderMissed(_)
            | SarcaError::HeaderIsInvalid(..)
            | SarcaError::InvalidFolderName
            | SarcaError::InvalidPath
            | SarcaError::NoStorageWorkers
            | SarcaError::TelegramAPIError(_) => (StatusCode::BAD_REQUEST, e.to_string()),
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
