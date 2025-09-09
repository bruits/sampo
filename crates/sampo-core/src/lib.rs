pub mod changeset;
pub mod config;
pub mod enrichment;
pub mod errors;
pub mod publish;
pub mod release;
pub mod types;
pub mod workspace;

// Re-export commonly used items
pub use changeset::{
    ChangesetInfo, detect_changesets_dir, load_changesets, parse_changeset,
    render_changeset_markdown,
};
pub use config::Config;
pub use enrichment::{
    CommitInfo, GitHubUserInfo, detect_github_repo_slug, detect_github_repo_slug_with_config,
    enrich_changeset_message, get_commit_hash_for_path,
};
pub use errors::SampoError;
pub use publish::{
    is_publishable_to_crates_io, run_publish, tag_published_crate, topo_order,
    version_exists_on_crates_io,
};
pub use release::{
    build_dependency_updates, bump_version, create_dependency_update_entry,
    create_fixed_dependency_policy_entry, detect_all_dependency_explanations,
    detect_fixed_dependency_policy_packages, format_dependency_updates_message,
    infer_bump_from_versions, run_release, update_manifest_versions,
};
pub use types::{Bump, CrateInfo, DependencyUpdate, ReleaseOutput, ReleasedPackage, Workspace};
pub use workspace::{WorkspaceError, discover_workspace, parse_workspace_members};

#[cfg(test)]
mod release_tests;
