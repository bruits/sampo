use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateInfo {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub internal_deps: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub root: PathBuf,
    pub members: Vec<CrateInfo>,
}

#[derive(Debug)]
pub enum WorkspaceError {
    Io(io::Error),
    NotFound,
    InvalidToml(String),
    InvalidWorkspace(String),
}

impl Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceError::Io(e) => write!(f, "IO error: {}", e),
            WorkspaceError::NotFound => write!(f, "No Cargo.toml with [workspace] found"),
            WorkspaceError::InvalidToml(msg) => write!(f, "Invalid Cargo.toml: {}", msg),
            WorkspaceError::InvalidWorkspace(msg) => write!(f, "Invalid workspace: {}", msg),
        }
    }
}

impl std::error::Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WorkspaceError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for WorkspaceError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

type Result<T> = std::result::Result<T, WorkspaceError>;

impl Workspace {
    pub fn discover_from(start_dir: &Path) -> Result<Self> {
        let (root, root_toml) = find_workspace_root(start_dir)?;
        let members = parse_members(&root, &root_toml)?;
        let mut crates = Vec::new();

        // First pass: parse per-crate metadata (name, version)
        let mut name_to_path: BTreeMap<String, PathBuf> = BTreeMap::new();
        for member_dir in &members {
            let manifest_path = member_dir.join("Cargo.toml");
            let text = fs::read_to_string(&manifest_path)?;
            let value: toml::Value = text.parse().map_err(|e| {
                WorkspaceError::InvalidToml(format!("{}: {}", manifest_path.display(), e))
            })?;
            let pkg = value
                .get("package")
                .and_then(|v| v.as_table())
                .ok_or_else(|| {
                    WorkspaceError::InvalidToml(format!(
                        "missing [package] in {}",
                        manifest_path.display()
                    ))
                })?;
            let name = pkg
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    WorkspaceError::InvalidToml(format!(
                        "missing package.name in {}",
                        manifest_path.display()
                    ))
                })?
                .to_string();
            let version = pkg
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            name_to_path.insert(name.clone(), member_dir.clone());
            crates.push((name, version, member_dir.clone(), value));
        }

        // Second pass: compute internal dependencies
        let mut out: Vec<CrateInfo> = Vec::new();
        for (name, version, path, manifest) in crates {
            let internal_deps = collect_internal_deps(&path, &name_to_path, &manifest);
            out.push(CrateInfo {
                name,
                version,
                path,
                internal_deps,
            });
        }

        Ok(Workspace { root, members: out })
    }
}

fn find_workspace_root(start_dir: &Path) -> Result<(PathBuf, toml::Value)> {
    let mut dir = start_dir;
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.exists() {
            let text = fs::read_to_string(&manifest)?;
            let value: toml::Value = text.parse().map_err(|e| {
                WorkspaceError::InvalidToml(format!("{}: {}", manifest.display(), e))
            })?;
            if value.get("workspace").is_some() {
                return Ok((dir.to_path_buf(), value));
            }
        }

        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    Err(WorkspaceError::NotFound)
}

fn parse_members(root: &Path, root_toml: &toml::Value) -> Result<Vec<PathBuf>> {
    let ws = root_toml
        .get("workspace")
        .and_then(|v| v.as_table())
        .ok_or_else(|| WorkspaceError::InvalidWorkspace("missing [workspace] table".into()))?;
    let members = ws
        .get("members")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            WorkspaceError::InvalidWorkspace("missing workspace.members array".into())
        })?;

    let mut out = Vec::new();
    for m in members {
        let Some(pat) = m.as_str() else { continue };
        expand_member_pattern(root, pat, &mut out)?;
    }
    // De-duplicate and keep stable order
    let mut seen = BTreeSet::new();
    out.retain(|p| seen.insert(clean_path(p)));
    Ok(out)
}

fn expand_member_pattern(root: &Path, pat: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    // Support plain paths and the common "dir/*" pattern used by Cargo workspaces.
    if !(pat.contains('*') || pat.contains('?') || pat.contains('[')) {
        let p = clean_path(&root.join(pat));
        if p.join("Cargo.toml").exists() {
            out.push(p);
        }
        return Ok(());
    }

    // Only implement the simple and most common trailing-segment wildcard: "base/*".
    if let Some(prefix) = pat.strip_suffix("/*") {
        let base = clean_path(&root.join(prefix));
        if base.is_dir() {
            for entry in fs::read_dir(&base)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() && path.join("Cargo.toml").exists() {
                    out.push(clean_path(&path));
                }
            }
        }
        return Ok(());
    }

    Err(WorkspaceError::InvalidWorkspace(format!(
        "unsupported workspace member pattern: {}",
        pat
    )))
}

fn collect_internal_deps(
    crate_dir: &Path,
    name_to_path: &BTreeMap<String, PathBuf>,
    manifest: &toml::Value,
) -> BTreeSet<String> {
    let mut internal = BTreeSet::new();
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(tbl) = manifest.get(key).and_then(|v| v.as_table()) {
            for (dep_name, dep_val) in tbl.iter() {
                if is_internal_dep(crate_dir, name_to_path, dep_val) {
                    internal.insert(dep_name.clone());
                }
            }
        }
    }
    internal
}

fn is_internal_dep(
    crate_dir: &Path,
    name_to_path: &BTreeMap<String, PathBuf>,
    dep_val: &toml::Value,
) -> bool {
    match dep_val {
        toml::Value::String(_) => false,
        toml::Value::Table(t) => {
            if t.get("workspace")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                return true;
            }
            if let Some(path_str) = t.get("path").and_then(|v| v.as_str()) {
                // Path is relative to the dependent crate directory.
                let p = clean_path(&crate_dir.join(path_str));
                return p.join("Cargo.toml").exists();
            }
            if let Some(pkg_name) = t.get("package").and_then(|v| v.as_str()) {
                return name_to_path.contains_key(pkg_name);
            }
            false
        }
        _ => false,
    }
}

// Pure path normalization (logical), without touching the filesystem.
// - Collapses '.' and '..' where possible
// - Removes redundant separators
// - Preserves absolute vs. relative nature
fn clean_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // pop only normal components; keep root prefixes
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn clean_path_collapses_segments() {
        let base = Path::new("/tmp/one/two");
        let got = clean_path(&base.join("./three/../four"));
        assert_eq!(got, Path::new("/tmp/one/two/four"));
    }

    #[test]
    fn expand_members_supports_plain_and_glob() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // Create workspace Cargo.toml
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        // Create crates/a and crates/b with manifests
        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("a")).unwrap();
        fs::create_dir_all(crates_dir.join("b")).unwrap();
        fs::write(
            crates_dir.join("a/Cargo.toml"),
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            crates_dir.join("b/Cargo.toml"),
            "[package]\nname = \"b\"\nversion = \"0.2.0\"\n",
        )
        .unwrap();

        let (_root, root_toml) = super::find_workspace_root(root).unwrap();
        let members = super::parse_members(root, &root_toml).unwrap();
        let mut names: Vec<_> = members
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn internal_deps_detect_path_and_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        // workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        // crates: x depends on y via path, and on z via workspace
        let crates_dir = root.join("crates");
        fs::create_dir_all(crates_dir.join("x")).unwrap();
        fs::create_dir_all(crates_dir.join("y")).unwrap();
        fs::create_dir_all(crates_dir.join("z")).unwrap();
        fs::write(
            crates_dir.join("x/Cargo.toml"),
            format!(
                "{}{}{}",
                "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
                "[dependencies]\n",
                "y={ path=\"../y\" }\n z={ workspace=true }\n"
            ),
        )
        .unwrap();
        fs::write(
            crates_dir.join("y/Cargo.toml"),
            "[package]\nname=\"y\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            crates_dir.join("z/Cargo.toml"),
            "[package]\nname=\"z\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        let ws = Workspace::discover_from(root).unwrap();
        let x = ws.members.iter().find(|c| c.name == "x").unwrap();
        assert!(x.internal_deps.contains("y"));
        assert!(x.internal_deps.contains("z"));
    }
}
