use crate::changeset::{Bump, load_all};
use crate::cli::ReleaseArgs;
use crate::config::Config;
use crate::workspace::{CrateInfo, Workspace};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
struct CommitInfo {
    sha: String,
    short_sha: String,
    author_name: String,
    author_email: String,
    author_login: Option<String>,
}

pub fn run(args: &ReleaseArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    run_in(&cwd, args)
}

pub fn run_in(root: &std::path::Path, args: &ReleaseArgs) -> io::Result<()> {
    let ws = Workspace::discover_from(root).map_err(io::Error::other)?;
    let cfg = Config::load(&ws.root)?;

    let changesets = load_all(&cfg.changesets_dir)?;
    if changesets.is_empty() {
        println!("No changesets found in {}", cfg.changesets_dir.display());
        return Ok(());
    }

    // Compute highest bump per package and collect messages per package
    let mut bump_by_pkg: BTreeMap<String, Bump> = BTreeMap::new();
    let mut messages_by_pkg: BTreeMap<String, Vec<(String, Bump)>> = BTreeMap::new();
    let mut used_paths: BTreeSet<std::path::PathBuf> = BTreeSet::new();

    // Resolve GitHub repo slug once if available (env or origin remote)
    let repo_slug = detect_github_repo_slug(&ws.root);
    let github_token = std::env::var("GITHUB_TOKEN")
        .ok()
        .or_else(|| std::env::var("GH_TOKEN").ok());

    for cs in &changesets {
        for pkg in &cs.packages {
            used_paths.insert(cs.path.clone());
            bump_by_pkg
                .entry(pkg.clone())
                .and_modify(|b| {
                    if cs.bump > *b {
                        *b = cs.bump;
                    }
                })
                .or_insert(cs.bump);
            // Enrich message with commit info and acknowledgments
            let enriched = enrich_changeset_message(
                &ws.root,
                &cs.path,
                &cs.message,
                repo_slug.as_deref(),
                github_token.as_deref(),
            );

            messages_by_pkg
                .entry(pkg.clone())
                .or_default()
                .push((enriched, cs.bump));
        }
    }

    if bump_by_pkg.is_empty() {
        println!("No applicable packages found in changesets.");
        return Ok(());
    }

    // Map crate name -> CrateInfo for quick lookup
    let mut by_name: BTreeMap<String, &CrateInfo> = BTreeMap::new();
    for c in &ws.members {
        by_name.insert(c.name.clone(), c);
    }

    // Prepare plan
    let mut releases: Vec<(String, String, String)> = Vec::new(); // (name, old_version, new_version)
    for (name, bump) in &bump_by_pkg {
        if let Some(info) = by_name.get(name) {
            let old = if info.version.is_empty() {
                "0.0.0".to_string()
            } else {
                info.version.clone()
            };
            let newv = bump_version(&old, *bump).unwrap_or_else(|_| old.clone());
            releases.push((name.clone(), old, newv));
        }
    }

    if releases.is_empty() {
        println!("No matching workspace crates to release.");
        return Ok(());
    }

    // Print plan
    println!("Planned releases:");
    for (name, old, newv) in &releases {
        println!("  {name}: {old} -> {newv}");
    }

    if args.dry_run {
        println!("Dry-run: no files modified, no tags created.");
        return Ok(());
    }

    // Apply: update Cargo.toml and CHANGELOG
    for (name, old, newv) in &releases {
        let info = by_name.get(name.as_str()).unwrap();
        let manifest_path = info.path.join("Cargo.toml");
        let text = fs::read_to_string(&manifest_path)?;
        let updated = update_package_version_in_toml(&text, newv)?;
        fs::write(&manifest_path, updated)?;

        let messages = messages_by_pkg.get(name).cloned().unwrap_or_default();
        update_changelog(&info.path, name, old, newv, &messages)?;
    }

    // Remove consumed changesets
    for p in used_paths {
        let _ = fs::remove_file(p);
    }
    println!("Removed consumed changesets.");

    Ok(())
}

fn bump_version(old: &str, bump: Bump) -> Result<String, String> {
    let mut parts = old
        .split('.')
        .map(|s| s.parse::<u64>().unwrap_or(0))
        .collect::<Vec<_>>();
    while parts.len() < 3 {
        parts.push(0);
    }
    let (maj, min, pat) = (parts[0], parts[1], parts[2]);
    let (maj, min, pat) = match bump {
        Bump::Patch => (maj, min, pat + 1),
        Bump::Minor => (maj, min + 1, 0),
        Bump::Major => (maj + 1, 0, 0),
    };
    Ok(format!("{maj}.{min}.{pat}"))
}

fn update_package_version_in_toml(input: &str, new_version: &str) -> io::Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut in_package = false;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_package = trimmed == "[package]";
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_package {
            // Replace version assignment in package section
            if let Some(_idx) = trimmed.find("version") {
                // naive key detection, ensure not commented out
                if !trimmed.starts_with('#') && trimmed.starts_with("version") {
                    // preserve leading whitespace up to key
                    let leading = &line[..line.find('v').unwrap_or(0)];
                    out.push_str(leading);
                    out.push_str(&format!("version = \"{}\"\n", new_version));
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

fn update_changelog(
    crate_dir: &Path,
    package: &str,
    old_version: &str,
    new_version: &str,
    entries: &[(String, Bump)],
) -> io::Result<()> {
    let path = crate_dir.join("CHANGELOG.md");
    let existing = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let mut body = existing.trim_start_matches('\u{feff}').to_string();
    // Remove existing top package header if present
    let package_header = format!("# {}", package);
    if body.starts_with(&package_header) {
        if let Some(idx) = body.find('\n') {
            body = body[idx + 1..].to_string();
        } else {
            body.clear();
        }
    }

    // Parse and merge the current top section only if it's an unpublished section.
    // Heuristic: if the top section header equals the current (old) version, it is published
    // and must be preserved. Otherwise, treat it as in-progress and merge its bullets.
    let mut merged_major: Vec<String> = Vec::new();
    let mut merged_minor: Vec<String> = Vec::new();
    let mut merged_patch: Vec<String> = Vec::new();

    // helper to push without duplicates (preserve append order)
    let push_unique = |list: &mut Vec<String>, msg: &str| {
        if !list.iter().any(|m| m == msg) {
            list.push(msg.to_string());
        }
    };

    // Collect new entries
    for (msg, bump) in entries {
        match bump {
            Bump::Major => push_unique(&mut merged_major, msg),
            Bump::Minor => push_unique(&mut merged_minor, msg),
            Bump::Patch => push_unique(&mut merged_patch, msg),
        }
    }

    // If body starts with a previous top section (## ...), inspect its header.
    // If header == old_version => preserve it (do not merge or strip).
    // Else => parse and merge its bullets, then strip that section.
    let trimmed = body.trim_start();
    if trimmed.starts_with("## ") {
        // Extract first header line text
        let mut lines_iter = trimmed.lines();
        let header_line = lines_iter.next().unwrap_or("").trim();
        let header_text = header_line.trim_start_matches("## ").trim();

        let is_published_top = header_text == old_version;

        if !is_published_top {
            // Determine the extent of the first section in 'trimmed'
            let after_header_offset = header_line.len();
            let rest_after_header = &trimmed[after_header_offset..];
            // Find next section marker starting at a new line
            let next_rel = rest_after_header.find("\n## ");
            let (section_text, remaining) = match next_rel {
                Some(pos) => {
                    let end = after_header_offset + pos + 1; // include leading newline
                    (&trimmed[..end], &trimmed[end..])
                }
                None => (trimmed, ""),
            };

            let mut current = None::<&str>;
            for line in section_text.lines() {
                let t = line.trim();
                if t.eq_ignore_ascii_case("### Major changes") {
                    current = Some("major");
                    continue;
                } else if t.eq_ignore_ascii_case("### Minor changes") {
                    current = Some("minor");
                    continue;
                } else if t.eq_ignore_ascii_case("### Patch changes") {
                    current = Some("patch");
                    continue;
                }
                if t.starts_with("- ") {
                    let msg = t.trim_start_matches("- ").trim();
                    match current {
                        Some("major") => push_unique(&mut merged_major, msg),
                        Some("minor") => push_unique(&mut merged_minor, msg),
                        Some("patch") => push_unique(&mut merged_patch, msg),
                        _ => {}
                    }
                }
            }

            body = remaining.to_string();
        }
    }

    // Build new aggregated top section
    let mut section = String::new();
    section.push_str(&format!("# {}\n\n", package));
    section.push_str(&format!("## {}\n\n", new_version));

    if !merged_major.is_empty() {
        section.push_str("### Major changes\n\n");
        for msg in &merged_major {
            section.push_str("- ");
            section.push_str(msg);
            section.push('\n');
        }
        section.push('\n');
    }
    if !merged_minor.is_empty() {
        section.push_str("### Minor changes\n\n");
        for msg in &merged_minor {
            section.push_str("- ");
            section.push_str(msg);
            section.push('\n');
        }
        section.push('\n');
    }
    if !merged_patch.is_empty() {
        section.push_str("### Patch changes\n\n");
        for msg in &merged_patch {
            section.push_str("- ");
            section.push_str(msg);
            section.push('\n');
        }
        section.push('\n');
    }

    let combined = if body.trim().is_empty() {
        section
    } else {
        format!("{}{}", section, body)
    };
    fs::write(&path, combined)
}

// (tags are created during publish)

fn enrich_changeset_message(
    repo_root: &Path,
    changeset_path: &Path,
    message: &str,
    repo_slug: Option<&str>,
    github_token: Option<&str>,
) -> String {
    // Try to resolve commit info for the changeset file
    let commit = get_commit_info_for_path(repo_root, changeset_path);

    // Build commit link prefix if possible
    let mut prefix = String::new();
    if let Some(ci) = &commit {
        if let Some(slug) = repo_slug {
            prefix.push_str("[");
            prefix.push('`');
            prefix.push_str(&ci.short_sha);
            prefix.push('`');
            prefix.push(']');
            prefix.push('(');
            prefix.push_str(&format!(
                "https://github.com/{}/commit/{}",
                slug, ci.sha
            ));
            prefix.push(')');
            prefix.push(' ');
        } else {
            // Show short sha without link
            prefix.push('`');
            prefix.push_str(&ci.short_sha);
            prefix.push('`');
            prefix.push(' ');
        }
    }

    // Build acknowledgment suffix
    let mut suffix = String::new();
    let mut login: Option<String> = None;
    if let (Some(slug), Some(token), Some(ci)) = (repo_slug, github_token.as_deref(), &commit) {
        login = lookup_github_login_for_commit(slug, token, &ci.sha);
    }

    // Determine which user string to display and whether it's a GitHub login
    let (display_user, is_login) = if let Some(ref l) = login {
        (l.clone(), true)
    } else if let Some(ci) = &commit {
        (ci.author_name.clone(), false)
    } else {
        (String::new(), false)
    };

    if !display_user.is_empty() {
        suffix.push_str(" — Thanks ");
        if is_login {
            suffix.push('@');
        }
        suffix.push_str(&display_user);
        suffix.push(' ');

        // Attempt first-contribution detection if we have a username
        if let (Some(slug), Some(token), Some(user_login)) = (repo_slug, github_token.as_deref(), login.as_deref()) {
            if is_first_contribution(slug, token, user_login).unwrap_or(false) {
                suffix.push_str("for your first contribution 🎉 ");
            }
        }
        suffix.push('!');
    }

    if prefix.is_empty() && suffix.is_empty() {
        message.to_string()
    } else if suffix.is_empty() {
        format!("{}{}", prefix, message)
    } else if prefix.is_empty() {
        format!("{} {}", message, suffix)
    } else {
        format!("{}{} {}", prefix, message, suffix)
    }
}

fn detect_github_repo_slug(repo_root: &Path) -> Option<String> {
    // Prefer explicit env var set in CI
    if let Ok(repo) = std::env::var("GITHUB_REPOSITORY") {
        if !repo.is_empty() && repo.contains('/') {
            return Some(repo);
        }
    }
    // Try origin remote
    let url = run_git_capture(repo_root, &["config", "--get", "remote.origin.url"])?;
    parse_github_slug(&url.trim())
}

fn parse_github_slug(url: &str) -> Option<String> {
    // Support git@github.com:owner/repo(.git) and https(s)://github.com/owner/repo(.git)
    if let Some(rest) = url.strip_prefix("git@github.com:") {
        let slug = rest.trim_end_matches(".git");
        if slug.split('/').count() == 2 {
            return Some(slug.to_string());
        }
    }
    if let Some(pos) = url.find("github.com/") {
        let rest = &url[pos + "github.com/".len()..];
        let slug = rest.trim_end_matches('.').trim_end_matches("git");
        let slug = slug.trim_end_matches('/');
        let parts: Vec<&str> = slug.split('/').collect();
        if parts.len() >= 2 {
            return Some(format!("{}/{}", parts[0], parts[1]));
        }
    }
    None
}

fn run_git_capture(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn get_commit_info_for_path(repo_root: &Path, path: &Path) -> Option<CommitInfo> {
    // Use a relative path if possible (git prefers that)
    let rel = path.strip_prefix(repo_root).unwrap_or(path);
    let rel_str = rel.to_string_lossy();
    // Retrieve the first (adding) commit for the file
    let fmt = "%H\x1f%h\x1f%an\x1f%ae";
    let arg = format!("--pretty=format:{}", fmt);
    let output = Command::new("git")
        .current_dir(repo_root)
        .args([
            "log",
            "--diff-filter=A",
            "--follow",
            "-n",
            "1",
            &arg,
            "--",
            &rel_str,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout);
    let line = s.lines().next().unwrap_or("");
    let parts: Vec<&str> = line.split('\u{001F}').collect();
    if parts.len() < 4 {
        return None;
    }
    Some(CommitInfo {
        sha: parts[0].to_string(),
        short_sha: parts[1].to_string(),
        author_name: parts[2].to_string(),
        author_email: parts[3].to_string(),
        author_login: None,
    })
}

fn lookup_github_login_for_commit(repo_slug: &str, token: &str, sha: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{}/commits/{}", repo_slug, sha);
    let output = Command::new("curl")
        .args([
            "-sS",
            "-H",
            &format!("Authorization: Bearer {}", token),
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let body = String::from_utf8_lossy(&output.stdout);
    // Naive parse: prefer top-level author.login if present; else commit.author.name
    if let Some(pos) = body.find("\"login\":\"") {
        let start = pos + 9;
        if let Some(end) = body[start..].find('"') {
            let login = &body[start..start + end];
            if !login.is_empty() {
                return Some(login.to_string());
            }
        }
    }
    None
}

fn is_first_contribution(repo_slug: &str, token: &str, login: &str) -> Option<bool> {
    let url = format!(
        "https://api.github.com/repos/{}/commits?author={}&per_page=2",
        repo_slug, login
    );
    let output = Command::new("curl")
        .args([
            "-sS",
            "-H",
            &format!("Authorization: Bearer {}", token),
            "-H",
            "Accept: application/vnd.github+json",
            &url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let body = String::from_utf8_lossy(&output.stdout);
    // Count occurrences of "sha":
    let count = body.matches("\"sha\"").count();
    Some(count <= 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bumps_versions() {
        assert_eq!(bump_version("0.0.0", Bump::Patch).unwrap(), "0.0.1");
        assert_eq!(bump_version("0.1.2", Bump::Minor).unwrap(), "0.2.0");
        assert_eq!(bump_version("1.2.3", Bump::Major).unwrap(), "2.0.0");
    }

    #[test]
    fn updates_version_in_toml() {
        let input = "[package]\nname=\"x\"\nversion = \"0.1.0\"\n\n[dependencies]\n";
        let out = update_package_version_in_toml(input, "0.2.0").unwrap();
        assert!(out.contains("version = \"0.2.0\""));
        assert!(out.contains("[dependencies]"));
    }

    #[test]
    fn no_changesets_returns_ok_and_no_changes() {
        use crate::cli::ReleaseArgs;
        use std::fs;
        use std::path::Path;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        // one crate
        let cdir = root.join("crates/x");
        fs::create_dir_all(&cdir).unwrap();
        let manifest_path = cdir.join("Cargo.toml");
        fs::write(&manifest_path, "[package]\nname=\"x\"\nversion=\"0.1.0\"\n").unwrap();

        // No .sampo/changesets directory created -> load_all returns empty
        super::run_in(root, &ReleaseArgs { dry_run: false }).unwrap();

        // Verify no change to manifest
        let after = fs::read_to_string(&manifest_path).unwrap();
        assert!(after.contains("version=\"0.1.0\"") || after.contains("version = \"0.1.0\""));

        // No changelog created
        assert!(!Path::new(&cdir.join("CHANGELOG.md")).exists());
    }

    #[test]
    fn changelog_top_section_is_merged_and_reheaded() {
        use crate::cli::ReleaseArgs;
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        // crate x @ 0.1.0
        let cdir = root.join("crates/x");
        fs::create_dir_all(&cdir).unwrap();
        fs::write(
            cdir.join("Cargo.toml"),
            "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        // initial changelog with an unpublished section 0.1.1
        fs::write(
            cdir.join("CHANGELOG.md"),
            "# x\n\n## 0.1.1\n\n### Patch changes\n\n- fix: a bug\n\n",
        )
        .unwrap();

        // Configure .sampo changesets
        let csdir = root.join(".sampo/changesets");
        fs::create_dir_all(&csdir).unwrap();
        // add a minor changeset -> should rehead to 0.2.0 and merge entries
        fs::write(
            csdir.join("one.md"),
            "---\npackages:\n  - x\nrelease: minor\n---\n\nfeat: new thing\n",
        )
        .unwrap();

        // run release (not dry-run)
        super::run_in(root, &ReleaseArgs { dry_run: false }).unwrap();

        let log = fs::read_to_string(cdir.join("CHANGELOG.md")).unwrap();
        assert!(log.contains("# x"));
        assert!(log.contains("## 0.2.0"), "should rehead to next version");
        assert!(log.contains("### Minor changes"));
        assert!(log.contains("feat: new thing"));
        assert!(log.contains("### Patch changes"));
        assert!(log.contains("fix: a bug"));

        // ensure only one top section, and previous 0.1.1 header is gone
        assert!(!log.contains("## 0.1.1\n"));
    }

    #[test]
    fn published_top_section_is_preserved_and_new_section_is_added() {
        use crate::cli::ReleaseArgs;
        use std::fs;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // workspace
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers=[\"crates/*\"]\n",
        )
        .unwrap();

        // crate x @ 0.1.0
        let cdir = root.join("crates/x");
        fs::create_dir_all(&cdir).unwrap();
        fs::write(
            cdir.join("Cargo.toml"),
            "[package]\nname=\"x\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();

        // existing changelog with published 0.1.0 at the top
        fs::write(
            cdir.join("CHANGELOG.md"),
            "# x\n\n## 0.1.0\n\n### Patch changes\n\n- initial patch\n\n",
        )
        .unwrap();

        // Configure .sampo changesets
        let csdir = root.join(".sampo/changesets");
        fs::create_dir_all(&csdir).unwrap();
        // add a minor changeset -> should add a new 0.2.0 section above 0.1.0
        fs::write(
            csdir.join("one.md"),
            "---\npackages:\n  - x\nrelease: minor\n---\n\nfeat: new minor\n",
        )
        .unwrap();

        // run release (not dry-run)
        super::run_in(root, &ReleaseArgs { dry_run: false }).unwrap();

        let log = fs::read_to_string(cdir.join("CHANGELOG.md")).unwrap();
        // The new section should be present and come before 0.1.0
        let idx_new = log.find("## 0.2.0").unwrap();
        let idx_old = log.find("## 0.1.0").unwrap();
        assert!(idx_new < idx_old, "new section must precede published one");
        assert!(log.contains("### Minor changes"));
        assert!(log.contains("feat: new minor"));
        // old section remains intact
        assert!(log.contains("### Patch changes"));
        assert!(log.contains("initial patch"));
    }
}
