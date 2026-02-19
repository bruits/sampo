#[cfg(test)]
mod tests {
    use rustc_hash::FxHashMap;
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::PathBuf,
        sync::{Mutex, MutexGuard, OnceLock},
    };

    use crate::*;

    /// Test workspace builder for reducing test boilerplate
    struct TestWorkspace {
        root: PathBuf,
        _temp_dir: tempfile::TempDir,
        crates: FxHashMap<String, PathBuf>,
    }

    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_MUTEX.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        original: Option<String>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let lock = env_lock().lock().unwrap();
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                key,
                original,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref value) = self.original {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    impl TestWorkspace {
        fn new() -> Self {
            let temp_dir = tempfile::tempdir().unwrap();
            let root = temp_dir.path().to_path_buf();

            {
                let _lock = env_lock().lock().unwrap();
                unsafe {
                    std::env::set_var("SAMPO_RELEASE_BRANCH", "main");
                }
            }

            // Create .sampo/ directory (required for discover_workspace)
            fs::create_dir_all(root.join(".sampo")).unwrap();

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
            fs::create_dir_all(crate_dir.join("src")).unwrap();

            fs::write(
                crate_dir.join("Cargo.toml"),
                format!("[package]\nname=\"{}\"\nversion=\"{}\"\n", name, version),
            )
            .unwrap();

            fs::write(
                crate_dir.join("src/lib.rs"),
                "pub fn __sampo_test_marker() {}\n",
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

        fn write_changeset_to_dir(
            dir: &std::path::Path,
            packages: &[&str],
            release: Bump,
            message: &str,
        ) {
            fs::create_dir_all(dir).unwrap();

            let mut frontmatter = String::from("---\n");
            for p in packages {
                frontmatter.push_str(&format!("{}: {}\n", p, release));
            }
            frontmatter.push_str("---\n\n");
            let content = format!("{}{}\n", frontmatter, message);

            let filename = message
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-')
                .collect::<String>()
                .to_lowercase()
                + ".md";

            fs::write(dir.join(filename), content).unwrap();
        }

        fn add_changeset(&self, packages: &[&str], release: Bump, message: &str) -> &Self {
            Self::write_changeset_to_dir(
                &self.root.join(".sampo/changesets"),
                packages,
                release,
                message,
            );
            self
        }

        fn add_preserved_changeset(
            &self,
            packages: &[&str],
            release: Bump,
            message: &str,
        ) -> &Self {
            Self::write_changeset_to_dir(
                &self.root.join(".sampo/prerelease"),
                packages,
                release,
                message,
            );
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

        fn set_publishable(&self, crate_name: &str, publishable: bool) -> &Self {
            let crate_dir = self.crates.get(crate_name).expect("crate must exist");
            let manifest_path = crate_dir.join("Cargo.toml");
            let current_manifest = fs::read_to_string(&manifest_path).unwrap();

            let new_manifest = if publishable {
                // Remove any publish = false lines (simple approach)
                current_manifest
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("publish = false"))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                let mut s = current_manifest;
                if !s.contains("publish = false") {
                    s.push_str("\npublish = false\n");
                }
                s
            };

            fs::write(manifest_path, new_manifest).unwrap();
            self
        }

        fn run_release(&self, dry_run: bool) -> crate::errors::Result<ReleaseOutput> {
            run_release(&self.root, dry_run)
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
    fn run_release_rejects_unconfigured_branch() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("foo", "0.1.0");
        workspace.set_config("[git]\nrelease_branches = [\"main\"]\n");

        let _guard = EnvVarGuard::set("SAMPO_RELEASE_BRANCH", "feature");
        let err = workspace.run_release(true).unwrap_err();
        match err {
            crate::errors::SampoError::Release(message) => {
                assert!(
                    message.contains("not configured for releases"),
                    "unexpected message: {message}"
                );
            }
            other => panic!("expected Release error, got {other:?}"),
        }
    }

    #[test]
    fn run_release_allows_configured_branch() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("foo", "0.1.0");
        workspace.set_config("[git]\nrelease_branches = [\"3.x\"]\n");

        let _guard = EnvVarGuard::set("SAMPO_RELEASE_BRANCH", "3.x");
        let output = workspace
            .run_release(true)
            .expect("release should succeed on configured branch");
        assert!(output.released_packages.is_empty());
        assert!(output.dry_run);
    }

    #[test]
    fn pre_release_preserves_changesets() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("foo", "1.0.0-alpha");
        workspace.add_changeset(&["foo"], Bump::Minor, "feat: alpha launch");

        workspace
            .run_release(false)
            .expect("pre-release should succeed");

        let changesets_dir = workspace.root.join(".sampo/changesets");
        let prerelease_dir = workspace.root.join(".sampo/prerelease");

        let pending = fs::read_dir(&changesets_dir)
            .unwrap()
            .map(|entry| entry.expect("dir entry"))
            .collect::<Vec<_>>();
        assert!(pending.is_empty(), "changesets directory should be empty");

        let preserved = fs::read_dir(&prerelease_dir)
            .unwrap()
            .map(|entry| entry.expect("dir entry"))
            .collect::<Vec<_>>();
        assert!(
            !preserved.is_empty(),
            "pre-release changesets should be preserved"
        );
    }

    #[test]
    fn final_release_restores_preserved_changesets() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("foo", "1.0.0-alpha");
        workspace.add_changeset(&["foo"], Bump::Major, "feat: release candidate");

        workspace
            .run_release(false)
            .expect("initial pre-release should succeed");

        // Simulate stabilizing the version before the official release
        let manifest = workspace
            .crates
            .get("foo")
            .expect("crate should exist")
            .join("Cargo.toml");
        fs::write(&manifest, "[package]\nname=\"foo\"\nversion=\"1.0.0\"\n")
            .expect("should rewrite manifest");

        workspace
            .run_release(false)
            .expect("final release should succeed");

        let changesets_dir = workspace.root.join(".sampo/changesets");
        let prerelease_dir = workspace.root.join(".sampo/prerelease");

        let remaining = fs::read_dir(&changesets_dir)
            .unwrap()
            .map(|entry| entry.expect("dir entry"))
            .collect::<Vec<_>>();
        assert!(remaining.is_empty(), "all changesets should be consumed");

        let preserved = fs::read_dir(&prerelease_dir)
            .unwrap()
            .map(|entry| entry.expect("dir entry"))
            .collect::<Vec<_>>();
        assert!(
            preserved.is_empty(),
            "pre-release cache should be empty after final release"
        );

        workspace.assert_crate_version("foo", "2.0.0");
    }

    #[test]
    fn preserved_changesets_do_not_retrigger_prerelease_bump() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("foo", "0.1.0-alpha.1");
        workspace.add_preserved_changeset(&["foo"], Bump::Minor, "Added some feature");

        let output = workspace.run_release(true).expect("release should succeed");

        assert!(
            output.released_packages.is_empty(),
            "preserved changesets should NOT trigger a new prerelease bump"
        );
    }

    #[test]
    fn preserved_changesets_proceed_when_target_is_stable() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0-alpha.1")
            .add_crate("b", "2.0.0");
        workspace.add_preserved_changeset(&["b"], Bump::Minor, "Added feature for b");

        let output = workspace
            .run_release(false)
            .expect("release should succeed");

        assert_eq!(output.released_packages.len(), 1);
        assert_eq!(output.released_packages[0].name, "b");
        assert_eq!(output.released_packages[0].new_version, "2.1.0");

        workspace.assert_crate_version("a", "1.0.0-alpha.1");
    }

    #[test]
    fn preserved_changesets_skip_when_all_targets_prerelease() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0-alpha.1")
            .add_crate("b", "2.0.0");
        workspace.add_preserved_changeset(&["a"], Bump::Minor, "Added feature for a");

        let output = workspace.run_release(true).expect("release should succeed");

        assert!(
            output.released_packages.is_empty(),
            "should skip when all preserved targets are in prerelease"
        );
    }

    #[test]
    fn preserved_changesets_proceed_when_mixed_targets() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0-alpha.1")
            .add_crate("b", "2.0.0");
        workspace.add_preserved_changeset(&["a", "b"], Bump::Minor, "Added shared feature");

        let output = workspace
            .run_release(false)
            .expect("release should succeed");

        let released_names: Vec<&str> = output
            .released_packages
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(
            released_names.contains(&"b"),
            "stable package B should be released"
        );
        assert!(
            !released_names.contains(&"a"),
            "prerelease package A should NOT be released (entry stays preserved)"
        );

        workspace.assert_crate_version("b", "2.1.0");
        workspace.assert_crate_version("a", "1.0.0-alpha.1");

        // The prerelease entry for 'a' should remain in the prerelease dir
        let prerelease_dir = workspace.root.join(".sampo/prerelease");
        let preserved_files: Vec<_> = fs::read_dir(&prerelease_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
            .collect();
        assert_eq!(
            preserved_files.len(),
            1,
            "one preserved changeset should remain for prerelease entry"
        );

        // Verify the preserved file only contains the prerelease entry for 'a'
        let preserved_content = fs::read_to_string(preserved_files[0].path()).unwrap();
        assert!(
            preserved_content.contains("a:"),
            "preserved file should contain entry for 'a'"
        );
        assert!(
            !preserved_content.contains("b:"),
            "preserved file should NOT contain entry for 'b'"
        );
    }

    #[test]
    fn preserved_changesets_skip_when_all_workspace_prerelease() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0-alpha.1")
            .add_crate("b", "2.0.0-beta.1");
        workspace.add_preserved_changeset(&["a", "b"], Bump::Minor, "Added cross feature");

        let output = workspace.run_release(true).expect("release should succeed");

        assert!(
            output.released_packages.is_empty(),
            "should skip when all targets in preserved changesets are in prerelease"
        );
    }

    #[test]
    fn preserved_mixed_changeset_keeps_prerelease_entries_preserved() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0-alpha.1")
            .add_crate("b", "2.0.0")
            .add_crate("c", "3.0.0");
        // Mixed changeset: a (prerelease) + b (stable)
        workspace.add_preserved_changeset(&["a", "b"], Bump::Minor, "Added shared feature");
        // Pure stable changeset for c
        workspace.add_preserved_changeset(&["c"], Bump::Patch, "Fixed c bug");

        let output = workspace
            .run_release(false)
            .expect("release should succeed");

        let released_names: Vec<&str> = output
            .released_packages
            .iter()
            .map(|p| p.name.as_str())
            .collect();

        // b and c should be released, a should not
        assert!(released_names.contains(&"b"), "b should be released");
        assert!(released_names.contains(&"c"), "c should be released");
        assert!(!released_names.contains(&"a"), "a should NOT be released");

        workspace.assert_crate_version("b", "2.1.0");
        workspace.assert_crate_version("c", "3.0.1");
        workspace.assert_crate_version("a", "1.0.0-alpha.1");

        // The prerelease entry for a should remain preserved
        let prerelease_dir = workspace.root.join(".sampo/prerelease");
        let preserved_files: Vec<_> = fs::read_dir(&prerelease_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
            .collect();
        assert_eq!(
            preserved_files.len(),
            1,
            "one file should remain in prerelease dir (the rewritten mixed changeset)"
        );

        let preserved_content = fs::read_to_string(preserved_files[0].path()).unwrap();
        assert!(
            preserved_content.contains("a:"),
            "preserved file should still have entry for prerelease package a"
        );
        assert!(
            !preserved_content.contains("b:"),
            "preserved file should NOT have entry for stable package b"
        );

        // Changesets dir should be empty (all consumed)
        let changesets_dir = workspace.root.join(".sampo/changesets");
        let remaining: Vec<_> = fs::read_dir(&changesets_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
            .collect();
        assert!(
            remaining.is_empty(),
            "changesets dir should be empty after release"
        );
    }

    #[test]
    fn preserved_mixed_changeset_dry_run_only_plans_stable() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0-alpha.1")
            .add_crate("b", "2.0.0");
        workspace.add_preserved_changeset(&["a", "b"], Bump::Minor, "Added shared feature");

        let output = workspace
            .run_release(true)
            .expect("dry-run release should succeed");

        let released_names: Vec<&str> = output
            .released_packages
            .iter()
            .map(|p| p.name.as_str())
            .collect();

        assert!(
            released_names.contains(&"b"),
            "stable package b should appear in dry-run plan"
        );
        assert!(
            !released_names.contains(&"a"),
            "prerelease package a should NOT appear in dry-run plan"
        );
        assert!(output.dry_run);
    }

    #[test]
    fn prerelease_second_changeset_bumps_again() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("pkg1", "1.0.0-alpha");
        workspace.add_crate("pkg2", "1.0.0");
        workspace.add_crate("pkg3", "1.0.0");
        workspace.add_changeset(&["pkg1"], Bump::Minor, "feat: first prerelease change");

        let output = workspace
            .run_release(false)
            .expect("first release should succeed");
        assert_eq!(output.released_packages.len(), 1);
        assert_eq!(output.released_packages[0].name, "pkg1");
        workspace.assert_crate_version("pkg1", "1.0.0-alpha.1");

        let prerelease_dir = workspace.root.join(".sampo/prerelease");
        assert!(prerelease_dir.exists());

        workspace.add_changeset(&["pkg1"], Bump::Patch, "fix: second prerelease fix");

        let output = workspace
            .run_release(false)
            .expect("second release should succeed");
        assert_eq!(output.released_packages.len(), 1);
        assert_eq!(output.released_packages[0].name, "pkg1");
        workspace.assert_crate_version("pkg1", "1.0.0-alpha.2");
        workspace.assert_crate_version("pkg2", "1.0.0");
        workspace.assert_crate_version("pkg3", "1.0.0");
    }

    #[test]
    fn new_stable_changeset_with_preserved_prerelease_changesets() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("pkg1", "1.0.0-alpha");
        workspace.add_crate("pkg2", "1.0.0");
        workspace.add_crate("pkg3", "1.0.0");
        workspace.add_changeset(&["pkg1"], Bump::Minor, "feat: prerelease feature");

        workspace
            .run_release(false)
            .expect("prerelease should succeed");
        workspace.assert_crate_version("pkg1", "1.0.0-alpha.1");

        let prerelease_dir = workspace.root.join(".sampo/prerelease");
        assert!(prerelease_dir.exists());

        workspace.add_changeset(&["pkg2"], Bump::Patch, "fix: stable bug fix");

        let output = workspace
            .run_release(false)
            .expect("release should succeed");
        let released_names: Vec<&str> = output
            .released_packages
            .iter()
            .map(|p| p.name.as_str())
            .collect();

        assert!(
            released_names.contains(&"pkg2"),
            "pkg2 should be released, got: {:?}",
            released_names
        );
        workspace.assert_crate_version("pkg2", "1.0.1");
        workspace.assert_crate_version("pkg3", "1.0.0");
    }

    #[test]
    fn preserved_changesets_after_exit_prerelease_proceeds() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("pkg1", "1.0.0");
        workspace.add_crate("pkg2", "2.0.0");
        workspace.add_preserved_changeset(&["pkg1"], Bump::Minor, "feat: from prerelease era");

        let output = workspace
            .run_release(false)
            .expect("release should succeed");

        assert_eq!(output.released_packages.len(), 1);
        assert_eq!(output.released_packages[0].name, "pkg1");
        workspace.assert_crate_version("pkg1", "1.1.0");
    }

    #[test]
    fn switching_prerelease_label_restores_changesets() {
        let mut workspace = TestWorkspace::new();
        let _guard = EnvVarGuard::set("SAMPO_RELEASE_BRANCH", "main");
        workspace.add_crate("foo", "1.0.0-alpha");
        workspace.add_changeset(&["foo"], Bump::Minor, "feat: alpha foundation");

        workspace
            .run_release(false)
            .expect("initial alpha pre-release should succeed");

        workspace.assert_crate_version("foo", "1.0.0-alpha.1");

        let packages = vec![String::from("foo")];
        let exit_updates = exit_prerelease(&workspace.root, &packages).unwrap();
        assert!(!exit_updates.is_empty());

        let restored = restore_preserved_changesets(&workspace.root).unwrap();
        assert_eq!(restored, 1);

        enter_prerelease(&workspace.root, &packages, "beta").unwrap();

        workspace
            .run_release(false)
            .expect("beta pre-release should succeed");

        workspace.assert_crate_version("foo", "1.0.0-beta.1");

        let changelog_path = workspace
            .crates
            .get("foo")
            .expect("crate should exist")
            .join("CHANGELOG.md");
        let changelog = fs::read_to_string(changelog_path).expect("changelog to exist");

        assert!(
            changelog.contains("## 1.0.0-beta.1"),
            "expected changelog to include beta entry"
        );
        assert!(
            changelog.contains("feat: alpha foundation"),
            "expected changelog to include preserved alpha changes"
        );
    }

    #[test]
    fn bumps_versions() {
        assert_eq!(bump_version("0.0.0", Bump::Patch).unwrap(), "0.0.1");
        assert_eq!(bump_version("0.1.2", Bump::Minor).unwrap(), "0.2.0");
        assert_eq!(bump_version("1.2.3", Bump::Major).unwrap(), "2.0.0");
        assert_eq!(bump_version("1.2", Bump::Patch).unwrap(), "1.2.1");
    }

    #[test]
    fn bumps_prerelease_versions() {
        assert_eq!(
            bump_version("1.8.0-alpha", Bump::Patch).unwrap(),
            "1.8.0-alpha.1"
        );
        assert_eq!(
            bump_version("1.8.0-alpha.1", Bump::Minor).unwrap(),
            "1.8.0-alpha.2"
        );
        assert_eq!(
            bump_version("1.8.0-alpha.2", Bump::Major).unwrap(),
            "2.0.0-alpha"
        );
        assert_eq!(
            bump_version("2.0.0-beta.3", Bump::Major).unwrap(),
            "2.0.0-beta.4"
        );
        assert_eq!(
            bump_version("1.2.3-alpha", Bump::Minor).unwrap(),
            "1.3.0-alpha"
        );
    }

    #[test]
    fn rejects_numeric_only_prerelease_when_escalating() {
        let err = bump_version("1.2.3-1", Bump::Minor).unwrap_err();
        assert!(
            err.contains("must include a non-numeric identifier"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn updates_version_in_toml() {
        let input = "[package]\nname=\"x\"\nversion = \"0.1.0\"\n\n[dependencies]\n";
        let new_versions = BTreeMap::new();
        let manifest_path = std::path::Path::new("/test/crates/x/Cargo.toml");
        let (out, _) = crate::adapters::PackageAdapter::Cargo
            .update_manifest_versions(manifest_path, input, Some("0.2.0"), &new_versions, None)
            .unwrap();
        assert!(out.contains("version = \"0.2.0\""));
        assert!(out.contains("[dependencies]"));
    }

    #[test]
    fn preserves_original_formatting() {
        let input = r#"[package]
name = "sampo-github-action"
version = "0.1.0"
license = "MIT"
authors = ["Goulven Clech <goulven.clech@protonmail.com>"]
edition = "2024"
description = "GitHub Action runner for Sampo CLI (release/publish orchestrator)"
homepage = "https://github.com/bruits/sampo"
repository = "https://github.com/bruits/sampo"
readme = "README.md"
keywords = ["changeset", "versioning", "publishing", "semver", "monorepo"]
categories = ["development-tools"]

[dependencies]
sampo-core = { version = "0.2.0", path = "../sampo-core" }
clap = { version = "4.5", features = ["derive"] }
thiserror = "1.0"
toml = "0.8"
rustc-hash = "2.0"

[dev-dependencies]
tempfile = "3.0"
"#;

        let new_versions = BTreeMap::new();
        let manifest_path = std::path::Path::new("/test/crates/sampo-github-action/Cargo.toml");
        let (out, _) = crate::adapters::PackageAdapter::Cargo
            .update_manifest_versions(manifest_path, input, Some("0.2.0"), &new_versions, None)
            .unwrap();

        // Should update version but preserve all other formatting
        assert!(out.contains("version = \"0.2.0\""));
        assert!(out.contains("license = \"MIT\""));
        assert!(out.contains("authors = [\"Goulven Clech <goulven.clech@protonmail.com>\"]"));
        assert!(out.contains("clap = { version = \"4.5\", features = [\"derive\"] }"));
        assert!(out.contains("sampo-core = { version = \"0.2.0\", path = \"../sampo-core\" }"));

        // Check that sections remain in original order
        let package_pos = out.find("[package]").unwrap();
        let deps_pos = out.find("[dependencies]").unwrap();
        let dev_deps_pos = out.find("[dev-dependencies]").unwrap();
        assert!(package_pos < deps_pos);
        assert!(deps_pos < dev_deps_pos);
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
            .set_config("[packages]\nfixed = [[\"a\", \"b\"]]\n")
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
            .set_config("[packages]\nfixed = [[\"a\", \"b\"]]\n")
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
            .set_config("[packages]\nfixed = [[\"a\", \"b\"], [\"c\", \"d\"]]\n")
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
            .set_config("[packages]\nfixed = [[\"a\", \"nonexistent\"]]\n");

        let result = workspace.run_release(false);
        assert!(result.is_err());
        let error_msg = format!("{}", result.unwrap_err());
        assert!(error_msg.contains("packages.fixed group 1"));
        assert!(error_msg.contains("package 'nonexistent' not found in the workspace"));
    }

    #[test]
    fn linked_dependencies_basic_scenario() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0")
            .add_crate("b", "1.0.0")
            .add_dependency("a", "b", "1.0.0") // a depends on b
            .set_config("[packages]\nlinked = [[\"a\", \"b\"]]\n")
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
            .set_config("[packages]\nlinked = [[\"a\", \"b\", \"c\"]]\n")
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
            .set_config("[packages]\nlinked = [[\"a\", \"b\", \"c\"]]\n")
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
            .set_config("[packages]\nlinked = [[\"affected_directly\", \"affected_by_cascade\", \"unaffected_in_group\"]]\n")
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
            .add_crate("pkg_a", "1.0.0") // Has major changeset
            .add_crate("pkg_b", "1.0.0") // Has minor changeset
            .add_crate("pkg_c", "1.0.0") // In group but no changeset, no deps
            .add_crate("pkg_d", "1.0.0") // Depends on pkg_a
            .add_dependency("pkg_d", "pkg_a", "1.0.0")
            .set_config("[packages]\nlinked = [[\"pkg_a\", \"pkg_b\", \"pkg_c\", \"pkg_d\"]]\n")
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
            .set_config("[packages]\nfixed = [[\"a\", \"b\"]]\n")
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
            .set_config("[packages]\nfixed = [[\"pkg_a\", \"pkg_b\", \"pkg_c\"]]\n")
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
            .set_config("[packages]\nlinked = [[\"a\", \"b\"]]\n");

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
    fn formats_dependency_updates_with_canonical_identifiers() {
        let updates = vec![DependencyUpdate {
            name: "cargo/pkg1".to_string(),
            new_version: "1.2.0".to_string(),
        }];
        let msg = format_dependency_updates_message(&updates).unwrap();
        assert_eq!(msg, "Updated dependencies: pkg1@1.2.0");
    }

    #[test]
    fn formats_dependency_updates_with_ambiguous_ecosystems() {
        let updates = vec![
            DependencyUpdate {
                name: "cargo/shared".to_string(),
                new_version: "1.1.0".to_string(),
            },
            DependencyUpdate {
                name: "npm/shared".to_string(),
                new_version: "2.0.0".to_string(),
            },
        ];
        let msg = format_dependency_updates_message(&updates).unwrap();
        assert_eq!(
            msg,
            "Updated dependencies: cargo/shared@1.1.0, npm/shared@2.0.0"
        );
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
    fn release_consumes_changesets_with_quoted_package_names() {
        let mut workspace = TestWorkspace::new();
        workspace.add_crate("sampo-core", "0.1.0");

        let changesets_dir = workspace.root.join(".sampo/changesets");
        fs::create_dir_all(&changesets_dir).unwrap();
        fs::write(
            changesets_dir.join("quoted.md"),
            "---\n\"sampo-core\": minor\n---\n\nfeat: quoted release\n",
        )
        .unwrap();

        let _guard = EnvVarGuard::set("SAMPO_RELEASE_BRANCH", "main");
        let result = workspace.run_release(false).unwrap();

        assert_eq!(result.released_packages.len(), 1);
        let released = &result.released_packages[0];
        assert_eq!(released.name, "sampo-core");
        assert_eq!(released.bump, Bump::Minor);

        workspace.assert_crate_version("sampo-core", "0.2.0");
    }

    #[test]
    fn ignores_unpublished_packages_when_configured() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("public", "1.0.0")
            .add_crate("private", "1.0.0");

        // Mark private as non-publishable
        workspace.set_publishable("private", false);

        // Configure to ignore unpublished packages
        workspace.set_config("[packages]\nignore_unpublished = true\n");

        // Add changesets for both
        workspace
            .add_changeset(&["public"], Bump::Patch, "fix: public bug")
            .add_changeset(&["private"], Bump::Patch, "fix: private bug");

        let result = workspace.run_release(false).unwrap();
        // Only one package should be released
        assert_eq!(result.released_packages.len(), 1);
        assert_eq!(result.released_packages[0].name, "public");

        // Verify versions: public bumped, private unchanged
        workspace.assert_crate_version("public", "1.0.1");
        workspace.assert_crate_version("private", "1.0.0");

        // Verify that the changeset for private was NOT consumed (still present)
        let changesets_dir = workspace.root.join(".sampo/changesets");
        let remaining_files: Vec<_> = std::fs::read_dir(&changesets_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();
        // One changeset should remain (the private one)
        assert_eq!(remaining_files.len(), 1);
    }

    #[test]
    fn ignores_specific_packages_by_pattern() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("internal-tool", "0.1.0")
            .add_crate("example-lib", "0.1.0")
            .add_crate("normal-lib", "0.1.0");

        // Configure ignore patterns (by name)
        workspace.set_config("[packages]\nignore = [\"internal-*\", \"example-*\"]\n");

        // Add one changeset that only targets ignored packages
        workspace.add_changeset(
            &["internal-tool", "example-lib"],
            Bump::Patch,
            "ignored changes",
        );
        // And one for a normal package
        workspace.add_changeset(&["normal-lib"], Bump::Minor, "feat: normal update");

        let out = workspace.run_release(false).unwrap();

        // Only normal-lib should be released
        assert_eq!(out.released_packages.len(), 1);
        assert_eq!(out.released_packages[0].name, "normal-lib");

        // Versions: normal updated, ignored unchanged
        workspace.assert_crate_version("normal-lib", "0.2.0");
        workspace.assert_crate_version("internal-tool", "0.1.0");
        workspace.assert_crate_version("example-lib", "0.1.0");

        // The changeset that only targeted ignored packages should remain on disk
        let changesets_dir = workspace.root.join(".sampo/changesets");
        let remaining_files: Vec<_> = std::fs::read_dir(&changesets_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();
        // After consuming the normal changeset, one ignored-only changeset should remain
        assert_eq!(remaining_files.len(), 1);
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
        // Create test workspace with dependencies
        let ws = Workspace {
            root: PathBuf::from("/test"),
            members: vec![
                PackageInfo {
                    name: "pkg-a".to_string(),
                    identifier: "cargo/pkg-a".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-a"),
                    internal_deps: BTreeSet::from(["cargo/pkg-b".to_string()]),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "pkg-b".to_string(),
                    identifier: "cargo/pkg-b".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-b"),
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "pkg-c".to_string(),
                    identifier: "cargo/pkg-c".to_string(),
                    version: "1.0.0".to_string(),
                    path: PathBuf::from("/test/pkg-c"),
                    internal_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
            ],
        };

        // Create config with fixed dependencies
        let config = Config {
            version: 1,
            github_repository: None,
            changelog_show_commit_hash: true,
            changelog_show_acknowledgments: true,
            changelog_show_release_date: true,
            changelog_release_date_format: "%Y-%m-%d".to_string(),
            changelog_release_date_timezone: None,
            changesets_tags: vec![],
            fixed_dependencies: vec![vec!["pkg-a".to_string(), "pkg-c".to_string()]],
            linked_dependencies: vec![],
            ignore_unpublished: false,
            ignore: vec![],
            git_default_branch: None,
            git_release_branches: Vec::new(),
            git_short_tags: None,
        };

        // Create changeset that affects pkg-b only
        let changesets = vec![ChangesetInfo {
            entries: vec![(
                crate::types::PackageSpecifier::parse("pkg-b").unwrap(),
                Bump::Minor,
                None,
            )],
            message: "feat: new feature".to_string(),
            path: PathBuf::from("/test/.sampo/changesets/test.md"),
        }];

        // Simulate releases: pkg-a and pkg-c get fixed bump, pkg-b gets direct bump
        let mut releases = BTreeMap::new();
        releases.insert(
            "cargo/pkg-a".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );
        releases.insert(
            "cargo/pkg-b".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );
        releases.insert(
            "cargo/pkg-c".to_string(),
            ("1.0.0".to_string(), "1.1.0".to_string()),
        );

        let explanations =
            detect_all_dependency_explanations(&changesets, &ws, &config, &releases).unwrap();

        // pkg-a should have dependency update message (depends on pkg-b)
        let pkg_a_messages = explanations.get("cargo/pkg-a").unwrap();
        assert_eq!(pkg_a_messages.len(), 1);
        assert!(
            pkg_a_messages[0]
                .0
                .contains("Updated dependencies: pkg-b@1.1.0")
        );
        assert_eq!(pkg_a_messages[0].1, ChangelogCategory::Bump(Bump::Patch));

        // pkg-c should have fixed dependency policy message (no deps but in fixed group)
        let pkg_c_messages = explanations.get("cargo/pkg-c").unwrap();
        assert_eq!(pkg_c_messages.len(), 1);
        assert_eq!(
            pkg_c_messages[0].0,
            "Bumped due to fixed dependency group policy"
        );
        assert_eq!(pkg_c_messages[0].1, ChangelogCategory::Bump(Bump::Minor)); // Inferred from version change

        // pkg-b should have no messages (explicit changeset)
        assert!(!explanations.contains_key("cargo/pkg-b"));
    }

    #[test]
    fn detect_all_dependency_explanations_empty_cases() {
        let ws = Workspace {
            root: PathBuf::from("/test"),
            members: vec![PackageInfo {
                name: "pkg-a".to_string(),
                identifier: "cargo/pkg-a".to_string(),
                version: "1.0.0".to_string(),
                path: PathBuf::from("/test/pkg-a"),
                internal_deps: BTreeSet::new(),
                kind: PackageKind::Cargo,
            }],
        };

        let config = Config::default();
        let changesets = vec![];
        let releases = BTreeMap::new();

        let explanations =
            detect_all_dependency_explanations(&changesets, &ws, &config, &releases).unwrap();
        assert!(explanations.is_empty());
    }

    #[test]
    fn preserved_mixed_changeset_does_not_overwrite_existing_changeset() {
        let mut workspace = TestWorkspace::new();
        workspace
            .add_crate("a", "1.0.0-alpha.1")
            .add_crate("b", "2.0.0");

        // Mixed preserved changeset: a (prerelease) + b (stable)
        workspace.add_preserved_changeset(&["a", "b"], Bump::Minor, "Added shared feature");

        // Place a pre-existing changeset in changesets_dir with the SAME filename
        let changesets_dir = workspace.root.join(".sampo/changesets");
        fs::create_dir_all(&changesets_dir).unwrap();
        let colliding_name = "addedsharedfeature.md";
        let original_content = "---\nb: patch\n---\n\nOriginal unrelated changeset\n";
        fs::write(changesets_dir.join(colliding_name), original_content).unwrap();

        let output = workspace
            .run_release(false)
            .expect("release should succeed");

        let released_names: Vec<&str> = output
            .released_packages
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(released_names.contains(&"b"), "b should be released");
        assert!(!released_names.contains(&"a"), "a should NOT be released");

        // The minor bump from the preserved changeset wins over the patch from
        // the original, so b ends up at 2.1.0.
        workspace.assert_crate_version("b", "2.1.0");

        // The real invariant: both changeset messages must appear in b's
        // changelog. If the stable split had overwritten the pre-existing file,
        // "Original unrelated changeset" would be lost.
        workspace.assert_changelog_contains("b", "Original unrelated changeset");
        workspace.assert_changelog_contains("b", "Added shared feature");

        // Prerelease entry for a should remain preserved
        let prerelease_dir = workspace.root.join(".sampo/prerelease");
        let preserved_files: Vec<_> = fs::read_dir(&prerelease_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
            .collect();
        assert_eq!(
            preserved_files.len(),
            1,
            "prerelease dir should keep the rewritten mixed changeset"
        );
    }
}
