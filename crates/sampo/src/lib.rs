pub mod enrichment;

// Re-export commonly used items
pub use enrichment::{
    CommitInfo, GitHubUserInfo, detect_github_repo_slug, enrich_changeset_message,
};
