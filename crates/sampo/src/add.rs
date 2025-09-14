use crate::cli::AddArgs;
use crate::names;
use sampo_core::{
    Bump, Config, discover_workspace, errors::Result, filters::list_visible_packages,
    render_changeset_markdown,
};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn run(args: &AddArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;

    // Discover workspace (optional but helps list packages)
    let (root, packages) = match discover_workspace(&cwd) {
        Ok(ws) => {
            // Load config to respect ignore rules when listing packages
            let cfg = Config::load(&ws.root).unwrap_or_default();
            let names = match list_visible_packages(&ws, &cfg) {
                Ok(v) => v,
                Err(_) => ws.members.into_iter().map(|c| c.name).collect::<Vec<_>>(),
            };
            (ws.root, names)
        }
        Err(_) => (cwd.clone(), Vec::new()),
    };

    // Create changesets directory if it doesn't exist
    let changesets_dir = root.join(".sampo").join("changesets");
    ensure_dir(&changesets_dir)?;

    // Collect inputs, prefilling from CLI args if provided
    let selected_packages = if args.package.is_empty() {
        prompt_packages(&packages)?
    } else {
        args.package.clone()
    };

    let bump = prompt_bump()?;

    let message = match &args.message {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => prompt_message()?,
    };

    // Compose file contents
    let contents = render_changeset_markdown(&selected_packages, bump, &message);
    let path = unique_changeset_path(&changesets_dir);
    fs::write(&path, contents)?;

    println!("Created: {}", path.display());
    Ok(())
}

fn ensure_dir(dir: &PathBuf) -> Result<()> {
    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

fn prompt_packages(available: &[String]) -> Result<Vec<String>> {
    let mut stdout = io::stdout();

    if available.is_empty() {
        loop {
            write!(
                stdout,
                "No packages detected. Enter package names (comma-separated): "
            )?;
            stdout.flush()?;
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            let items: Vec<String> = line
                .split([',', ' ', '\t', '\n', '\r'])
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .collect();
            if !items.is_empty() {
                // de-duplicate
                let mut seen = BTreeSet::new();
                let mut out = Vec::new();
                for it in items {
                    if seen.insert(it.clone()) {
                        out.push(it);
                    }
                }
                return Ok(out);
            }
        }
    } else {
        writeln!(stdout, "Detected workspace packages:")?;
        for (i, name) in available.iter().enumerate() {
            writeln!(stdout, "  {}. {}", i + 1, name)?;
        }
        loop {
            write!(
                stdout,
                "Which packages are affected by the changeset? (numbers/names, comma-separated, or '*' for all): "
            )?;
            stdout.flush()?;
            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == "*" || line.eq_ignore_ascii_case("all") {
                return Ok(available.to_vec());
            }
            let mut out: Vec<String> = Vec::new();
            'outer: for raw in line.split([',', ' ', '\t']).filter(|s| !s.is_empty()) {
                if let Ok(idx) = raw.parse::<usize>()
                    && idx >= 1
                    && idx <= available.len()
                {
                    out.push(available[idx - 1].clone());
                    continue 'outer;
                }
                // match by name
                if let Some(name) = available.iter().find(|n| n.as_str() == raw) {
                    out.push(name.clone());
                    continue 'outer;
                }
                writeln!(stdout, "Unknown: '{raw}' - try again.")?;
                out.clear();
            }
            if !out.is_empty() {
                // de-duplicate preserving order
                let mut seen = BTreeSet::new();
                out.retain(|p| seen.insert(p.clone()));
                return Ok(out);
            }
        }
    }
}

fn prompt_bump() -> Result<Bump> {
    let mut stdout = io::stdout();
    loop {
        write!(stdout, "Release type (patch/minor/major) [patch]: ")?;
        stdout.flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let l = line.trim();
        if l.is_empty() {
            return Ok(Bump::Patch);
        }
        if let Some(b) = Bump::parse(l) {
            return Ok(b);
        }
    }
}

fn prompt_message() -> Result<String> {
    let mut stdout = io::stdout();
    loop {
        write!(stdout, "Changeset message: ")?;
        stdout.flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let msg = line.trim();
        if !msg.is_empty() {
            return Ok(msg.to_string());
        }
    }
}

fn unique_changeset_path(dir: &Path) -> PathBuf {
    let mut rng = rand::thread_rng();
    let base = names::generate_file_name(&mut rng);
    let mut candidate = dir.join(format!("{base}.md"));
    // If somehow exists, add counter suffix
    let mut i = 1u32;
    while candidate.exists() {
        let name_with_counter = format!("{base}-{i}");
        candidate = dir.join(format!("{name_with_counter}.md"));
        i += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn render_has_frontmatter() {
        let md =
            render_changeset_markdown(&["a".into(), "b".into()], Bump::Minor, "feat: add stuff");
        assert!(md.starts_with("---\n"));
        assert!(md.contains("a: minor\n"));
        assert!(md.contains("b: minor\n"));
        assert!(md.ends_with("feat: add stuff\n"));
    }

    #[test]
    fn render_single_package() {
        let md = render_changeset_markdown(&["single".into()], Bump::Patch, "fix: bug");
        assert!(md.contains("single: patch\n"));
        assert!(md.ends_with("fix: bug\n"));
    }

    #[test]
    fn render_major_release() {
        let md = render_changeset_markdown(&["pkg".into()], Bump::Major, "breaking: api change");
        assert!(md.contains("pkg: major\n"));
        assert!(md.ends_with("breaking: api change\n"));
    }

    #[test]
    fn unique_changeset_path_creates_md_files() {
        let temp_dir = std::env::temp_dir().join("sampo-test");
        fs::create_dir_all(&temp_dir).unwrap();

        let path = unique_changeset_path(&temp_dir);
        assert!(path.starts_with(&temp_dir));
        assert!(path.extension().unwrap() == "md");

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn unique_changeset_path_avoids_conflicts() {
        let temp_dir = std::env::temp_dir().join("sampo-test-conflict");
        fs::create_dir_all(&temp_dir).unwrap();

        let path1 = unique_changeset_path(&temp_dir);
        let path2 = unique_changeset_path(&temp_dir);

        // Should generate different paths
        assert_ne!(path1, path2);
        assert!(path1.extension().unwrap() == "md");
        assert!(path2.extension().unwrap() == "md");

        // Clean up
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
