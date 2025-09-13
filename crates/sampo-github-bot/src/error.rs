pub type Result<T> = std::result::Result<T, BotError>;

#[derive(thiserror::Error, Debug)]
pub enum VerifyError {
    #[error("missing signature header")]
    MissingHeader,
    #[error("invalid signature header")]
    InvalidHeader,
    #[error("signature mismatch")]
    Mismatch,
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(thiserror::Error, Debug)]
pub enum BotError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Octocrab error: {0}")]
    Octo(#[from] octocrab::Error),
    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Verify(#[from] VerifyError),
    #[error("internal: {0}")]
    Internal(String),
}
