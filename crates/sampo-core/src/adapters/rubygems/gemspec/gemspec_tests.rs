use super::*;
use std::collections::BTreeMap;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
}

/// The `bundle gem` scaffold: a gemspec reading `Foo::VERSION` from `lib/foo/version.rb`.
fn scaffold_gem(dir: &Path, name: &str, module: &str, version: &str, deps: &str) {
    let path = name.replace('-', "/");
    write_file(
        &dir.join(format!("{name}.gemspec")),
        &format!(
            "# frozen_string_literal: true\n\nrequire_relative \"lib/{path}/version\"\n\n\
             Gem::Specification.new do |spec|\n  spec.name = \"{name}\"\n  \
             spec.version = {module}::VERSION\n  spec.summary = \"A gem\"\n{deps}end\n"
        ),
    );
    let modules: Vec<&str> = module.split("::").collect();
    let mut body = String::from("# frozen_string_literal: true\n\n");
    for (depth, m) in modules.iter().enumerate() {
        body.push_str(&format!("{}module {m}\n", "  ".repeat(depth)));
    }
    body.push_str(&format!(
        "{}VERSION = \"{version}\"\n",
        "  ".repeat(modules.len())
    ));
    for depth in (0..modules.len()).rev() {
        body.push_str(&format!("{}end\n", "  ".repeat(depth)));
    }
    write_file(&dir.join("lib").join(path).join("version.rb"), &body);
}

fn versions(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn rewrite_deps(gemspec: &str, pairs: &[(&str, &str)]) -> String {
    update_manifest_versions(Path::new("x.gemspec"), gemspec, None, &versions(pairs))
        .unwrap()
        .0
}

#[test]
fn discovers_scaffolded_gem_with_namespaced_version_constant() {
    let temp = tempfile::tempdir().unwrap();
    scaffold_gem(temp.path(), "foo-bar", "Foo::Bar", "0.3.1", "");

    assert!(can_discover(temp.path()));
    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "foo-bar");
    assert_eq!(packages[0].version, "0.3.1");
    assert_eq!(packages[0].identifier, "rubygems/foo-bar");
    assert_eq!(packages[0].path, temp.path());
}

#[test]
fn discovers_literal_and_positional_versions() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("lit/lit.gemspec"),
        "Gem::Specification.new do |s|\n  s.name = %q{lit}.freeze\n  s.version = \"2.3.9\".freeze\nend\n",
    );
    write_file(
        &temp.path().join("pos/pos.gemspec"),
        "Gem::Specification.new 'pos', '1.0' do |s|\nend\n",
    );

    let mut packages = discover(temp.path()).unwrap();
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    let found: Vec<(&str, &str)> = packages
        .iter()
        .map(|p| (p.name.as_str(), p.version.as_str()))
        .collect();
    assert_eq!(found, vec![("lit", "2.3.9"), ("pos", "1.0")]);
}

#[test]
fn discovery_links_runtime_and_development_siblings() {
    let temp = tempfile::tempdir().unwrap();
    scaffold_gem(&temp.path().join("core"), "core", "Core", "1.0.0", "");
    scaffold_gem(&temp.path().join("tools"), "tools", "Tools", "1.0.0", "");
    scaffold_gem(
        &temp.path().join("app"),
        "app",
        "App",
        "0.1.0",
        "  spec.add_dependency \"core\", \"~> 1.0\"\n  spec.add_dependency \"rack\"\n  \
         spec.add_development_dependency \"tools\"\n",
    );

    let packages = discover(temp.path()).unwrap();
    let app = packages.iter().find(|p| p.name == "app").unwrap();
    assert_eq!(
        app.internal_deps,
        BTreeSet::from(["rubygems/core".to_string()])
    );
    assert_eq!(
        app.internal_dev_deps,
        BTreeSet::from(["rubygems/tools".to_string()])
    );
}

#[test]
fn discovery_skips_unmanageable_versions_and_excluded_dirs() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("computed/computed.gemspec"),
        "Gem::Specification.new do |s|\n  s.name = \"computed\"\n  \
         s.version = File.read(\"VERSION\").strip\nend\n",
    );
    // Four numeric segments have no SemVer equivalent.
    write_file(
        &temp.path().join("four/four.gemspec"),
        "Gem::Specification.new do |s|\n  s.name = \"four\"\n  s.version = \"1.2.3.4\"\nend\n",
    );
    write_file(
        &temp.path().join("method/method.gemspec"),
        "require_relative \"lib/method/version\"\nGem::Specification.new do |s|\n  \
         s.name = \"method\"\n  s.version = Method::VERSION\nend\n",
    );
    write_file(
        &temp.path().join("method/lib/method/version.rb"),
        "module Method\n  VERSION = Version.version\nend\n",
    );
    scaffold_gem(
        &temp.path().join("vendor/gems/dep"),
        "dep",
        "Dep",
        "1.0.0",
        "",
    );
    scaffold_gem(
        &temp.path().join("pkg/built"),
        "built",
        "Built",
        "1.0.0",
        "",
    );

    assert!(discover(temp.path()).unwrap().is_empty());
}

#[test]
fn finds_version_constant_through_require_and_class_nesting() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("rubocop.gemspec"),
        "$LOAD_PATH.unshift File.expand_path('lib', __dir__)\nrequire 'rubocop/version'\n\
         Gem::Specification.new do |s|\n  s.name = 'rubocop'\n  \
         s.version = RuboCop::Version::STRING\nend\n",
    );
    write_file(
        &temp.path().join("lib/rubocop/version.rb"),
        "module RuboCop\n  module Version\n    STRING = '1.81.0'\n    \
         MINIMUM_PARSABLE_PRISM_VERSION = 3.3\n  end\nend\n",
    );

    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].version, "1.81.0");
}

#[test]
fn hand_written_prerelease_maps_to_semver_and_back() {
    assert_eq!(gem_to_semver("1.0.0.rc1").as_deref(), Some("1.0.0-rc1"));
    assert_eq!(gem_to_semver("8.2.0.alpha").as_deref(), Some("8.2.0-alpha"));
    assert_eq!(gem_to_semver("1.0.0.pre.1").as_deref(), Some("1.0.0-pre.1"));
    assert_eq!(gem_to_semver("1.0.0-rc.1").as_deref(), Some("1.0.0-rc.1"));
    assert_eq!(gem_to_semver("1.2").as_deref(), Some("1.2"));
    assert_eq!(gem_to_semver("1.2.3.4"), None);
    assert_eq!(gem_to_semver("1.0.0.rc.01"), None);
    assert_eq!(semver_to_gem("1.1.0-rc.1"), "1.1.0.rc.1");
    assert_eq!(semver_to_gem("1.1.0"), "1.1.0");
    for gem in ["1.0.0.rc1", "1.0.0.pre.1", "2.0.0.beta.3"] {
        assert_eq!(semver_to_gem(&gem_to_semver(gem).unwrap()), gem);
    }
}

#[test]
fn published_version_follows_rubygems_hyphen_rule() {
    let temp = tempfile::tempdir().unwrap();
    scaffold_gem(temp.path(), "foo", "Foo", "1.0.0-alpha.1", "");
    let gemspec = temp.path().join("foo.gemspec");
    assert_eq!(
        published_version(&gemspec, "1.0.0-alpha.1"),
        "1.0.0.pre.alpha.1"
    );

    scaffold_gem(temp.path(), "foo", "Foo", "1.0.0.rc1", "");
    assert_eq!(published_version(&gemspec, "1.0.0-rc1"), "1.0.0.rc1");
}

#[test]
fn literal_gemspec_version_is_rewritten_in_place() {
    let input = "Gem::Specification.new do |s|\n  s.name = \"lit\".freeze\n  \
                 s.version = \"2.3.9\".freeze\nend\n";
    let (output, _) = update_manifest_versions(
        Path::new("lit.gemspec"),
        input,
        Some("2.4.0-rc.1"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        output,
        "Gem::Specification.new do |s|\n  s.name = \"lit\".freeze\n  \
         s.version = \"2.4.0.rc.1\".freeze\nend\n"
    );
}

#[test]
fn version_constant_is_written_by_finalize() {
    let temp = tempfile::tempdir().unwrap();
    scaffold_gem(temp.path(), "foo", "Foo", "1.0.0", "");
    let packages = discover(temp.path()).unwrap();
    let gemspec_path = temp.path().join("foo.gemspec");
    let gemspec_before = fs::read_to_string(&gemspec_path).unwrap();

    let (gemspec_after, _) = update_manifest_versions(
        &gemspec_path,
        &gemspec_before,
        Some("1.1.0"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(gemspec_after, gemspec_before);

    finalize_version_files(&packages, &versions(&[("rubygems/foo", "1.1.0")])).unwrap();
    let version_rb = fs::read_to_string(temp.path().join("lib/foo/version.rb")).unwrap();
    assert!(version_rb.contains("VERSION = \"1.1.0\"\n"), "{version_rb}");
}

#[test]
fn finalize_writes_only_the_referenced_constant() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        &temp.path().join("hanami.gemspec"),
        "require_relative \"lib/hanami/version\"\nGem::Specification.new do |s|\n  \
         s.name = \"hanami\"\n  s.version = Hanami::Version::VERSION.dup\nend\n",
    );
    let version_rb = "module Hanami\n  module Version\n    VERSION = '2.0.0'.freeze\n  end\n\n  \
                      VERSION = Version::VERSION\nend\n";
    write_file(&temp.path().join("lib/hanami/version.rb"), version_rb);
    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages[0].version, "2.0.0");

    finalize_version_files(&packages, &versions(&[("rubygems/hanami", "2.1.0")])).unwrap();
    assert_eq!(
        fs::read_to_string(temp.path().join("lib/hanami/version.rb")).unwrap(),
        version_rb.replace("'2.0.0'", "'2.1.0'")
    );
}

#[test]
fn satisfied_requirements_stay_untouched() {
    let input = "s.add_dependency \"core\", \"~> 1.2\", \">= 1.2.3\"\n\
                 s.add_dependency \"other\", \"= 1.0.0\"\n";
    assert_eq!(rewrite_deps(input, &[("core", "1.9.0")]), input);
}

#[test]
fn unsatisfied_requirements_follow_the_new_version() {
    let input = "s.add_dependency \"pin\", \"= 1.0.0\"\n\
                 s.add_dependency \"bare\", \"1.0.0\"\n\
                 s.add_runtime_dependency(\"minor\", [\"~> 1.2\", \">= 1.2.5\"])\n\
                 s.add_development_dependency %q<patch>.freeze, [\"~> 1.2.3\".freeze]\n\
                 s.add_dependency 'major', '~> 1'\n";
    let expected = "s.add_dependency \"pin\", \"= 1.1.0\"\n\
                    s.add_dependency \"bare\", \"1.1.0\"\n\
                    s.add_runtime_dependency(\"minor\", [\"~> 2.0\", \">= 1.2.5\"])\n\
                    s.add_development_dependency %q<patch>.freeze, [\"~> 1.3.0\".freeze]\n\
                    s.add_dependency 'major', '~> 2'\n";
    let output = rewrite_deps(
        input,
        &[
            ("pin", "1.1.0"),
            ("bare", "1.1.0"),
            ("minor", "2.0.0"),
            ("patch", "1.3.0"),
            ("major", "2.1.0"),
        ],
    );
    assert_eq!(output, expected);
}

#[test]
fn every_duplicated_declaration_is_rewritten() {
    // Generated gemspecs declare each dependency in both branches of a version check.
    let input = "if s.respond_to? :add_runtime_dependency then\n  \
                 s.add_runtime_dependency(%q<core>.freeze, [\"~> 1.0\"])\nelse\n  \
                 s.add_dependency(%q<core>.freeze, [\"~> 1.0\"])\nend\n";
    assert_eq!(
        rewrite_deps(input, &[("core", "2.0.0")]),
        input.replace("~> 1.0", "~> 2.0")
    );
}

#[test]
fn requirements_sampo_cannot_move_are_left_alone() {
    let input = "s.add_dependency \"capped\", \">= 1.0\", \"< 2\"\n\
                 s.add_dependency \"interp\", \"= #{Foo::VERSION}\"\n\
                 s.add_dependency \"var\", version\n";
    assert_eq!(
        rewrite_deps(
            input,
            &[("capped", "2.0.0"), ("interp", "2.0.0"), ("var", "2.0.0")]
        ),
        input
    );
}

#[test]
fn prerelease_requirements_keep_their_range_and_return_to_stable() {
    let input = "s.add_dependency \"core\", \"~> 1.2\", \">= 1.0\"\n";
    let rc = rewrite_deps(input, &[("core", "1.3.0-rc.1")]);
    // `~> 1.3.rc.1` still allows anything below 2, like `~> 1.2`.
    assert_eq!(
        rc,
        "s.add_dependency \"core\", \"~> 1.3.rc.1\", \">= 1.0\"\n"
    );
    assert_eq!(
        rewrite_deps(&rc, &[("core", "1.3.0")]),
        "s.add_dependency \"core\", \"~> 1.3\", \">= 1.0\"\n"
    );

    let hyphenated = "s.add_dependency \"core\", \"~> 1.2.0-rc.1\"\n";
    assert_eq!(
        rewrite_deps(hyphenated, &[("core", "1.2.0")]),
        "s.add_dependency \"core\", \"~> 1.2.0\"\n"
    );

    let capped = "s.add_dependency \"core\", \"~> 1.2\", \"<= 1.2.9\"\n";
    assert_eq!(rewrite_deps(capped, &[("core", "1.3.0-rc.1")]), capped);
}

#[test]
fn constraint_check_follows_rubygems_semantics() {
    let temp = tempfile::tempdir().unwrap();
    let gemspec = temp.path().join("app.gemspec");
    write_file(
        &gemspec,
        "Gem::Specification.new do |s|\n  s.name = \"app\"\n  s.version = \"1.0.0\"\n  \
         s.add_dependency \"core\", \"~> 1.2\", \">= 1.2.3\"\n  \
         s.add_dependency \"free\"\n  s.add_dependency \"var\", version\n  \
         s.add_dependency \"pin\", \"= 1.0.0\"\n  \
         s.add_dependency \"capped_pin\", \"1.0.0\", \"< 1.5\"\nend\n",
    );

    let check = |dep: &str, version: &str| check_dependency_constraint(&gemspec, dep, version);
    assert_eq!(
        check("core", "1.9.0").unwrap(),
        ConstraintCheckResult::Satisfied
    );
    assert_eq!(
        check("core", "2.0.0").unwrap(),
        ConstraintCheckResult::NotSatisfied {
            constraint: "~> 1.2, >= 1.2.3".to_string(),
            new_version: "2.0.0".to_string(),
        }
    );
    assert_eq!(
        check("core", "1.2.2").unwrap(),
        ConstraintCheckResult::NotSatisfied {
            constraint: "~> 1.2, >= 1.2.3".to_string(),
            new_version: "1.2.2".to_string(),
        }
    );
    assert_eq!(
        check("free", "9.0.0").unwrap(),
        ConstraintCheckResult::Satisfied
    );
    // The release rewrites a pin, so it never blocks a fixed or linked group.
    assert!(matches!(
        check("pin", "2.0.0").unwrap(),
        ConstraintCheckResult::Skipped { .. }
    ));
    assert!(matches!(
        check("capped_pin", "2.0.0").unwrap(),
        ConstraintCheckResult::NotSatisfied { .. }
    ));
    assert!(matches!(
        check("var", "2.0.0").unwrap(),
        ConstraintCheckResult::Unverifiable { .. }
    ));
    assert!(matches!(
        check("core", "2.0.0-rc.1").unwrap(),
        ConstraintCheckResult::Skipped { .. }
    ));
    assert!(matches!(
        check("absent", "2.0.0").unwrap(),
        ConstraintCheckResult::Skipped { .. }
    ));
}

#[test]
fn gem_version_ordering_matches_rubygems() {
    let cmp = |a: &str, b: &str| {
        compare_gem_versions(&gem_segments(a).unwrap(), &gem_segments(b).unwrap())
    };
    assert_eq!(cmp("1.0", "1.0.0"), Ordering::Equal);
    assert_eq!(cmp("1.0.0.rc1", "1.0.0"), Ordering::Less);
    assert_eq!(cmp("1.0.0.rc10", "1.0.0.rc2"), Ordering::Greater);
    assert_eq!(cmp("1.0.0.alpha", "1.0.0.beta"), Ordering::Less);
    assert_eq!(cmp("1.0.0-alpha.1", "1.0.0.pre.alpha.1"), Ordering::Equal);
    assert_eq!(cmp("1.10.0", "1.9.0"), Ordering::Greater);
    assert_eq!(requirement_satisfied("~> 1.2.3", "1.2.9"), Some(true));
    assert_eq!(requirement_satisfied("~> 1.2.3", "1.3.0"), Some(false));
    assert_eq!(requirement_satisfied("~> 1", "1.9.0"), Some(true));
    assert_eq!(requirement_satisfied("~> 1", "2.0.0"), Some(false));
}

#[test]
fn scaffold_push_host_placeholder_is_unpublishable() {
    let temp = tempfile::tempdir().unwrap();
    let gemspec = temp.path().join("foo.gemspec");
    let with_host = |host: &str| {
        format!(
            "Gem::Specification.new do |spec|\n  spec.name = \"foo\"\n  spec.version = \"1.0.0\"\n  \
             spec.metadata[\"allowed_push_host\"] = \"{host}\"\nend\n"
        )
    };

    write_file(
        &gemspec,
        &with_host("TODO: Set to your gem server 'https://example.com'"),
    );
    assert!(!is_publishable(&gemspec).unwrap());

    write_file(&gemspec, &with_host("https://gems.example.com"));
    assert!(is_publishable(&gemspec).unwrap());
    assert_eq!(
        private_push_host(&gemspec).as_deref(),
        Some("https://gems.example.com")
    );

    write_file(&gemspec, &with_host("https://rubygems.org"));
    assert_eq!(private_push_host(&gemspec), None);
}

#[test]
fn lockfile_dirs_cover_root_and_gem_level_locks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    scaffold_gem(&root.join("a"), "a", "A", "1.0.0", "");
    scaffold_gem(&root.join("b"), "b", "B", "1.0.0", "");
    for dir in [root.to_path_buf(), root.join("a")] {
        write_file(
            &dir.join("Gemfile"),
            "source \"https://rubygems.org\"\ngemspec\n",
        );
        write_file(&dir.join("Gemfile.lock"), "");
    }
    // A lock without its Gemfile cannot be regenerated.
    write_file(&root.join("b/Gemfile.lock"), "");

    assert_eq!(
        gemfile_lock_dirs(root),
        vec![root.to_path_buf(), root.join("a")]
    );
}

#[test]
fn hash_form_push_host_is_recognised() {
    let temp = tempfile::tempdir().unwrap();
    let gemspec = temp.path().join("foo.gemspec");
    let with_metadata = |host: &str| {
        format!(
            "Gem::Specification.new do |spec|\n  spec.name = \"foo\"\n  spec.version = \"1.0.0\"\n  \
             spec.metadata = {{ \"homepage_uri\" => \"https://x\", \"allowed_push_host\" => \"{host}\" }}\nend\n"
        )
    };

    write_file(&gemspec, &with_metadata("https://gems.example.com"));
    assert_eq!(
        private_push_host(&gemspec).as_deref(),
        Some("https://gems.example.com")
    );

    write_file(&gemspec, &with_metadata("TODO: x"));
    assert!(!is_publishable(&gemspec).unwrap());
}

#[test]
fn hyphenated_literal_keeps_its_spelling() {
    let input =
        "Gem::Specification.new do |s|\n  s.name = \"lit\"\n  s.version = \"1.0.0-alpha.1\"\nend\n";
    let (output, _) = update_manifest_versions(
        Path::new("lit.gemspec"),
        input,
        Some("1.0.0-alpha.2"),
        &BTreeMap::new(),
    )
    .unwrap();
    // Published as `1.0.0.pre.alpha.2`, above `1.0.0.pre.alpha.1`, where the dotted
    // `1.0.0.alpha.2` would sort below it.
    assert_eq!(output, input.replace("alpha.1", "alpha.2"));
}

#[test]
fn hyphenated_label_round_trips() {
    let written = semver_to_gem("1.0.0-release-candidate.1");
    assert_eq!(written, "1.0.0.release.candidate.1");
    assert_eq!(
        gem_to_semver(&written).as_deref(),
        Some("1.0.0-release.candidate.1")
    );
    assert_eq!(gem_to_semver("1.2.3.4-rc1"), None);
    assert_eq!(gem_to_semver("1.0.0-"), None);
}

#[test]
fn a_runtime_declaration_wins_over_a_development_one() {
    let temp = tempfile::tempdir().unwrap();
    scaffold_gem(&temp.path().join("core"), "core", "Core", "1.0.0", "");
    scaffold_gem(
        &temp.path().join("app"),
        "app",
        "App",
        "0.1.0",
        "  spec.add_development_dependency \"core\"\n  spec.add_runtime_dependency \"core\"\n",
    );

    let packages = discover(temp.path()).unwrap();
    let app = packages.iter().find(|p| p.name == "app").unwrap();
    assert_eq!(
        app.internal_deps,
        BTreeSet::from(["rubygems/core".to_string()])
    );
    assert!(app.internal_dev_deps.is_empty());
}

#[test]
fn a_directory_with_several_gemspecs_is_skipped() {
    let temp = tempfile::tempdir().unwrap();
    scaffold_gem(temp.path(), "one", "One", "1.0.0", "");
    scaffold_gem(temp.path(), "two", "Two", "1.0.0", "");
    scaffold_gem(&temp.path().join("kept"), "kept", "Kept", "1.0.0", "");

    let names: Vec<String> = discover(temp.path())
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, vec!["kept".to_string()]);
}

#[test]
fn undotted_prerelease_has_no_semver_equivalent() {
    for gem in ["1.0.0a", "2.0.0beta1", "1.0.0rc1", "1.0a"] {
        assert_eq!(gem_to_semver(gem), None, "{gem}");
    }
}

#[test]
fn version_constant_defined_in_the_gemspec_itself() {
    let temp = tempfile::tempdir().unwrap();
    let gemspec = temp.path().join("foo.gemspec");
    write_file(
        &gemspec,
        "VERSION = \"1.2.3\"\n\nGem::Specification.new do |s|\n  s.name = \"foo\"\n  \
         s.version = VERSION\nend\n",
    );

    let packages = discover(temp.path()).unwrap();
    assert_eq!(packages[0].version, "1.2.3");

    finalize_version_files(&packages, &versions(&[("rubygems/foo", "1.3.0")])).unwrap();
    assert!(
        fs::read_to_string(&gemspec)
            .unwrap()
            .starts_with("VERSION = \"1.3.0\"\n")
    );
}

#[test]
fn discovery_skips_test_fixture_gemspecs() {
    // Jekyll keeps `test/fixtures/test-theme/test-theme.gemspec`, spec/ trees do the same.
    let temp = tempfile::tempdir().unwrap();
    let literal = |name: &str| {
        format!(
            "Gem::Specification.new do |s|\n  s.name = \"{name}\"\n  s.version = \"0.1.0\"\nend\n"
        )
    };
    write_file(&temp.path().join("jekyll.gemspec"), &literal("jekyll"));
    write_file(
        &temp
            .path()
            .join("test/fixtures/test-theme/test-theme.gemspec"),
        &literal("test-theme"),
    );
    write_file(
        &temp
            .path()
            .join("spec/fixtures/spec-theme/spec-theme.gemspec"),
        &literal("spec-theme"),
    );

    let names: Vec<String> = discover(temp.path())
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, vec!["jekyll".to_string()]);
}
