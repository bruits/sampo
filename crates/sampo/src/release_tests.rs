#[cfg(test)]
mod tests {
    use crate::cli::ReleaseArgs;
    use crate::release::run_in;
    use rustc_hash::FxHashMap;
    use sampo_core::{Bump, bump_version, update_manifest_versions};
    use std::fs;
    use std::path::PathBuf;

    /// Test workspace builder for reducing test boilerplate
    struct TestWorkspace {
        root: PathBuf,
        _temp_dir: tempfile::TempDir,
        crates: FxHashMap<String, PathBuf>,
    }

    impl TestWorkspace {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().unwrap();
            let root = temp_dir.path().to_path_buf();

            // Create basic workspace structure
            fs::write(
                root.join("Cargo.toml"),
                "[workspace]\nmembers=[\"crates/*\"]\n",
            )
            .unwrap();

            Self {
                root,
                _temp_dir: temp_dir,
                crates: FxHashMap::default(),
            }
        }

        fn add_crate(&mut self, name: &str, version: &str) -> &mut Self {
            let crate_dir = self.root.join("crates").join(name);
            fs::create_dir_all(&crate_dir).unwrap();

            fs::write(
                crate_dir.join("Cargo.toml"),
                format!("[package]\nname=\"{}\"\nversion=\"{}\"\n", name, version),
            )
            .unwrap();

            self.crates.insert(name.to_string(), crate_dir);
            self
        }

        fn add_dependency(&mut self, from: &str, to: &str, version: &str) -> &mut Self {
            let from_dir = self.crates.get(from).expect("from crate must exist");
            let current_manifest = fs::read_to_string(from_dir.join("Cargo.toml")).unwrap();

            let dependency_section = format!(
                "\n[dependencies]\n{} = {{ path=\"../{}\", version=\"{}\" }}\n",
                to, to, version
            );

            fs::write(
                from_dir.join("Cargo.toml"),
                current_manifest + &dependency_section,
            )
            .unwrap();

            self
        }

        fn add_changeset(&self, packages: &[&str], release: Bump, message: &str) -> &Self {
            let changesets_dir = self.root.join(".sampo/changesets");
            fs::create_dir_all(&changesets_dir).unwrap();

            let packages_yaml = packages
                .iter()
                .map(|p| format!("  - {}", p))
                .collect::<Vec<_>>()
                .join("\n");

            let release_type = match release {
                Bump::Patch => "patch",
                Bump::Minor => "minor",
                Bump::Major => "major",
            };

            let changeset_content = format!(
                "---\npackages:\n{}\nrelease: {}\n---\n\n{}\n",
                packages_yaml, release_type, message
            );

            // Use message slug as filename to avoid conflicts
            let filename = message
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-')
                .collect::<String>()
                .to_lowercase()
                + ".md";

            fs::write(changesets_dir.join(filename), changeset_content).unwrap();
            self
        }

        fn set_config(&self, config_content: &str) -> &Self {
            fs::create_dir_all(self.root.join(".sampo")).unwrap();
            fs::write(self.root.join(".sampo/config.toml"), config_content).unwrap();
            self
        }

        fn add_existing_changelog(&self, crate_name: &str, content: &str) -> &Self {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            fs::write(crate_dir.join("CHANGELOG.md"), content).unwrap();
            self
        }

        fn run_release(&self, dry_run: bool) -> Result<(), std::io::Error> {
            run_in(&self.root, &ReleaseArgs { dry_run })
        }

        fn assert_crate_version(&self, crate_name: &str, expected_version: &str) {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            let manifest = fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();

            let version_check = format!("version=\"{}\"", expected_version);
            let version_check_spaces = format!("version = \"{}\"", expected_version);

            assert!(
                manifest.contains(&version_check) || manifest.contains(&version_check_spaces),
                "Expected {} to have version {}, but manifest was:\n{}",
                crate_name,
                expected_version,
                manifest
            );
        }

        fn assert_dependency_version(
            &self,
            from_crate: &str,
            to_crate: &str,
            expected_version: &str,
        ) {
            let from_dir = self.crates.get(from_crate).expect("from crate must exist");
            let manifest = fs::read_to_string(from_dir.join("Cargo.toml")).unwrap();
            let manifest_toml: toml::Value = manifest.parse().unwrap();

            let dep_entry = manifest_toml
                .get("dependencies")
                .and_then(toml::Value::as_table)
                .and_then(|t| t.get(to_crate))
                .cloned()
                .unwrap_or_else(|| {
                    panic!("dependency '{}' must exist in {}", to_crate, from_crate)
                });

            match dep_entry {
                toml::Value::String(v) => assert_eq!(v, expected_version),
                toml::Value::Table(tbl) => {
                    let v = tbl.get("version").and_then(toml::Value::as_str).unwrap();
                    assert_eq!(v, expected_version);
                }
                _ => panic!("unexpected dependency entry type"),
            }
        }

        fn assert_changelog_contains(&self, crate_name: &str, content: &str) {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            let changelog_path = crate_dir.join("CHANGELOG.md");
            assert!(
                changelog_path.exists(),
                "CHANGELOG.md should exist for {}",
                crate_name
            );

            let changelog = fs::read_to_string(changelog_path).unwrap();
            assert!(
                changelog.contains(content),
                "Expected changelog for {} to contain '{}', but was:\n{}",
                crate_name,
                content,
                changelog
            );
        }

        fn read_changelog(&self, crate_name: &str) -> String {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            let changelog_path = crate_dir.join("CHANGELOG.md");
            if changelog_path.exists() {
                fs::read_to_string(changelog_path).unwrap()
            } else {
                String::new()
            }
        }
    }

    #[test]
    fn bumps_versions() {
        assert_eq!(bump_version("0.0.0", Bump::Patch).unwrap(), "0.0.1");
        assert_eq!(bump_version("0.1.2", Bump::Minor).unwrap(), "0.2.0");
        assert_eq!(bump_version("1.2.3", Bump::Major).unwrap(), "2.0.0");
    }

    #[test]
    fn updates_version_in_toml() {
        use sampo_core::{CrateInfo, Workspace};
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        let input = "[package]\nname=\"x\"\nversion = \"0.1.0\"\n\n[dependencies]\n";
        let ws = Workspace {
            root: PathBuf::from("/test"),
            members: vec![CrateInfo {
                name: "x".to_string(),
                version: "0.1.0".to_string(),
                path: PathBuf::from("/test/crates/x"),
                internal_deps: Default::default(),
            }],
        };
        let new_versions = BTreeMap::new();
        let (out, _) = update_manifest_versions(input, Some("0.2.0"), &ws, &new_versions).unwrap();
        assert!(out.contains("version = \"0.2.0\""));
        assert!(out.contains("[dependencies]"));
    }

    #[test]
    fn no_changesets_returns_ok_and_no_changes() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("x", "0.1.0");

        // No changesets directory created -> load_all returns empty
        workspace.run_release(false).unwrap();

        // Verify no change to manifest
        workspace.assert_crate_version("x", "0.1.0");

        // No changelog created
        let crate_dir = workspace.crates.get("x").unwrap();
        assert!(!crate_dir.join("CHANGELOG.md").exists());
    }

    #[test]
    fn changelog_top_section_is_merged_and_reheaded() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("x", "0.1.0")
            .add_existing_changelog(
                "x",
                "# x\n\n## 0.1.1\n\n### Patch changes\n\n- fix: a bug\n\n",
            )
            .add_changeset(&["x"], Bump::Minor, "feat: new thing");

        workspace.run_release(false).unwrap();

        workspace.assert_crate_version("x", "0.2.0");
        workspace.assert_changelog_contains("x", "# x");
        workspace.assert_changelog_contains("x", "## 0.2.0");
        workspace.assert_changelog_contains("x", "### Minor changes");
        workspace.assert_changelog_contains("x", "feat: new thing");
        workspace.assert_changelog_contains("x", "### Patch changes");
        workspace.assert_changelog_contains("x", "fix: a bug");

        // Ensure only one top section, and previous 0.1.1 header is gone
        let crate_dir = workspace.crates.get("x").unwrap();
        let log = fs::read_to_string(crate_dir.join("CHANGELOG.md")).unwrap();
        assert!(!log.contains("## 0.1.1\n"));
    }

    #[test]
    fn published_top_section_is_preserved_and_new_section_is_added() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("x", "0.1.0")
            .add_existing_changelog(
                "x",
                "# x\n\n## 0.1.0\n\n### Patch changes\n\n- initial patch\n\n",
            )
            .add_changeset(&["x"], Bump::Minor, "feat: new minor");

        workspace.run_release(false).unwrap();

        workspace.assert_crate_version("x", "0.2.0");

        // The new section should be present and come before 0.1.0
        let crate_dir = workspace.crates.get("x").unwrap();
        let log = fs::read_to_string(crate_dir.join("CHANGELOG.md")).unwrap();
        let idx_new = log.find("## 0.2.0").unwrap();
        let idx_old = log.find("## 0.1.0").unwrap();
        assert!(idx_new < idx_old, "new section must precede published one");

        workspace.assert_changelog_contains("x", "### Minor changes");
        workspace.assert_changelog_contains("x", "feat: new minor");
        workspace.assert_changelog_contains("x", "### Patch changes");
        workspace.assert_changelog_contains("x", "initial patch");
    }

    #[test]
    fn auto_bumps_dependents_and_updates_internal_dep_versions() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "0.1.0")
            .add_crate("b", "0.1.0")
            .add_dependency("a", "b", "0.1.0")
            .add_changeset(&["b"], Bump::Minor, "feat: b adds new feature");

        workspace.run_release(false).unwrap();

        // Verify b bumped minor -> 0.2.0
        workspace.assert_crate_version("b", "0.2.0");

        // Verify a auto-bumped patch and its dependency updated to 0.2.0
        workspace.assert_crate_version("a", "0.1.1");
        workspace.assert_dependency_version("a", "b", "0.2.0");

        // Changelog for a exists with 0.1.1 section and dependency update message
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 0.1.1");
        workspace.assert_changelog_contains("a", "Updated dependencies: b@0.2.0");
    }

    #[test]
    fn fixed_dependencies_bump_with_same_level() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("a", "b", "1.0.0")
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"b\"]]\n")
            .add_changeset(&["b"], Bump::Major, "breaking: b breaking change");

        workspace.run_release(false).unwrap();

        // Both should be bumped to 2.0.0 (same level as fixed dependencies)
        workspace.assert_crate_version("a", "2.0.0");
        workspace.assert_crate_version("b", "2.0.0");
        workspace.assert_dependency_version("a", "b", "2.0.0");

        // Both should have changelogs with major bump
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 2.0.0");
        workspace.assert_changelog_contains("b", "# b");
        workspace.assert_changelog_contains("b", "## 2.0.0");
        // Check that the automatically bumped package 'a' has dependency update message
        workspace.assert_changelog_contains("a", "Updated dependencies: b@2.0.0");
    }

    #[test]
    fn fixed_dependencies_bidirectional() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("b", "a", "1.0.0") // b depends on a (reverse)
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"b\"]]\n")
            .add_changeset(&["a"], Bump::Minor, "feat: a adds new feature");

        workspace.run_release(false).unwrap();

        // Both should be bumped to 1.1.0 (bidirectional)
        workspace.assert_crate_version("a", "1.1.0");
        workspace.assert_crate_version("b", "1.1.0");
        workspace.assert_dependency_version("b", "a", "1.1.0");

        // Both should have changelogs
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 1.1.0");
        workspace.assert_changelog_contains("b", "# b");
        workspace.assert_changelog_contains("b", "## 1.1.0");
    }

    #[test]
    fn multiple_fixed_dependency_groups() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_crate("c", "1.0.0")
            .add_crate("d", "1.0.0")
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"b\"], [\"c\", \"d\"]]\n")
            .add_changeset(&["a"], Bump::Minor, "feat: a feature");

        workspace.run_release(false).unwrap();

        // Only a and b should be bumped (same group)
        workspace.assert_crate_version("a", "1.1.0");
        workspace.assert_crate_version("b", "1.1.0");

        // c and d should remain unchanged (different group)
        workspace.assert_crate_version("c", "1.0.0");
        workspace.assert_crate_version("d", "1.0.0");
    }

    #[test]
    fn rejects_nonexistent_package_in_fixed_dependencies() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"nonexistent\"]]\n");

        let result = workspace.run_release(false);
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("Package 'nonexistent' in fixed dependency group"));
        assert!(error_msg.contains("does not exist in the workspace"));
    }

    #[test]
    fn linked_dependencies_basic_scenario() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("a", "b", "1.0.0") // a depends on b
            .set_config("[packages]\nlinked_dependencies = [[\"a\", \"b\"]]\n")
            .add_changeset(&["b"], Bump::Major, "breaking: b breaking change");

        workspace.run_release(false).unwrap();

        // Both should be bumped to 2.0.0 (highest bump level)
        workspace.assert_crate_version("a", "2.0.0");
        workspace.assert_crate_version("b", "2.0.0");
        workspace.assert_dependency_version("a", "b", "2.0.0");
    }

    #[test]
    fn linked_dependencies_mixed_bump_levels() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_crate("c", "1.0.0")
            .add_dependency("a", "b", "1.0.0") // a depends on b
            .add_dependency("c", "b", "1.0.0") // c depends on b
            .set_config("[packages]\nlinked_dependencies = [[\"a\", \"b\", \"c\"]]\n")
            .add_changeset(&["b"], Bump::Minor, "feat: b new feature")
            .add_changeset(&["c"], Bump::Patch, "fix: c bug fix");

        workspace.run_release(false).unwrap();

        // All should be bumped to 1.1.0 (highest bump level is minor)
        workspace.assert_crate_version("a", "1.1.0");
        workspace.assert_crate_version("b", "1.1.0");
        workspace.assert_crate_version("c", "1.1.0");

        // Check that auto-bumped package 'a' has dependency update message
        workspace.assert_changelog_contains("a", "Updated dependencies: b@1.1.0");
    }

    #[test]
    fn linked_dependencies_only_affected_packages() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_crate("c", "1.0.0") // c is in group but has no dependencies
            .add_dependency("a", "b", "1.0.0") // a depends on b
            .set_config("[packages]\nlinked_dependencies = [[\"a\", \"b\", \"c\"]]\n")
            .add_changeset(&["b"], Bump::Minor, "feat: b new feature");

        workspace.run_release(false).unwrap();

        // Only a and b should be bumped (affected by changes)
        workspace.assert_crate_version("a", "1.1.0");
        workspace.assert_crate_version("b", "1.1.0");

        // c should remain unchanged (not affected by dependency cascade)
        workspace.assert_crate_version("c", "1.0.0");
    }

    #[test]
    fn linked_dependencies_comprehensive_behavior() {
        // Comprehensive test to document linked dependencies behavior
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("affected_directly", "1.0.0")      // Has changeset
            .add_crate("affected_by_cascade", "1.0.0")    // Depends on affected_directly
            .add_crate("unaffected_in_group", "1.0.0")    // In group but no relation
            .add_crate("outside_group", "1.0.0")          // Not in group at all
            .add_dependency("affected_by_cascade", "affected_directly", "1.0.0")
            .set_config("[packages]\nlinked_dependencies = [[\"affected_directly\", \"affected_by_cascade\", \"unaffected_in_group\"]]\n")
            .add_changeset(&["affected_directly"], Bump::Minor, "feat: new feature");

        workspace.run_release(false).unwrap();

        // affected_directly: has changeset -> bumped to 1.1.0 (minor)
        workspace.assert_crate_version("affected_directly", "1.1.0");

        // affected_by_cascade: depends on affected_directly -> bumped by cascade,
        // then upgraded to 1.1.0 due to linked group highest bump
        workspace.assert_crate_version("affected_by_cascade", "1.1.0");

        // unaffected_in_group: in linked group but no changeset and no dependencies
        // -> should NOT be bumped (key behavior!)
        workspace.assert_crate_version("unaffected_in_group", "1.0.0");

        // outside_group: not in any group -> should NOT be bumped
        workspace.assert_crate_version("outside_group", "1.0.0");

        // Verify changelogs
        workspace.assert_changelog_contains("affected_directly", "feat: new feature");
        workspace.assert_changelog_contains(
            "affected_by_cascade",
            "Updated dependencies: affected_directly@1.1.0",
        );

        // unaffected_in_group should have no changelog (not bumped)
        let changelog = workspace.read_changelog("unaffected_in_group");
        assert!(
            changelog.is_empty(),
            "unaffected_in_group should have no changelog"
        );
    }

    #[test]
    fn linked_dependencies_multiple_direct_changes() {
        // Test case: multiple packages in linked group have their own changesets
        // The unaffected package should still not be bumped
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("pkg_a", "1.0.0")           // Has major changeset
            .add_crate("pkg_b", "1.0.0")           // Has minor changeset
            .add_crate("pkg_c", "1.0.0")           // In group but no changeset, no deps
            .add_crate("pkg_d", "1.0.0")           // Depends on pkg_a
            .add_dependency("pkg_d", "pkg_a", "1.0.0")
            .set_config("[packages]\nlinked_dependencies = [[\"pkg_a\", \"pkg_b\", \"pkg_c\", \"pkg_d\"]]\n")
            .add_changeset(&["pkg_a"], Bump::Major, "breaking: major change in a")
            .add_changeset(&["pkg_b"], Bump::Minor, "feat: minor change in b");

        workspace.run_release(false).unwrap();

        // pkg_a: major changeset -> 2.0.0 (highest bump in group)
        workspace.assert_crate_version("pkg_a", "2.0.0");

        // pkg_b: minor changeset, but upgraded to major due to linked group -> 2.0.0
        workspace.assert_crate_version("pkg_b", "2.0.0");

        // pkg_d: depends on pkg_a, affected by cascade, upgraded to major -> 2.0.0
        workspace.assert_crate_version("pkg_d", "2.0.0");

        // pkg_c: in linked group but no changeset and no dependencies -> NOT bumped
        workspace.assert_crate_version("pkg_c", "1.0.0");

        // Verify changelog messages
        workspace.assert_changelog_contains("pkg_a", "breaking: major change in a");
        workspace.assert_changelog_contains("pkg_b", "feat: minor change in b");
        workspace.assert_changelog_contains("pkg_d", "Updated dependencies: pkg_a@2.0.0");

        // pkg_c should have no changelog
        let changelog = workspace.read_changelog("pkg_c");
        assert!(
            changelog.is_empty(),
            "pkg_c should have no changelog since it wasn't affected"
        );
    }

    #[test]
    fn fixed_dependencies_without_actual_dependency() {
        // Test case: two packages in fixed group but no actual dependency between them
        // Should the auto-bumped package still show "Updated dependencies" message?
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            // Note: no dependency between a and b
            .set_config("[packages]\nfixed_dependencies = [[\"a\", \"b\"]]\n")
            .add_changeset(&["b"], Bump::Major, "breaking: b breaking change");

        workspace.run_release(false).unwrap();

        // Both should be bumped to 2.0.0 (same level as fixed dependencies)
        workspace.assert_crate_version("a", "2.0.0");
        workspace.assert_crate_version("b", "2.0.0");

        // The question: should 'a' have "Updated dependencies" message when
        // it doesn't actually depend on 'b'? Currently it won't because
        // apply_releases only adds dependency update messages for actual dependencies.

        // Let's verify this behavior
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 2.0.0");
        // This should NOT contain "Updated dependencies" since there's no actual dependency

        // Let's check what the actual changelog content is
        let changelog_content = workspace.read_changelog("a");
        println!("Changelog content for 'a':\n{}", changelog_content);

        // Package 'a' should have a changelog but with empty sections since no explicit changes
        assert!(!changelog_content.contains("Updated dependencies"));
        assert!(!changelog_content.contains("breaking: b breaking change"));

        // FIXED: Package 'a' should now have an explanation for why it was bumped!
        workspace.assert_changelog_contains("a", "Bumped due to fixed dependency group policy");
    }

    #[test]
    fn fixed_dependencies_complex_scenario() {
        // Test case: multiple packages in fixed group, some with dependencies, some without
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("pkg_a", "1.0.0") // In group but no changes, no dependencies
            .add_crate("pkg_b", "1.0.0") // In group with changeset
            .add_crate("pkg_c", "1.0.0") // In group, depends on pkg_d (outside group)
            .add_crate("pkg_d", "1.0.0") // Not in group but has changeset
            .add_dependency("pkg_c", "pkg_d", "1.0.0")
            .set_config("[packages]\nfixed_dependencies = [[\"pkg_a\", \"pkg_b\", \"pkg_c\"]]\n")
            .add_changeset(&["pkg_b"], Bump::Minor, "feat: pkg_b new feature")
            .add_changeset(&["pkg_d"], Bump::Patch, "fix: pkg_d bug fix");

        workspace.run_release(false).unwrap();

        // All packages in fixed group should be bumped to 1.1.0 (highest bump in group)
        workspace.assert_crate_version("pkg_a", "1.1.0");
        workspace.assert_crate_version("pkg_b", "1.1.0");
        workspace.assert_crate_version("pkg_c", "1.1.0");
        // pkg_d is bumped to 1.0.1 (its own patch changeset)
        workspace.assert_crate_version("pkg_d", "1.0.1");

        // Check changelog messages
        workspace.assert_changelog_contains("pkg_a", "Bumped due to fixed dependency group policy");
        workspace.assert_changelog_contains("pkg_b", "feat: pkg_b new feature");
        workspace.assert_changelog_contains("pkg_c", "Updated dependencies: pkg_d@1.0.1");
        workspace.assert_changelog_contains("pkg_d", "fix: pkg_d bug fix");
    }

    #[test]
    fn package_with_both_changeset_and_dependency_update() {
        // Test case: package has its own changeset AND gets dependency updates
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "0.1.0")
            .add_crate("b", "0.1.0")
            .add_dependency("a", "b", "0.1.0")
            .add_changeset(&["a"], Bump::Minor, "feat: a adds new feature")
            .add_changeset(&["b"], Bump::Patch, "fix: b bug fix");

        workspace.run_release(false).unwrap();

        // a should be bumped minor (0.2.0) due to its own changeset
        workspace.assert_crate_version("a", "0.2.0");
        // b should be bumped patch (0.1.1) due to its changeset
        workspace.assert_crate_version("b", "0.1.1");

        // a should have both its own message AND dependency update message
        workspace.assert_changelog_contains("a", "# a");
        workspace.assert_changelog_contains("a", "## 0.2.0");
        workspace.assert_changelog_contains("a", "feat: a adds new feature");
        workspace.assert_changelog_contains("a", "Updated dependencies: b@0.1.1");
    }

    /// Test the complete README scenario: multiple releases in sequence
    #[test]
    fn linked_dependencies_readme_scenario_complete() {
        let mut workspace = TestWorkspace::new();

        // Step 1: Initial state a@1.0.0 depends on b@1.0.0
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("a", "b", "1.0.0")
            .set_config("[packages]\nlinked_dependencies = [[\"a\", \"b\"]]\n");

        // Step 2: b is updated to 2.0.0 (major), a should also get 2.0.0
        workspace.add_changeset(&["b"], Bump::Major, "breaking: b major update");
        workspace.run_release(false).unwrap();

        workspace.assert_crate_version("a", "2.0.0");
        workspace.assert_crate_version("b", "2.0.0");
        workspace.assert_dependency_version("a", "b", "2.0.0");

        // Step 3: Manually update manifests to simulate progression
        // In real scenario, these would be updated by previous release
        let a_dir = workspace.crates.get("a").unwrap();
        let b_dir = workspace.crates.get("b").unwrap();

        fs::write(
            a_dir.join("Cargo.toml"),
            "[package]\nname=\"a\"\nversion=\"2.0.0\"\n\n[dependencies]\nb = { path=\"../b\", version=\"2.0.0\" }\n",
        ).unwrap();
        fs::write(
            b_dir.join("Cargo.toml"),
            "[package]\nname=\"b\"\nversion=\"2.0.0\"\n",
        )
        .unwrap();

        // Step 4: a is updated to 2.1.0 (minor), b should remain at 2.0.0
        workspace.add_changeset(&["a"], Bump::Minor, "feat: a minor update");
        workspace.run_release(false).unwrap();

        workspace.assert_crate_version("a", "2.1.0");
        workspace.assert_crate_version("b", "2.0.0"); // b not affected
    }
}
