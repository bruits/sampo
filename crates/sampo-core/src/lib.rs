pub mod adapters;
pub mod changeset;
pub mod config;
pub mod enrichment;
pub mod errors;
pub mod filters;
pub mod git;
pub mod markdown;
pub mod prerelease;
pub mod publish;
pub mod release;
pub mod types;
pub mod workspace;

// Re-export commonly used items
pub use adapters::ManifestMetadata;
pub use changeset::{
    ChangesetInfo, load_changesets, parse_changeset, render_changeset_markdown,
    render_changeset_markdown_with_tags,
};
pub use config::Config;
pub use enrichment::{
    CommitInfo, GitHubUserInfo, detect_github_repo_slug, detect_github_repo_slug_with_config,
    enrich_changeset_message, get_commit_hash_for_path,
};
pub use errors::{Result, SampoError, WorkspaceError};
pub use filters::{filter_members, list_visible_packages, should_ignore_package, wildcard_match};
pub use git::current_branch;
pub use markdown::format_markdown_list_item;
pub use prerelease::{
    VersionChange, enter_prerelease, exit_prerelease, restore_preserved_changesets,
};
pub use publish::{run_publish, tag_published_crate, topo_order};
pub use release::{
    build_dependency_updates, bump_version, create_dependency_update_entry,
    create_fixed_dependency_policy_entry, detect_all_dependency_explanations,
    detect_fixed_dependency_policy_packages, format_dependency_updates_message,
    infer_bump_from_versions, run_release,
};
pub use types::{
    Bump, ChangelogCategory, DependencyUpdate, PackageInfo, PackageKind, ParsedChangeType,
    ReleaseOutput, ReleasedPackage, Workspace,
};
pub use workspace::discover_workspace;

#[cfg(test)]
mod release_tests;
