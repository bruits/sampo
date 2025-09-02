use crate::cli::PublishArgs;
use crate::workspace::{CrateInfo, Workspace};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

pub fn run(args: &PublishArgs) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    let ws = Workspace::discover_from(&cwd).map_err(io::Error::other)?;

    // Determine which crates are publishable to crates.io
    let mut name_to_crate: BTreeMap<String, &CrateInfo> = BTreeMap::new();
    let mut publishable: BTreeSet<String> = BTreeSet::new();
    for c in &ws.members {
        let manifest = c.path.join("Cargo.toml");
        if is_publishable_to_crates_io(&manifest)? {
            publishable.insert(c.name.clone());
            name_to_crate.insert(c.name.clone(), c);
        }
    }

    if publishable.is_empty() {
        println!("No publishable crates for crates.io were found in the workspace.");
        return Ok(());
    }

    // Validate internal deps do not include non-publishable crates
    let mut errors: Vec<String> = Vec::new();
    for name in &publishable {
        let c = name_to_crate.get(name).unwrap();
        for dep in &c.internal_deps {
            if !publishable.contains(dep) {
                errors.push(format!(
                    "crate '{}' depends on internal crate '{}' which is not publishable",
                    name, dep
                ));
            }
        }
    }
    if !errors.is_empty() {
        for e in errors {
            eprintln!("{e}");
        }
        return Err(io::Error::other(
            "cannot publish due to non-publishable internal dependencies",
        ));
    }

    // Compute publish order (topological: deps first)
    let order = topo_order(&name_to_crate, &publishable).map_err(io::Error::other)?;

    println!("Publish plan (crates.io):");
    for name in &order {
        println!("  - {name}");
    }

    // Execute cargo publish in order
    for name in &order {
        let c = name_to_crate.get(name).unwrap();
        let manifest = c.path.join("Cargo.toml");
        let mut cmd = Command::new("cargo");
        cmd.arg("publish").arg("--manifest-path").arg(&manifest);
        if args.dry_run {
            cmd.arg("--dry-run");
        }

        println!(
            "Running: {}",
            format_command_display(cmd.get_program(), cmd.get_args())
        );

        let status = cmd.status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "cargo publish failed for crate '{}' with status {}",
                name, status
            )));
        }
    }

    if args.dry_run {
        println!("Dry-run complete.");
    } else {
        println!("Publish complete.");
    }

    Ok(())
}

fn is_publishable_to_crates_io(manifest_path: &Path) -> io::Result<bool> {
    let text = fs::read_to_string(manifest_path)?;
    let value: toml::Value = text
        .parse()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e}")))?;

    let pkg = match value.get("package").and_then(|v| v.as_table()) {
        Some(p) => p,
        None => return Ok(false),
    };

    // If publish = false => skip
    if let Some(val) = pkg.get("publish") {
        match val {
            toml::Value::Boolean(false) => return Ok(false),
            toml::Value::Array(arr) => {
                // Only publish if the array contains "crates-io"
                // (Cargo uses this to whitelist registries.)
                let allowed: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                return Ok(allowed.iter().any(|s| s == "crates-io"));
            }
            _ => {}
        }
    }

    // Default case: publishable
    Ok(true)
}

fn topo_order(
    name_to_crate: &BTreeMap<String, &CrateInfo>,
    include: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    // Build graph: edge dep -> crate
    let mut indegree: BTreeMap<&str, usize> = BTreeMap::new();
    let mut forward: BTreeMap<&str, Vec<&str>> = BTreeMap::new();

    for name in include {
        indegree.insert(name.as_str(), 0);
        forward.entry(name.as_str()).or_default();
    }

    for name in include {
        let c = name_to_crate
            .get(name)
            .ok_or_else(|| format!("missing crate info for '{}'", name))?;
        for dep in &c.internal_deps {
            if include.contains(dep) {
                // dep -> name
                let entry = forward.entry(dep.as_str()).or_default();
                entry.push(name.as_str());
                *indegree.get_mut(name.as_str()).unwrap() += 1;
            }
        }
    }

    let mut q: VecDeque<&str> = indegree
        .iter()
        .filter_map(|(k, &d)| if d == 0 { Some(*k) } else { None })
        .collect();
    let mut out: Vec<String> = Vec::new();

    while let Some(n) = q.pop_front() {
        out.push(n.to_string());
        if let Some(children) = forward.get(n) {
            for &m in children {
                if let Some(d) = indegree.get_mut(m) {
                    *d -= 1;
                    if *d == 0 {
                        q.push_back(m);
                    }
                }
            }
        }
    }

    if out.len() != include.len() {
        return Err("dependency cycle detected among publishable crates".into());
    }
    Ok(out)
}

fn format_command_display(program: &std::ffi::OsStr, args: std::process::CommandArgs) -> String {
    let prog = program.to_string_lossy();
    let mut s = String::new();
    s.push_str(&prog);
    for a in args {
        s.push(' ');
        s.push_str(&a.to_string_lossy());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn topo_orders_deps_first() {
        // Build a small fake graph using CrateInfo structures
        let a = CrateInfo {
            name: "a".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/a"),
            internal_deps: BTreeSet::new(),
        };
        let mut deps_b = BTreeSet::new();
        deps_b.insert("a".into());
        let b = CrateInfo {
            name: "b".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/b"),
            internal_deps: deps_b,
        };
        let mut deps_c = BTreeSet::new();
        deps_c.insert("b".into());
        let c = CrateInfo {
            name: "c".into(),
            version: "0.1.0".into(),
            path: PathBuf::from("/tmp/c"),
            internal_deps: deps_c,
        };

        let mut map: BTreeMap<String, &CrateInfo> = BTreeMap::new();
        map.insert("a".into(), &a);
        map.insert("b".into(), &b);
        map.insert("c".into(), &c);

        let mut include = BTreeSet::new();
        include.insert("a".into());
        include.insert("b".into());
        include.insert("c".into());

        let order = topo_order(&map, &include).unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
    }
}
