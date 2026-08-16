//! Bounded directory scan shared by ecosystems without a native workspace
//! declaration: there a monorepo is just several directories each carrying
//! their own manifest, so packages are found by walking the tree.
//!
//! `.gitignore` files met along the walk are honoured. Only files inside the
//! walked tree are read — never the global gitignore, `.git/info/exclude`, or
//! anything above `root` — so local and CI scans agree and no git repository
//! is required.

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::fs;
use std::path::Path;

/// Keeps the scan cheap on unrelated repos and out of deep vendored trees.
pub(crate) const MAX_SCAN_DEPTH: usize = 4;

/// Build output and fetched dependencies skipped for every ecosystem; hidden
/// directories are skipped by the walk itself.
const EXCLUDED_DIR_NAMES: &[&str] = &["build", "_build", "deps", "node_modules", "target"];

/// The `.gitignore` rules gathered between the scan root and the visited
/// directory, deeper files taking precedence as in git.
///
/// Matching is by pattern only: a manifest force-added to git (`git add -f`)
/// inside an ignored directory is still skipped.
#[derive(Default)]
pub(crate) struct GitignoreChain {
    matchers: Vec<Gitignore>,
}

impl GitignoreChain {
    /// Whether the scan should skip the file at `path`, which must live below
    /// `base`, the visited directory. As in git, each level between `base`
    /// and the file resolves separately: an ignored ancestor hides the file;
    /// a whitelisted ancestor settles only itself, never the file.
    /// `.gitignore` files deeper than `base` are not consulted.
    pub(crate) fn is_ignored(&self, base: &Path, path: &Path) -> bool {
        let Ok(rel) = path.strip_prefix(base) else {
            debug_assert!(false, "{path:?} must live below the visited {base:?}");
            return false;
        };
        let mut level_path = base.to_path_buf();
        let mut components = rel.components().peekable();
        while let Some(component) = components.next() {
            level_path.push(component);
            let is_dir = components.peek().is_some();
            if self.level(&level_path, is_dir) == Some(true) {
                return true;
            }
        }
        false
    }

    fn level(&self, path: &Path, is_dir: bool) -> Option<bool> {
        for matcher in self.matchers.iter().rev() {
            match matcher.matched(path, is_dir) {
                Match::Ignore(_) => return Some(true),
                Match::Whitelist(_) => return Some(false),
                Match::None => {}
            }
        }
        None
    }

    fn is_ignored_dir(&self, path: &Path) -> bool {
        self.level(path, true) == Some(true)
    }

    /// Load `dir/.gitignore` if present; reports whether a matcher was pushed.
    fn push(&mut self, dir: &Path) -> bool {
        let file = dir.join(".gitignore");
        if !file.is_file() {
            return false;
        }
        let mut builder = GitignoreBuilder::new(dir);
        // Unparseable lines are dropped by the builder; valid ones still apply.
        builder.add(&file);
        match builder.build() {
            Ok(matcher) if !matcher.is_empty() => {
                self.matchers.push(matcher);
                true
            }
            _ => false,
        }
    }

    fn pop(&mut self) {
        self.matchers.pop();
    }
}

/// Visit `root` and its subdirectories up to [`MAX_SCAN_DEPTH`] levels deep,
/// calling `visit` once per directory with the `.gitignore` rules in force
/// there. Hidden directories, build/dependency output, `extra_excluded`
/// names, and gitignored directories are skipped; symlinked directories are
/// never followed. Callers must check candidate manifests against the chain:
/// a directory's own `.gitignore` can exclude its files (how virtual
/// environments self-exclude) while the directory itself is still visited.
pub(crate) fn walk_package_dirs<F: FnMut(&Path, &GitignoreChain)>(
    root: &Path,
    extra_excluded: &[&str],
    mut visit: F,
) {
    let mut chain = GitignoreChain::default();
    walk(root, 0, extra_excluded, &mut chain, &mut visit);
}

fn walk<F: FnMut(&Path, &GitignoreChain)>(
    dir: &Path,
    depth: usize,
    extra_excluded: &[&str],
    chain: &mut GitignoreChain,
    visit: &mut F,
) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let pushed = chain.push(dir);
    visit(dir, chain);
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if is_excluded_dir(&path, extra_excluded) || chain.is_ignored_dir(&path) {
                continue;
            }
            walk(&path, depth + 1, extra_excluded, chain, visit);
        }
    }
    if pushed {
        chain.pop();
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
        walk_package_dirs(root, extra_excluded, |dir, _| {
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

    #[test]
    fn gitignore_excludes_matching_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("vendored/inner")).unwrap();
        fs::create_dir_all(root.join("kept")).unwrap();
        fs::write(root.join(".gitignore"), "vendored/\n").unwrap();

        let visited = visited_dirs(root, &[]);

        assert!(visited.contains(&root.join("kept")));
        assert!(!visited.contains(&root.join("vendored")));
        assert!(!visited.contains(&root.join("vendored/inner")));
    }

    #[test]
    fn nested_gitignore_applies_to_its_subtree_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("gen")).unwrap();
        fs::create_dir_all(root.join("sub/gen")).unwrap();
        fs::write(root.join("sub/.gitignore"), "gen/\n").unwrap();

        let visited = visited_dirs(root, &[]);

        assert!(visited.contains(&root.join("gen")));
        assert!(!visited.contains(&root.join("sub/gen")));
    }

    #[test]
    fn deeper_gitignore_negation_overrides_shallower_rule() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("keep")).unwrap();
        fs::create_dir_all(root.join("sub/keep")).unwrap();
        fs::write(root.join(".gitignore"), "keep/\n").unwrap();
        fs::write(root.join("sub/.gitignore"), "!keep/\n").unwrap();

        let visited = visited_dirs(root, &[]);

        assert!(!visited.contains(&root.join("keep")));
        assert!(visited.contains(&root.join("sub/keep")));
    }

    #[test]
    fn chain_excludes_files_ignored_by_their_own_dir() {
        // The virtual-environment shape: a catch-all `.gitignore` in the
        // directory itself.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("myenv")).unwrap();
        fs::create_dir_all(root.join("pkg")).unwrap();
        fs::write(root.join("myenv/.gitignore"), "*\n").unwrap();
        fs::write(root.join("myenv/pyproject.toml"), "").unwrap();
        fs::write(root.join("pkg/pyproject.toml"), "").unwrap();

        let mut with_manifest = BTreeSet::new();
        walk_package_dirs(root, &[], |dir, chain| {
            let manifest = dir.join("pyproject.toml");
            if manifest.is_file() && !chain.is_ignored(dir, &manifest) {
                with_manifest.insert(dir.to_path_buf());
            }
        });

        assert!(with_manifest.contains(&root.join("pkg")));
        assert!(visited_dirs(root, &[]).contains(&root.join("myenv")));
        assert!(!with_manifest.contains(&root.join("myenv")));
    }

    #[test]
    fn ignored_intermediate_level_hides_deeper_files() {
        // The rebar3 shape: the candidate sits a level below the visited
        // directory, beyond the walk's pruning.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("app/src")).unwrap();
        fs::write(root.join("app/.gitignore"), "src/\n").unwrap();
        fs::write(root.join("app/src/thing.app.src"), "").unwrap();

        let mut ignored = None;
        walk_package_dirs(root, &[], |dir, chain| {
            if dir.ends_with("app") {
                ignored = Some(chain.is_ignored(dir, &dir.join("src/thing.app.src")));
            }
        });

        assert_eq!(ignored, Some(true));
    }

    #[test]
    fn whitelisted_ancestor_dir_does_not_unignore_file() {
        // Matches git: `!config/` re-includes only the directory, never its
        // files — `git check-ignore -v` still reports `.gitignore:1:*.toml`.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("sub/config")).unwrap();
        fs::write(root.join(".gitignore"), "*.toml\n").unwrap();
        fs::write(root.join("sub/.gitignore"), "!config/\n").unwrap();
        fs::write(root.join("sub/config/pyproject.toml"), "").unwrap();

        let mut with_manifest = BTreeSet::new();
        walk_package_dirs(root, &[], |dir, chain| {
            let manifest = dir.join("pyproject.toml");
            if manifest.is_file() && !chain.is_ignored(dir, &manifest) {
                with_manifest.insert(dir.to_path_buf());
            }
        });

        assert!(visited_dirs(root, &[]).contains(&root.join("sub/config")));
        assert!(!with_manifest.contains(&root.join("sub/config")));
    }
}
