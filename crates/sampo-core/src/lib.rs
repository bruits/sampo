pub mod changeset;
pub mod config;
pub mod discovery;
pub mod enrichment;
pub mod errors;
pub mod filters;
pub mod git;
pub mod manifest;
pub mod markdown;
pub mod prerelease;
pub mod publish;
pub mod release;
pub mod types;
pub mod workspace;

// Re-export commonly used items
pub use changeset::{ChangesetInfo, load_changesets, parse_changeset, render_changeset_markdown};
pub use config::Config;
pub use discovery::{CargoDiscovery, PackageDiscovery};
pub use enrichment::{
    CommitInfo, GitHubUserInfo, detect_github_repo_slug, detect_github_repo_slug_with_config,
    enrich_changeset_message, get_commit_hash_for_path,
};
pub use errors::{Result, SampoError, WorkspaceError};
pub use filters::{filter_members, list_visible_packages, should_ignore_package, wildcard_match};
pub use git::current_branch;
pub use manifest::{ManifestMetadata, update_manifest_versions};
pub use markdown::format_markdown_list_item;
pub use prerelease::{
    VersionChange, enter_prerelease, exit_prerelease, restore_preserved_changesets,
};
pub use publish::{
    is_publishable_to_crates_io, run_publish, tag_published_crate, topo_order,
    version_exists_on_crates_io,
};
pub use release::{
    build_dependency_updates, bump_version, create_dependency_update_entry,
    create_fixed_dependency_policy_entry, detect_all_dependency_explanations,
    detect_fixed_dependency_policy_packages, format_dependency_updates_message,
    infer_bump_from_versions, run_release,
};
pub use types::{
    Bump, DependencyUpdate, PackageInfo, PackageKind, ReleaseOutput, ReleasedPackage, Workspace,
};
pub use workspace::{discover_workspace, parse_workspace_members};

#[cfg(test)]
mod release_tests;
