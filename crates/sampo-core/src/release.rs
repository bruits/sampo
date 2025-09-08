use crate::types::{Bump, CrateInfo, DependencyUpdate, Workspace};
use crate::{changeset::ChangesetInfo, config::Config};
use std::collections::{BTreeMap, BTreeSet};

/// Format dependency updates for changelog display
///
/// Creates a message in the style of Changesets for dependency updates,
/// e.g., "Updated dependencies [hash]: pkg1@1.2.0, pkg2@2.0.0"
pub fn format_dependency_updates_message(updates: &[DependencyUpdate]) -> Option<String> {
    if updates.is_empty() {
        return None;
    }

    let dep_list = updates
        .iter()
        .map(|dep| format!("{}@{}", dep.name, dep.new_version))
        .collect::<Vec<_>>()
        .join(", ");

    Some(format!("Updated dependencies: {}", dep_list))
}

/// Convert a list of (name, version) tuples into DependencyUpdate structs
pub fn build_dependency_updates(updates: &[(String, String)]) -> Vec<DependencyUpdate> {
    updates
        .iter()
        .map(|(name, version)| DependencyUpdate {
            name: name.clone(),
            new_version: version.clone(),
        })
        .collect()
}

/// Create a changelog entry for dependency updates
///
/// Returns a tuple of (message, bump_type) suitable for adding to changelog messages
pub fn create_dependency_update_entry(updates: &[DependencyUpdate]) -> Option<(String, Bump)> {
    format_dependency_updates_message(updates).map(|msg| (msg, Bump::Patch))
}

/// Create a changelog entry for fixed dependency group policy
///
/// Returns a tuple of (message, bump_type) suitable for adding to changelog messages
pub fn create_fixed_dependency_policy_entry(bump: Bump) -> (String, Bump) {
    (
        "Bumped due to fixed dependency group policy".to_string(),
        bump,
    )
}

/// Infer bump type from version changes
///
/// This helper function determines the semantic version bump type based on
/// the difference between old and new version strings.
pub fn infer_bump_from_versions(old_ver: &str, new_ver: &str) -> Bump {
    let old_parts: Vec<u32> = old_ver.split('.').filter_map(|s| s.parse().ok()).collect();
    let new_parts: Vec<u32> = new_ver.split('.').filter_map(|s| s.parse().ok()).collect();

    if old_parts.len() >= 3 && new_parts.len() >= 3 {
        if new_parts[0] > old_parts[0] {
            Bump::Major
        } else if new_parts[1] > old_parts[1] {
            Bump::Minor
        } else {
            Bump::Patch
        }
    } else {
        Bump::Patch
    }
}

/// Detect all dependency-related explanations for package releases
///
/// This function is the unified entry point for detecting all types of automatic
/// dependency-related changelog entries. It identifies:
/// - Packages bumped due to internal dependency updates ("Updated dependencies: ...")
/// - Packages bumped due to fixed dependency group policy ("Bumped due to fixed dependency group policy")
///
/// # Arguments
/// * `changesets` - The changesets being processed
/// * `workspace` - The workspace containing all packages
/// * `config` - The configuration with dependency policies
/// * `releases` - Map of package name to (old_version, new_version) for all planned releases
///
/// # Returns
/// A map of package name to list of (message, bump_type) explanations to add to changelogs
pub fn detect_all_dependency_explanations(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    releases: &BTreeMap<String, (String, String)>,
) -> BTreeMap<String, Vec<(String, Bump)>> {
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, Bump)>> = BTreeMap::new();

    // 1. Detect packages bumped due to fixed dependency group policy
    let bumped_packages: BTreeSet<String> = releases.keys().cloned().collect();
    let policy_packages =
        detect_fixed_dependency_policy_packages(changesets, workspace, config, &bumped_packages);

    for (pkg_name, policy_bump) in policy_packages {
        // For accurate bump detection, infer from actual version changes
        let actual_bump = if let Some((old_ver, new_ver)) = releases.get(&pkg_name) {
            infer_bump_from_versions(old_ver, new_ver)
        } else {
            policy_bump
        };

        let (msg, bump_type) = create_fixed_dependency_policy_entry(actual_bump);
        messages_by_pkg
            .entry(pkg_name)
            .or_default()
            .push((msg, bump_type));
    }

    // 2. Detect packages bumped due to internal dependency updates
    // Note: Even packages with explicit changesets can have dependency updates

    // Build new version lookup from releases
    let new_version_by_name: BTreeMap<String, String> = releases
        .iter()
        .map(|(name, (_old, new_ver))| (name.clone(), new_ver.clone()))
        .collect();

    // Build map of crate name -> CrateInfo for quick lookup
    let by_name: BTreeMap<String, &CrateInfo> = workspace
        .members
        .iter()
        .map(|c| (c.name.clone(), c))
        .collect();

    // For each released crate, check if it has internal dependencies that were updated
    for crate_name in releases.keys() {
        if let Some(crate_info) = by_name.get(crate_name) {
            // Find which internal dependencies were updated
            let mut updated_deps = Vec::new();
            for dep_name in &crate_info.internal_deps {
                if let Some(new_version) = new_version_by_name.get(dep_name as &str) {
                    // This internal dependency was updated
                    updated_deps.push((dep_name.clone(), new_version.clone()));
                }
            }

            if !updated_deps.is_empty() {
                // Create dependency update entry
                let updates = build_dependency_updates(&updated_deps);
                if let Some((msg, bump)) = create_dependency_update_entry(&updates) {
                    messages_by_pkg
                        .entry(crate_name.clone())
                        .or_default()
                        .push((msg, bump));
                }
            }
        }
    }

    messages_by_pkg
}

/// Detect packages that need fixed dependency group policy messages
///
/// This function identifies packages that were bumped solely due to fixed dependency
/// group policies (not due to direct changesets or normal dependency cascades).
/// Returns a map of package name to the bump level they received.
pub fn detect_fixed_dependency_policy_packages(
    changesets: &[ChangesetInfo],
    workspace: &Workspace,
    config: &Config,
    bumped_packages: &BTreeSet<String>,
) -> BTreeMap<String, Bump> {
    // Build set of packages with direct changesets
    let packages_with_changesets: BTreeSet<String> = changesets
        .iter()
        .flat_map(|cs| cs.packages.iter().cloned())
        .collect();

    // Build dependency graph (dependent -> set of dependencies)
    let mut dependents: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for crate_info in &workspace.members {
        for dep_name in &crate_info.internal_deps {
            dependents
                .entry(dep_name.clone())
                .or_default()
                .insert(crate_info.name.clone());
        }
    }

    // Find packages affected by normal dependency cascade
    let mut packages_affected_by_cascade = BTreeSet::new();
    for pkg_with_changeset in &packages_with_changesets {
        let mut queue = vec![pkg_with_changeset.clone()];
        let mut visited = BTreeSet::new();

        while let Some(pkg) = queue.pop() {
            if visited.contains(&pkg) {
                continue;
            }
            visited.insert(pkg.clone());

            if let Some(deps) = dependents.get(&pkg) {
                for dep in deps {
                    packages_affected_by_cascade.insert(dep.clone());
                    queue.push(dep.clone());
                }
            }
        }
    }

    // Find packages that need fixed dependency policy messages
    let mut result = BTreeMap::new();

    for pkg_name in bumped_packages {
        // Skip if package has direct changeset
        if packages_with_changesets.contains(pkg_name) {
            continue;
        }

        // Skip if package is affected by normal dependency cascade
        if packages_affected_by_cascade.contains(pkg_name) {
            continue;
        }

        // Check if this package is in a fixed dependency group with an affected package
        for group in &config.fixed_dependencies {
            if group.contains(&pkg_name.to_string()) {
                // Check if any other package in this group has changes
                let has_affected_group_member = group.iter().any(|group_member| {
                    group_member != pkg_name
                        && (packages_with_changesets.contains(group_member)
                            || packages_affected_by_cascade.contains(group_member))
                });

                if has_affected_group_member {
                    // Find the highest bump level in the group to determine the policy bump
                    let group_bump = group
                        .iter()
                        .filter_map(|member| {
                            if packages_with_changesets.contains(member) {
                                // Find the highest bump from changesets affecting this member
                                changesets
                                    .iter()
                                    .filter(|cs| cs.packages.contains(member))
                                    .map(|cs| cs.bump)
                                    .max()
                            } else {
                                None
                            }
                        })
                        .max()
                        .unwrap_or(Bump::Patch);

                    result.insert(pkg_name.clone(), group_bump);
                    break;
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_single_dependency_update() {
        let updates = vec![DependencyUpdate {
            name: "pkg1".to_string(),
            new_version: "1.2.0".to_string(),
        }];
        let msg = format_dependency_updates_message(&updates).unwrap();
        assert_eq!(msg, "Updated dependencies: pkg1@1.2.0");
    }

    #[test]
    fn formats_multiple_dependency_updates() {
        let updates = vec![
            DependencyUpdate {
                name: "pkg1".to_string(),
                new_version: "1.2.0".to_string(),
            },
            DependencyUpdate {
                name: "pkg2".to_string(),
                new_version: "2.0.0".to_string(),
            },
        ];
        let msg = format_dependency_updates_message(&updates).unwrap();
        assert_eq!(msg, "Updated dependencies: pkg1@1.2.0, pkg2@2.0.0");
    }

    #[test]
    fn returns_none_for_empty_updates() {
        let updates = vec![];
        let msg = format_dependency_updates_message(&updates);
        assert_eq!(msg, None);
    }

    #[test]
    fn builds_dependency_updates_from_tuples() {
        let tuples = vec![
            ("pkg1".to_string(), "1.2.0".to_string()),
            ("pkg2".to_string(), "2.0.0".to_string()),
        ];
        let updates = build_dependency_updates(&tuples);
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].name, "pkg1");
        assert_eq!(updates[0].new_version, "1.2.0");
        assert_eq!(updates[1].name, "pkg2");
        assert_eq!(updates[1].new_version, "2.0.0");
    }

    #[test]
    fn creates_dependency_update_entry() {
        let updates = vec![DependencyUpdate {
            name: "pkg1".to_string(),
            new_version: "1.2.0".to_string(),
        }];
        let (msg, bump) = create_dependency_update_entry(&updates).unwrap();
        assert_eq!(msg, "Updated dependencies: pkg1@1.2.0");
        assert_eq!(bump, Bump::Patch);
    }

    #[test]
    fn creates_fixed_dependency_policy_entry() {
        let (msg, bump) = create_fixed_dependency_policy_entry(Bump::Major);
        assert_eq!(msg, "Bumped due to fixed dependency group policy");
        assert_eq!(bump, Bump::Major);

        let (msg, bump) = create_fixed_dependency_policy_entry(Bump::Minor);
        assert_eq!(msg, "Bumped due to fixed dependency group policy");
        assert_eq!(bump, Bump::Minor);
    }

    #[test]
    fn infers_bump_from_version_changes() {
        assert_eq!(infer_bump_from_versions("1.0.0", "2.0.0"), Bump::Major);
        assert_eq!(infer_bump_from_versions("1.0.0", "1.1.0"), Bump::Minor);
        assert_eq!(infer_bump_from_versions("1.0.0", "1.0.1"), Bump::Patch);

        // Edge cases
        assert_eq!(infer_bump_from_versions("0.1", "0.2"), Bump::Patch);
        assert_eq!(infer_bump_from_versions("invalid", "1.0.0"), Bump::Patch);
    }

    #[test]
    fn detect_all_dependency_explanations_comprehensive() {
        use crate::types::{CrateInfo, Workspace};
        use std::collections::BTreeSet;
        use std::path::PathBuf;

        // Create test workspace with dependencies
        let ws = Workspace {
            root: PathBuf::from("/test"),
            members: vec![
                CrateInfo {
                    name: "pkg-a".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-a"),
                    internal_deps: BTreeSet::from(["pkg-b".to_string()]),
                },
                CrateInfo {
                    name: "pkg-b".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-b"),
                    internal_deps: BTreeSet::new(),
                },
                CrateInfo {
                    name: "pkg-c".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-c"),
                    internal_deps: BTreeSet::new(),
                },
            ],
        };

        // Create config with fixed dependencies
        let config = Config {
            version: 1,
            github_repository: None,
            changelog_show_commit_hash: true,
            changelog_show_acknowledgments: true,
            fixed_dependencies: vec![vec!["pkg-a".to_string(), "pkg-c".to_string()]],
            linked_dependencies: vec![],
        };

        // Create changeset that affects pkg-b only
        let changesets = vec![ChangesetInfo {
            packages: vec!["pkg-b".to_string()],
            bump: Bump::Minor,
            message: "feat: new feature".to_string(),
            path: PathBuf::from("/test/.sampo/changesets/test.md"),
        }];

        // Simulate releases: pkg-a and pkg-c get fixed bump, pkg-b gets direct bump
        let mut releases = BTreeMap::new();
        releases.insert(
            "pkg-a".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );
        releases.insert(
            "pkg-b".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );
        releases.insert(
            "pkg-c".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );

        let explanations = detect_all_dependency_explanations(&changesets, &ws, &config, &releases);

        // pkg-a should have dependency update message (depends on pkg-b)
        let pkg_a_messages = explanations.get("pkg-a").unwrap();
        assert_eq!(pkg_a_messages.len(), 1);
        assert!(
            pkg_a_messages[0]
                .0
                .contains("Updated dependencies: pkg-b@1.1.0")
        );
        assert_eq!(pkg_a_messages[0].1, Bump::Patch);

        // pkg-c should have fixed dependency policy message (no deps but in fixed group)
        let pkg_c_messages = explanations.get("pkg-c").unwrap();
        assert_eq!(pkg_c_messages.len(), 1);
        assert_eq!(
            pkg_c_messages[0].0,
            "Bumped due to fixed dependency group policy"
        );
        assert_eq!(pkg_c_messages[0].1, Bump::Minor); // Inferred from version change

        // pkg-b should have no messages (explicit changeset)
        assert!(!explanations.contains_key("pkg-b"));
    }

    #[test]
    fn detect_all_dependency_explanations_empty_cases() {
        use crate::types::{CrateInfo, Workspace};
        use std::collections::BTreeSet;
        use std::path::PathBuf;

        let ws = Workspace {
            root: PathBuf::from("/test"),
            members: vec![CrateInfo {
                name: "pkg-a".to_string(),
                version: "1.0.0".to_string(),
                path: PathBuf::from("/test/pkg-a"),
                internal_deps: BTreeSet::new(),
            }],
        };

        let config = Config::default();
        let changesets = vec![];
        let releases = BTreeMap::new();

        let explanations = detect_all_dependency_explanations(&changesets, &ws, &config, &releases);
        assert!(explanations.is_empty());
    }
}
