use std::io;
use std::path::Path;

/// Canonical result type for Sampo code
pub type Result<T> = std::result::Result<T, SampoError>;

/// Common error type for Sampo operations
#[derive(Debug, thiserror::Error)]
pub enum SampoError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Workspace error: {0}")]
    Workspace(#[from] WorkspaceError),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Changeset error: {0}")]
    Changeset(String),

    #[error("Git error: {0}")]
    Git(String),

    #[error("GitHub error: {0}")]
    GitHub(String),

    #[error("Publish error: {0}")]
    Publish(String),

    #[error("Release error: {0}")]
    Release(String),

    #[error("Pre-release error: {0}")]
    Prerelease(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Not found: {0}")]
    NotFound(String),
}

/// Errors that can occur when working with workspaces
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("No supported workspace manifest found")]
    NotFound,
    #[error("Invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("Invalid workspace: {0}")]
    InvalidWorkspace(String),
}

/// Helper to create an IO error with file path context
pub fn io_error_with_path<P: AsRef<Path>>(error: io::Error, path: P) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("{}: {}", path.as_ref().display(), error),
    )
}
