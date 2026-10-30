use crate::errors::Result;
use crate::{
    Config,
    adapters::PackageAdapter,
    config::TagUnpublished,
    types::{PackageInfo, Workspace},
};
use std::collections::BTreeSet;

/// Determines whether a package should be ignored based on configuration.
///
/// Rules:
/// - When `ignore_unpublished` is true, skip packages that are not publishable to any registry
/// - When `ignore` contains patterns, skip packages matching by name or workspace-relative path
pub fn should_ignore_package(cfg: &Config, ws: &Workspace, info: &PackageInfo) -> Result<bool> {
    // 1) ignore_unpublished
    if cfg.ignore_unpublished {
        let adapter = match info.kind {
            crate::types::PackageKind::Cargo => PackageAdapter::Cargo,
            crate::types::PackageKind::Npm => PackageAdapter::Npm,
            crate::types::PackageKind::Hex => PackageAdapter::Hex,
            crate::types::PackageKind::PyPI => PackageAdapter::PyPI,
            crate::types::PackageKind::Packagist => PackageAdapter::Packagist,
            crate::types::PackageKind::Maven => PackageAdapter::Maven,
        };
        let manifest = adapter.manifest_path(&info.path);
        if !adapter.is_publishable(&manifest)? {
            return Ok(true);
        }
    }

    // 2) explicit ignore patterns
    Ok(matches_any_pattern(&cfg.ignore, ws, info))
}

/// Whether `sampo publish` tags a package that is not publishable, per `git.tag_unpublished`.
pub fn should_tag_unpublished(cfg: &Config, ws: &Workspace, info: &PackageInfo) -> bool {
    match &cfg.git_tag_unpublished {
        TagUnpublished::All => true,
        TagUnpublished::Never => false,
        TagUnpublished::Matching(patterns) => matches_any_pattern(patterns, ws, info),
    }
}

/// Matches patterns against the canonical identifier, plain name, or workspace-relative path.
fn matches_any_pattern(patterns: &[String], ws: &Workspace, info: &PackageInfo) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let rel = info
        .path
        .strip_prefix(&ws.root)
        .unwrap_or(&info.path)
        .to_string_lossy()
        .replace('\\', "/");
    patterns.iter().any(|pat| {
        wildcard_match(pat, info.canonical_identifier())
            || wildcard_match(pat, &info.name)
            || wildcard_match(pat, &rel)
    })
}

/// Filters workspace members according to the configuration.
pub fn filter_members<'a>(ws: &'a Workspace, cfg: &Config) -> Result<Vec<&'a PackageInfo>> {
    let mut out = Vec::new();
    for c in &ws.members {
        if !should_ignore_package(cfg, ws, c)? {
            out.push(c);
        }
    }
    Ok(out)
}

/// Returns the list of visible package identifiers according to the configuration.
pub fn list_visible_packages(ws: &Workspace, cfg: &Config) -> Result<Vec<String>> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for c in filter_members(ws, cfg)? {
        names.insert(c.canonical_identifier().to_string());
    }
    Ok(names.into_iter().collect())
}

/// Simple wildcard match supporting '*' as any sequence (case-sensitive, anchored)
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    if pattern == text {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == text;
    }
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.is_empty() {
        return true;
    }
    let mut idx = 0usize;
    if !parts[0].is_empty() {
        if let Some(pos) = text.find(parts[0]) {
            if pos != 0 {
                return false;
            }
            idx = parts[0].len();
        } else {
            return false;
        }
    }
    for mid in parts.iter().skip(1).take(parts.len().saturating_sub(2)) {
        if mid.is_empty() {
            continue;
        }
        if let Some(pos) = text[idx..].find(mid) {
            idx += pos + mid.len();
        } else {
            return false;
        }
    }
    if let Some(last) = parts.last()
        && !last.is_empty()
    {
        if let Some(pos) = text[idx..].rfind(last) {
            return idx + pos + last.len() == text.len();
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_ws() -> Workspace {
        use crate::types::PackageKind;
        Workspace {
            root: PathBuf::from("/repo"),
            members: vec![
                PackageInfo {
                    name: "internal-tool".into(),
                    identifier: "cargo/internal-tool".into(),
                    version: "0.1.0".into(),
                    path: PathBuf::from("/repo/tools/internal-tool"),
                    internal_deps: Default::default(),
                    internal_dev_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "examples-lib".into(),
                    identifier: "cargo/examples-lib".into(),
                    version: "0.1.0".into(),
                    path: PathBuf::from("/repo/examples/lib"),
                    internal_deps: Default::default(),
                    internal_dev_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "normal".into(),
                    identifier: "cargo/normal".into(),
                    version: "0.1.0".into(),
                    path: PathBuf::from("/repo/crates/normal"),
                    internal_deps: Default::default(),
                    internal_dev_deps: BTreeSet::new(),
                    kind: PackageKind::Cargo,
                },
            ],
        }
    }

    #[test]
    fn wildcard_basic() {
        assert!(wildcard_match("a*", "abc"));
        assert!(wildcard_match("*c", "abc"));
        assert!(wildcard_match("a*c", "abc"));
        assert!(!wildcard_match("ab", "abc"));
    }

    #[test]
    fn filters_by_name_and_path() {
        let ws = dummy_ws();
        let cfg = Config {
            ignore: vec!["internal-*".into(), "examples/*".into()],
            ..Default::default()
        };

        let names = list_visible_packages(&ws, &cfg).unwrap();
        assert_eq!(names, vec!["cargo/normal".to_string()]);
    }

    #[test]
    fn should_ignore_package_by_name_pattern() {
        let ws = dummy_ws();
        let cfg = Config {
            ignore: vec!["internal-*".into()],
            ..Default::default()
        };

        let internal_crate = &ws.members[0]; // "internal-tool"
        let normal_crate = &ws.members[2]; // "normal"

        assert!(should_ignore_package(&cfg, &ws, internal_crate).unwrap());
        assert!(!should_ignore_package(&cfg, &ws, normal_crate).unwrap());
    }

    #[test]
    fn should_ignore_package_by_path_pattern() {
        let ws = dummy_ws();
        let cfg = Config {
            ignore: vec!["examples/*".into()],
            ..Default::default()
        };

        let examples_crate = &ws.members[1]; // "examples-lib" at "/repo/examples/lib"
        let normal_crate = &ws.members[2]; // "normal"

        assert!(should_ignore_package(&cfg, &ws, examples_crate).unwrap());
        assert!(!should_ignore_package(&cfg, &ws, normal_crate).unwrap());
    }

    #[test]
    fn filter_members_returns_non_ignored_crates() {
        let ws = dummy_ws();
        let cfg = Config {
            ignore: vec!["internal-*".into(), "examples/*".into()],
            ..Default::default()
        };

        let filtered = filter_members(&ws, &cfg).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "normal");
    }

    #[test]
    fn should_tag_unpublished_follows_config() {
        let ws = dummy_ws();
        let [internal, examples, normal] = [&ws.members[0], &ws.members[1], &ws.members[2]];

        let all = Config::default();
        assert!(should_tag_unpublished(&all, &ws, internal));

        let never = Config {
            git_tag_unpublished: TagUnpublished::Never,
            ..Default::default()
        };
        assert!(!should_tag_unpublished(&never, &ws, internal));

        let matching = Config {
            git_tag_unpublished: TagUnpublished::Matching(vec![
                "cargo/internal-*".into(),
                "examples/*".into(),
            ]),
            ..Default::default()
        };
        assert!(should_tag_unpublished(&matching, &ws, internal));
        assert!(should_tag_unpublished(&matching, &ws, examples));
        assert!(!should_tag_unpublished(&matching, &ws, normal));
    }

    #[test]
    fn ignore_unpublished_skips_crate_inheriting_publish_false() {
        use crate::types::PackageKind;
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\npublish = false\n",
        )
        .unwrap();
        let crate_dir = root.join("crates/app");
        fs::create_dir_all(&crate_dir).unwrap();
        fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\npublish.workspace = true\n",
        )
        .unwrap();

        let info = PackageInfo {
            name: "app".into(),
            identifier: "cargo/app".into(),
            version: "1.0.0".into(),
            path: crate_dir,
            internal_deps: BTreeSet::new(),
            internal_dev_deps: BTreeSet::new(),
            kind: PackageKind::Cargo,
        };
        let ws = Workspace {
            root: root.to_path_buf(),
            members: vec![info.clone()],
        };
        let cfg = Config {
            ignore_unpublished: true,
            ..Default::default()
        };

        assert!(should_ignore_package(&cfg, &ws, &info).unwrap());
    }

    #[test]
    fn wildcard_match_edge_cases() {
        // Empty pattern matches empty text
        assert!(wildcard_match("", ""));

        // Single wildcard matches anything
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("*", "anything"));

        // Multiple wildcards
        assert!(wildcard_match("a*b*c", "aXbYc"));
        assert!(wildcard_match("a*b*c", "abc"));
        assert!(!wildcard_match("a*b*c", "ab"));

        // No wildcards - exact match
        assert!(wildcard_match("exact", "exact"));
        assert!(!wildcard_match("exact", "different"));

        // Edge case: pattern ends with wildcard
        assert!(wildcard_match("prefix*", "prefix"));
        assert!(wildcard_match("prefix*", "prefixSUFFIX"));
    }
}
