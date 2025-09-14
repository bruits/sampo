use thiserror::Error;

/// Canonical result type for the GitHub Action
pub type Result<T> = std::result::Result<T, ActionError>;

/// Common error type for the GitHub Action operations
#[derive(Debug, Error)]
pub enum ActionError {
    #[error("No working directory provided and GITHUB_WORKSPACE is not set")]
    NoWorkingDirectory,
    #[error("Failed to execute sampo {operation}: {message}")]
    SampoCommandFailed { operation: String, message: String },
    #[error("GitHub credentials not available: GITHUB_REPOSITORY and GITHUB_TOKEN must be set")]
    GitHubCredentialsNotAvailable,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Sampo error: {0}")]
    Sampo(#[from] sampo_core::errors::SampoError),
}
