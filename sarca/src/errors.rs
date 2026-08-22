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
    #[error("too many attempts, try again later")]
    TooManyAttempts,
    #[error("forbidden")]
    Forbidden,
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
    #[error("The superuser account cannot be deleted")]
    CannotDeleteSuperuser,
    #[error("Only the superuser can manage administrator access")]
    OnlySuperuserManagesAdmins,
    #[error("Storage does not have workers")]
    StorageDoesNotHaveWorkers,
    #[error("storage_id is required")]
    WorkerRequiresStorage,
    #[error("A storage can have at most 3 channels")]
    TooManyChannels,
    #[error("Cannot remove the last active channel")]
    LastActiveChannel,
    #[error("Replacing the bot removes its channels and their files — confirm to continue")]
    BotReplacementRequiresChannelConfirmation,
    #[error("Storage has no active channel available")]
    NoActiveChannel,
    #[error("A file already exists at this path")]
    TrashPathConflict,
    #[error("Invalid trash retention days (must be 1–30)")]
    InvalidTrashRetention,
    #[error("Share expiry must be in the future")]
    InvalidShareExpiry,
    #[error("unknown error")]
    Unknown,
    #[error("{0} header is required")]
    HeaderMissed(String),
    #[error("{0} header should be a valid {1}")]
    HeaderIsInvalid(String, String),
    #[error("storage is busy, retry shortly")]
    StorageBusy,
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
            | SarcaError::BotReplacementRequiresChannelConfirmation
            | SarcaError::CannotManageAccessOfYourself
            | SarcaError::TrashPathConflict => (StatusCode::CONFLICT, e.to_string()),
            SarcaError::NotAuthenticated => (StatusCode::UNAUTHORIZED, e.to_string()),
            SarcaError::TooManyAttempts => (StatusCode::TOO_MANY_REQUESTS, e.to_string()),
            SarcaError::Forbidden
            | SarcaError::CannotDeleteSuperuser
            | SarcaError::OnlySuperuserManagesAdmins => (StatusCode::FORBIDDEN, e.to_string()),
            SarcaError::DoesNotExist(_) => (StatusCode::NOT_FOUND, e.to_string()),
            SarcaError::FolderTooLargeForZip => (StatusCode::PAYLOAD_TOO_LARGE, e.to_string()),
            // Distinct from TelegramAPIError/NoStorageWorkers below: this is a transient
            // capacity wait, not a client mistake, so it must not join their 400 arm.
            SarcaError::StorageBusy => (StatusCode::SERVICE_UNAVAILABLE, e.to_string()),
            SarcaError::HeaderMissed(_)
            | SarcaError::HeaderIsInvalid(..)
            | SarcaError::InvalidFolderName
            | SarcaError::InvalidPath
            | SarcaError::NoStorageWorkers
            | SarcaError::NoActiveChannel
            | SarcaError::WorkerRequiresStorage
            | SarcaError::InvalidTrashRetention
            | SarcaError::InvalidShareExpiry
            | SarcaError::TelegramAPIError(_) => (StatusCode::BAD_REQUEST, e.to_string()),
            _ => {
                tracing::error!("{e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong".to_owned())
            },
        }
    }
}

impl From<reqwest::Error> for SarcaError {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(e) if e.is_client_error() => Self::TelegramAPIError(e.to_string()),
            Some(_) | None => {
                tracing::error!("{e}");
                Self::Unknown
            },
        }
    }
}

pub type SarcaResult<T> = Result<T, SarcaError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn throttled_attempts_answer_429_not_401() {
        let (status, body): (StatusCode, String) = SarcaError::TooManyAttempts.into();
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        // The message must not say whether the credential was right.
        assert!(!body.contains("password"), "{body}");
    }

    #[test]
    fn a_bad_credential_still_answers_401() {
        let (status, _): (StatusCode, String) = SarcaError::NotAuthenticated.into();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
