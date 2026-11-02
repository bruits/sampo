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
    /// `sampo publish` failed, possibly after creating the tags of the packages it published first.
    #[error("Failed to execute sampo publish: sampo publish failed: {source}")]
    PartialPublish {
        tags: Vec<String>,
        source: sampo_core::errors::SampoError,
    },
    /// Tags or GitHub Releases that did not land, after trying all of them.
    #[error("Failed to finish the release:\n  - {}", .failures.join("\n  - "))]
    ReleaseIncomplete { failures: Vec<String> },
    #[error("GitHub credentials not available: GITHUB_REPOSITORY and GITHUB_TOKEN must be set")]
    GitHubCredentialsNotAvailable,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Sampo error: {0}")]
    Sampo(#[from] sampo_core::errors::SampoError),
}
