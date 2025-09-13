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
    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Verify(#[from] VerifyError),
    #[error("GitHub authentication failed: {0}")]
    GitHubAuth(String),
    #[error("Failed to fetch PR files: {0}")]
    PullRequestFiles(String),
    #[error("Failed to manage comments: {0}")]
    Comments(String),
    #[error("internal: {0}")]
    Internal(String),
}

impl BotError {
    /// Convert octocrab error to GitHub auth error
    pub fn from_github_auth(error: octocrab::Error) -> Self {
        BotError::GitHubAuth(error.to_string())
    }

    /// Convert octocrab error to PR files error
    pub fn from_pr_files(error: octocrab::Error) -> Self {
        BotError::PullRequestFiles(error.to_string())
    }

    /// Convert octocrab error to comments error
    pub fn from_comments(error: octocrab::Error) -> Self {
        BotError::Comments(error.to_string())
    }
}
