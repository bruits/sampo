use crate::errors::Result;
use crate::{
    Config,
    publish::is_publishable_to_crates_io,
    types::{PackageInfo, Workspace},
};
use std::collections::BTreeSet;

/// Determines whether a package should be ignored based on configuration.
///
/// Rules:
/// - When `ignore_unpublished` is true, skip packages that are not publishable to crates.io
/// - When `ignore` contains patterns, skip packages matching by name or workspace-relative path
pub fn should_ignore_package(cfg: &Config, ws: &Workspace, info: &PackageInfo) -> Result<bool> {
    // 1) ignore_unpublished
    if cfg.ignore_unpublished {
        let manifest = info.path.join("Cargo.toml");
        if !is_publishable_to_crates_io(&manifest)? {
            return Ok(true);
        }
    }

    // 2) explicit ignore patterns
    if !cfg.ignore.is_empty() {
        let rel = info
            .path
            .strip_prefix(&ws.root)
            .unwrap_or(&info.path)
            .to_string_lossy()
            .replace('\\', "/");
        for pat in &cfg.ignore {
            if wildcard_match(pat, &info.name) || wildcard_match(pat, &rel) {
                return Ok(true);
            }
        }
    }

    Ok(false)
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

/// Returns the list of visible package names according to the configuration.
pub fn list_visible_packages(ws: &Workspace, cfg: &Config) -> Result<Vec<String>> {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for c in filter_members(ws, cfg)? {
        names.insert(c.name.clone());
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
                    version: "0.1.0".into(),
                    path: PathBuf::from("/repo/tools/internal-tool"),
                    internal_deps: Default::default(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "examples-lib".into(),
                    version: "0.1.0".into(),
                    path: PathBuf::from("/repo/examples/lib"),
                    internal_deps: Default::default(),
                    kind: PackageKind::Cargo,
                },
                PackageInfo {
                    name: "normal".into(),
                    version: "0.1.0".into(),
                    path: PathBuf::from("/repo/crates/normal"),
                    internal_deps: Default::default(),
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
        assert_eq!(names, vec!["normal".to_string()]);
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
