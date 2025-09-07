use std::io;

/// Common error type for Sampo operations
#[derive(Debug, thiserror::Error)]
pub enum SampoError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Workspace error: {0}")]
    Workspace(#[from] crate::workspace::WorkspaceError),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Changeset error: {0}")]
    Changeset(String),

    #[error("Git error: {0}")]
    Git(String),

    #[error("GitHub error: {0}")]
    GitHub(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Not found: {0}")]
    NotFound(String),
}
