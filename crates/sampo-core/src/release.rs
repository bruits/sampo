use crate::types::{Bump, DependencyUpdate, Workspace};
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
                    let group_bump = group.iter()
                        .filter_map(|member| {
                            if packages_with_changesets.contains(member) {
                                // Find the highest bump from changesets affecting this member
                                changesets.iter()
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
}
