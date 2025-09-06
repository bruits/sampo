pub mod enrichment;

// Re-export commonly used items
pub use enrichment::{enrich_changeset_message, detect_github_repo_slug, CommitInfo, GitHubUserInfo};
