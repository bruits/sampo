//! Bounded directory scan shared by ecosystems without a native workspace
//! declaration: there a monorepo is just several directories each carrying
//! their own manifest, so packages are found by walking the tree.

use std::fs;
use std::path::Path;

/// Bounds the scan so it stays cheap on unrelated repos and never wanders
/// into deep vendored trees.
pub(crate) const MAX_SCAN_DEPTH: usize = 4;

/// Build output and fetched dependencies skipped for every ecosystem; hidden
/// directories are skipped by the walk itself.
const EXCLUDED_DIR_NAMES: &[&str] = &["build", "_build", "deps", "node_modules", "target"];

/// Visit `root` and its subdirectories up to [`MAX_SCAN_DEPTH`] levels deep,
/// calling `visit` once per directory. Hidden directories, common
/// build/dependency output, and `extra_excluded` names are skipped, and
/// symlinked directories are never followed, so vendored manifests are not
/// mistaken for workspace members.
pub(crate) fn walk_package_dirs<F: FnMut(&Path)>(
    root: &Path,
    extra_excluded: &[&str],
    mut visit: F,
) {
    walk(root, 0, extra_excluded, &mut visit);
}

fn walk<F: FnMut(&Path)>(dir: &Path, depth: usize, extra_excluded: &[&str], visit: &mut F) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    visit(dir);
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        if is_excluded_dir(&path, extra_excluded) {
            continue;
        }
        walk(&path, depth + 1, extra_excluded, visit);
    }
}

fn is_excluded_dir(path: &Path, extra_excluded: &[&str]) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => {
            name.starts_with('.')
                || EXCLUDED_DIR_NAMES.contains(&name)
                || extra_excluded.contains(&name)
        }
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    fn visited_dirs(root: &Path, extra_excluded: &[&str]) -> BTreeSet<PathBuf> {
        let mut out = BTreeSet::new();
        walk_package_dirs(root, extra_excluded, |dir| {
            out.insert(dir.to_path_buf());
        });
        out
    }

    #[test]
    fn visits_root_and_nested_dirs_up_to_max_depth() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let at_limit = root.join("a/b/c/d");
        let beyond_limit = at_limit.join("e");
        fs::create_dir_all(&beyond_limit).unwrap();

        let visited = visited_dirs(root, &[]);

        assert!(visited.contains(root));
        assert!(visited.contains(&at_limit));
        assert!(!visited.contains(&beyond_limit));
    }

    #[test]
    fn skips_hidden_shared_and_extra_excluded_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for dir in [".git", "node_modules", "target", "venv", "kept"] {
            fs::create_dir_all(root.join(dir).join("inner")).unwrap();
        }

        let visited = visited_dirs(root, &["venv"]);

        assert!(visited.contains(&root.join("kept")));
        assert!(visited.contains(&root.join("kept/inner")));
        for dir in [".git", "node_modules", "target", "venv"] {
            assert!(
                !visited.contains(&root.join(dir)),
                "{dir} should be skipped"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinked_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("real/inner")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("linked")).unwrap();

        let visited = visited_dirs(root, &[]);

        assert!(visited.contains(&root.join("real")));
        assert!(!visited.contains(&root.join("linked")));
    }
}
