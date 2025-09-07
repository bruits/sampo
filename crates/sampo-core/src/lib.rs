pub mod changeset;
pub mod config;
pub mod enrichment;
pub mod errors;
pub mod types;
pub mod workspace;

// Re-export commonly used items
pub use changeset::{
    detect_changesets_dir, load_changesets, parse_changeset, render_changeset_markdown,
    ChangesetInfo,
};
pub use config::Config;
pub use enrichment::{
    detect_github_repo_slug, detect_github_repo_slug_with_config, enrich_changeset_message,
    get_commit_hash_for_path, CommitInfo, GitHubUserInfo,
};
pub use errors::SampoError;
pub use types::{Bump, CrateInfo, Workspace};
pub use workspace::{discover_workspace, parse_workspace_members, WorkspaceError};
