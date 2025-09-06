pub mod config;
pub mod enrichment;

// Re-export commonly used items
pub use enrichment::{
    CommitInfo, GitHubUserInfo, detect_github_repo_slug, detect_github_repo_slug_with_config,
    enrich_changeset_message, get_commit_hash_for_path,
};
