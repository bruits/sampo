use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bump {
    Patch = 0,
    Minor = 1,
    Major = 2,
}

impl Bump {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "patch" | "p" => Some(Bump::Patch),
            "minor" | "mi" => Some(Bump::Minor),
            "major" | "ma" => Some(Bump::Major),
            _ => None,
        }
    }
}

impl std::fmt::Display for Bump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Bump::Patch => "patch",
            Bump::Minor => "minor",
            Bump::Major => "major",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone)]
pub struct Changeset {
    pub path: PathBuf,
    pub packages: Vec<String>,
    pub bump: Bump,
    pub message: String,
}

pub fn load_all(dir: &Path) -> io::Result<Vec<Changeset>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        if let Some(cs) = parse_changeset(&text, &path) {
            out.push(cs);
        }
    }
    Ok(out)
}

fn parse_changeset(text: &str, path: &Path) -> Option<Changeset> {
    // Expect frontmatter delimited by --- lines, with keys: packages (list), release (string)
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut packages: Vec<String> = Vec::new();
    let mut bump: Option<Bump> = None;
    let mut in_packages = false;
    for line in &mut lines {
        let l = line.trim();
        if l == "---" {
            break;
        }
        if l.starts_with("packages:") {
            in_packages = true;
            continue;
        }
        if in_packages {
            // list items like "- name"
            if let Some(rest) = l.strip_prefix('-') {
                let name = rest.trim().to_string();
                if !name.is_empty() {
                    packages.push(name);
                }
                continue;
            } else if !l.is_empty() {
                // a non-list line ends the packages block
                in_packages = false;
            }
        }
        if let Some(v) = l.strip_prefix("release:")
            && let Some(b) = Bump::from_str(v)
        {
            bump = Some(b);
        }
    }

    // The remainder after the second --- is the message
    let remainder: String = lines.collect::<Vec<_>>().join("\n");
    let message = remainder.trim().to_string();
    if packages.is_empty() || bump.is_none() || message.is_empty() {
        return None;
    }
    Some(Changeset {
        path: path.to_path_buf(),
        packages,
        bump: bump.unwrap(),
        message,
    })
}

pub fn render_markdown(packages: &[String], bump: Bump, message: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("packages:\n");
    for p in packages {
        let _ = writeln!(out, "  - {}", p);
    }
    let _ = writeln!(out, "release: {}", bump);
    out.push_str("---\n\n");
    out.push_str(message);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_changeset() {
        let text = "---\npackages:\n  - a\n  - b\nrelease: minor\n---\n\nfeat: message\n";
        let p = Path::new("/tmp/x.md");
        let cs = parse_changeset(text, p).unwrap();
        assert_eq!(cs.packages, vec!["a", "b"]);
        assert_eq!(cs.bump, Bump::Minor);
        assert_eq!(cs.message, "feat: message");
    }

    #[test]
    fn render_changeset_markdown() {
        let s = render_markdown(&["a".into(), "b".into()], Bump::Minor, "feat: x");
        assert!(s.starts_with("---\n"));
        assert!(s.contains("packages:\n  - a\n  - b\n"));
        assert!(s.contains("release: minor\n"));
        assert!(s.ends_with("feat: x\n"));
    }
}
