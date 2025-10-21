use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const MIX_MANIFEST: &str = "mix.exs";
const HEX_API_BASE: &str = "https://hex.pm/api";
const HEX_USER_AGENT: &str = concat!("sampo-core/", env!("CARGO_PKG_VERSION"));

/// Stateless adapter for Hex/Mix workspaces.
pub(super) struct HexAdapter;

impl HexAdapter {
    pub(super) fn can_discover(&self, root: &Path) -> bool {
        root.join(MIX_MANIFEST).exists()
    }

    pub(super) fn discover(
        &self,
        root: &Path,
    ) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
        discover_hex(root)
    }

    pub(super) fn manifest_path(&self, package_dir: &Path) -> PathBuf {
        package_dir.join(MIX_MANIFEST)
    }

    pub(super) fn is_publishable(&self, manifest_path: &Path) -> Result<bool> {
        let text = fs::read_to_string(manifest_path)
            .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
        let ProjectMetadata { app, version, .. } = parse_project_metadata(&text);

        let Some(app) = app else {
            return Err(SampoError::Publish(format!(
                "Manifest {} is missing an :app declaration",
                manifest_path.display()
            )));
        };
        if app.trim().is_empty() {
            return Err(SampoError::Publish(format!(
                "Manifest {} declares an empty app name",
                manifest_path.display()
            )));
        }

        let Some(version) = version else {
            return Err(SampoError::Publish(format!(
                "Manifest {} is missing a version field",
                manifest_path.display()
            )));
        };
        if version.trim().is_empty() {
            return Err(SampoError::Publish(format!(
                "Manifest {} declares an empty version",
                manifest_path.display()
            )));
        }

        Ok(true)
    }

    pub(super) fn version_exists(
        &self,
        package_name: &str,
        version: &str,
        _manifest_path: Option<&Path>,
    ) -> Result<bool> {
        let name = package_name.trim();
        if name.is_empty() {
            return Err(SampoError::Publish(
                "Package name cannot be empty when checking Hex registry".into(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(HEX_USER_AGENT)
            .build()
            .map_err(|e| {
                SampoError::Publish(format!("failed to build HTTP client for Hex: {}", e))
            })?;

        let url = format!("{HEX_API_BASE}/packages/{}/releases/{}", name, version);
        let response = client.get(&url).send().map_err(|e| {
            SampoError::Publish(format!(
                "failed to query Hex registry for '{}': {}",
                name, e
            ))
        })?;

        match response.status() {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(SampoError::Publish(format!(
                "Hex registry returned {} for '{}@{}'; authentication may be required",
                response.status(),
                name,
                version
            ))),
            status => {
                let body = response.text().unwrap_or_default();
                let snippet: String = body.trim().chars().take(300).collect();
                let snippet = snippet.split_whitespace().collect::<Vec<_>>().join(" ");
                let body_part = if snippet.is_empty() {
                    String::new()
                } else {
                    format!(" body=\"{}\"", snippet)
                };
                Err(SampoError::Publish(format!(
                    "Hex registry returned {} for '{}@{}'{}",
                    status, name, version, body_part
                )))
            }
        }
    }

    pub(super) fn publish(
        &self,
        manifest_path: &Path,
        dry_run: bool,
        extra_args: &[String],
    ) -> Result<()> {
        let manifest_dir = manifest_path.parent().ok_or_else(|| {
            SampoError::Publish(format!(
                "Manifest {} does not have a parent directory",
                manifest_path.display()
            ))
        })?;

        let text = fs::read_to_string(manifest_path)
            .map_err(|e| SampoError::Io(crate::errors::io_error_with_path(e, manifest_path)))?;
        let ProjectMetadata { app, version, .. } = parse_project_metadata(&text);
        let package = app.ok_or_else(|| {
            SampoError::Publish(format!(
                "Manifest {} is missing an :app declaration",
                manifest_path.display()
            ))
        })?;

        let version = version.ok_or_else(|| {
            SampoError::Publish(format!(
                "Manifest {} is missing a version field",
                manifest_path.display()
            ))
        })?;
        if version.trim().is_empty() {
            return Err(SampoError::Publish(format!(
                "Manifest {} declares an empty version",
                manifest_path.display()
            )));
        }

        let mut cmd = Command::new("mix");
        cmd.current_dir(manifest_dir);
        cmd.arg("hex.publish");

        if dry_run && !has_flag(extra_args, "--dry-run") {
            cmd.arg("--dry-run");
        }

        if !has_flag(extra_args, "--yes") {
            cmd.arg("--yes");
        }

        if !extra_args.is_empty() {
            cmd.args(extra_args);
        }

        println!("Running: {}", format_command_display(&cmd));

        let status = cmd.status()?;
        if !status.success() {
            return Err(SampoError::Publish(format!(
                "mix hex.publish failed for {} (package '{}') with status {}",
                manifest_path.display(),
                package,
                status
            )));
        }

        Ok(())
    }

    pub(super) fn regenerate_lockfile(&self, workspace_root: &Path) -> Result<()> {
        regenerate_mix_lockfile(workspace_root)
    }
}

fn regenerate_mix_lockfile(workspace_root: &Path) -> Result<()> {
    let manifest_path = workspace_root.join(MIX_MANIFEST);
    if !manifest_path.exists() {
        return Err(SampoError::Release(format!(
            "cannot regenerate mix.lock; {} not found in {}",
            MIX_MANIFEST,
            workspace_root.display()
        )));
    }

    println!("Regenerating mix.lock using mix…");

    let mut cmd = Command::new("mix");
    cmd.arg("deps.get");
    cmd.current_dir(workspace_root);

    println!("Running: {}", format_command_display(&cmd));

    let status = cmd.status().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            SampoError::Release(
                "mix command not found; install Elixir to regenerate mix.lock".to_string(),
            )
        } else {
            SampoError::Io(err)
        }
    })?;

    if !status.success() {
        return Err(SampoError::Release(format!(
            "mix deps.get failed with status {}",
            status
        )));
    }

    println!("mix.lock updated.");
    Ok(())
}

/// Update a Mix manifest with a new package version and refreshed dependency requirements.
pub fn update_manifest_versions(
    manifest_path: &Path,
    input: &str,
    new_pkg_version: Option<&str>,
    new_version_by_name: &BTreeMap<String, String>,
) -> Result<(String, Vec<(String, String)>)> {
    let mut replacements = Vec::new();
    let mut updated_deps: BTreeSet<String> = BTreeSet::new();

    if let Some(target_version) = new_pkg_version {
        let version_literal = locate_project_version_literal(input).ok_or_else(|| {
            SampoError::Release(format!(
                "Manifest {} is missing a version field",
                manifest_path.display()
            ))
        })?;

        if version_literal.value != target_version {
            replacements.push(Replacement {
                start: version_literal.start,
                end: version_literal.end,
                replacement: format!(
                    "{}{}{}",
                    version_literal.quote, target_version, version_literal.quote
                ),
            });
        }
    }

    let dependencies = collect_dependencies(
        input,
        manifest_path.parent().unwrap_or_else(|| Path::new(".")),
    );

    for dep in dependencies {
        let Some(new_version) = new_version_by_name.get(&dep.name) else {
            continue;
        };

        let Some(requirement) = dep.requirement else {
            continue;
        };

        if let Some(new_spec) = compute_requirement(&requirement.value, new_version)
            && new_spec != requirement.value
        {
            replacements.push(Replacement {
                start: requirement.start,
                end: requirement.end,
                replacement: format!("{}{}{}", requirement.quote, new_spec, requirement.quote),
            });
            updated_deps.insert(dep.name);
        }
    }

    if replacements.is_empty() {
        let applied: Vec<(String, String)> = Vec::new();
        return Ok((input.to_string(), applied));
    }

    replacements.sort_by(|a, b| a.start.cmp(&b.start));
    let mut output = input.to_string();
    for replacement in replacements.into_iter().rev() {
        output.replace_range(replacement.start..replacement.end, &replacement.replacement);
    }

    let applied = updated_deps
        .into_iter()
        .filter_map(|name| {
            new_version_by_name
                .get(&name)
                .cloned()
                .map(|version| (name, version))
        })
        .collect();

    Ok((output, applied))
}

fn discover_hex(root: &Path) -> std::result::Result<Vec<PackageInfo>, WorkspaceError> {
    let manifest_path = root.join(MIX_MANIFEST);
    if !manifest_path.exists() {
        return Err(WorkspaceError::InvalidWorkspace(format!(
            "Expected {} in {}",
            MIX_MANIFEST,
            root.display()
        )));
    }

    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, &manifest_path)))?;
    let project_meta = parse_project_metadata(&manifest_text);

    let mut package_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    if project_meta.app.is_some() {
        package_dirs.insert(normalize_path(root));
    }

    if let Some(apps_path) = project_meta.apps_path {
        let apps_dir = normalize_path(&root.join(apps_path));
        if apps_dir.exists() {
            for entry in fs::read_dir(&apps_dir)
                .map_err(|e| WorkspaceError::Io(crate::errors::io_error_with_path(e, &apps_dir)))?
            {
                let entry = entry.map_err(WorkspaceError::Io)?;
                if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    continue;
                }
                let dir = entry.path();
                if dir.join(MIX_MANIFEST).exists() {
                    package_dirs.insert(normalize_path(&dir));
                }
            }
        }
    }

    let mut manifests = Vec::new();
    let mut name_to_path: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut normalized_to_name: BTreeMap<PathBuf, String> = BTreeMap::new();

    for dir in package_dirs {
        let manifest_path = dir.join(MIX_MANIFEST);
        let text = fs::read_to_string(&manifest_path).map_err(|e| {
            WorkspaceError::Io(crate::errors::io_error_with_path(e, &manifest_path))
        })?;
        let meta = parse_project_metadata(&text);
        let app = meta.app.ok_or_else(|| {
            WorkspaceError::InvalidManifest(format!(
                "missing app name in {}",
                manifest_path.display()
            ))
        })?;
        let version = meta.version.unwrap_or_default();
        let deps = collect_dependencies(&text, &dir);

        name_to_path.insert(app.clone(), dir.clone());
        normalized_to_name.insert(normalize_path(&dir), app.clone());
        manifests.push((app, version, dir, deps));
    }

    let mut packages = Vec::new();
    for (name, version, dir, deps) in manifests {
        let identifier = PackageInfo::dependency_identifier(PackageKind::Hex, &name);
        let mut internal = BTreeSet::new();

        for dep in deps {
            if let Some(path) = dep.path {
                let normalized = normalize_path(&path);
                if let Some(dep_name) = normalized_to_name.get(&normalized) {
                    internal.insert(PackageInfo::dependency_identifier(
                        PackageKind::Hex,
                        dep_name,
                    ));
                    continue;
                }
            }

            if name_to_path.contains_key(&dep.name) {
                internal.insert(PackageInfo::dependency_identifier(
                    PackageKind::Hex,
                    &dep.name,
                ));
            }
        }

        packages.push(PackageInfo {
            name,
            identifier,
            version,
            path: dir,
            internal_deps: internal,
            kind: PackageKind::Hex,
        });
    }

    Ok(packages)
}

/// Metadata parsed from the `project` keyword list.
#[derive(Debug, Default)]
struct ProjectMetadata {
    app: Option<String>,
    version: Option<String>,
    apps_path: Option<PathBuf>,
}

/// Representation of a dependency entry in the Mix manifest.
#[derive(Debug, Clone)]
struct ParsedDependency {
    name: String,
    requirement: Option<ValueLiteral>,
    path: Option<PathBuf>,
}

/// String literal extracted from the manifest with location metadata.
#[derive(Debug, Clone)]
struct ValueLiteral {
    start: usize,
    end: usize,
    quote: char,
    value: String,
}

#[derive(Debug)]
struct Replacement {
    start: usize,
    end: usize,
    replacement: String,
}

fn parse_project_metadata(source: &str) -> ProjectMetadata {
    let mut metadata = ProjectMetadata::default();

    if let Some(project_span) = find_function_keyword_list(source, &["def project do"]) {
        for entry_span in split_top_level_ranges(source, project_span.clone()) {
            if let Some((key, value_span)) = parse_keyword_entry(source, entry_span) {
                match key.as_str() {
                    "app" => {
                        if let Some(app) = parse_atom(source, value_span) {
                            metadata.app = Some(app);
                        }
                    }
                    "version" => {
                        if let Some(literal) = parse_string_literal(source, value_span) {
                            metadata.version = Some(literal.value);
                        }
                    }
                    "apps_path" => {
                        if let Some(literal) = parse_string_literal(source, value_span) {
                            metadata.apps_path = Some(PathBuf::from(literal.value));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    metadata
}

fn locate_project_version_literal(source: &str) -> Option<ValueLiteral> {
    let project_span = find_function_keyword_list(source, &["def project do"])?;
    for entry_span in split_top_level_ranges(source, project_span) {
        if let Some((key, value_span)) = parse_keyword_entry(source, entry_span)
            && key == "version"
        {
            return parse_string_literal(source, value_span);
        }
    }
    None
}

fn collect_dependencies(source: &str, manifest_dir: &Path) -> Vec<ParsedDependency> {
    let mut out = Vec::new();
    let deps_span = find_function_keyword_list(source, &["defp deps do", "def deps do"]);
    let Some(span) = deps_span else {
        return out;
    };

    for entry_span in split_top_level_ranges(source, span) {
        if let Some(dep) = parse_dependency_entry(source, entry_span.clone(), manifest_dir) {
            out.push(dep);
        }
    }

    out
}

fn parse_dependency_entry(
    source: &str,
    span: std::ops::Range<usize>,
    manifest_dir: &Path,
) -> Option<ParsedDependency> {
    let trimmed = trim_range(source, span);
    if trimmed.is_empty() {
        return None;
    }

    let inner = strip_enclosing(source, trimmed.clone(), '{', '}')?;
    let parts = split_top_level_ranges(source, inner);
    if parts.is_empty() {
        return None;
    }

    let name = parse_atom(source, parts[0].clone())?;
    let mut requirement = None;
    let mut path = None;
    let mut option_index = 1;

    if parts.len() >= 2
        && let Some(literal) = parse_string_literal(source, parts[1].clone())
    {
        requirement = Some(literal);
        option_index = 2;
    }

    for part in parts.into_iter().skip(option_index) {
        if let Some((key, value_span)) = parse_keyword_entry(source, part)
            && key == "path"
            && let Some(literal) = parse_string_literal(source, value_span)
        {
            let joined = manifest_dir.join(&literal.value);
            path = Some(joined);
        }
    }

    Some(ParsedDependency {
        name,
        requirement,
        path,
    })
}

fn find_function_keyword_list(source: &str, patterns: &[&str]) -> Option<std::ops::Range<usize>> {
    for pattern in patterns {
        if let Some(idx) = source.find(pattern)
            && let Some(range) = find_bracketed_list(source, idx + pattern.len())
        {
            return Some(range);
        }
    }
    None
}

fn find_bracketed_list(source: &str, start_idx: usize) -> Option<std::ops::Range<usize>> {
    let mut in_string: Option<char> = None;
    let mut escape = false;
    let mut list_start = None;
    for (offset, ch) in source[start_idx..].char_indices() {
        let idx = start_idx + offset;
        if let Some(q) = in_string {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == q {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '[' => {
                list_start = Some(idx);
                break;
            }
            _ => {}
        }
    }

    let list_start = list_start?;
    let mut depth = 0usize;
    escape = false;
    in_string = None;
    for (offset, ch) in source[list_start..].char_indices() {
        let abs = list_start + offset;
        if let Some(q) = in_string {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == q {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((list_start + 1)..abs);
                }
            }
            _ => {}
        }
    }

    None
}

fn parse_keyword_entry(
    source: &str,
    span: std::ops::Range<usize>,
) -> Option<(String, std::ops::Range<usize>)> {
    let trimmed = trim_range(source, span);
    if trimmed.is_empty() {
        return None;
    }

    let slice = &source[trimmed.clone()];
    let mut colon_offset = None;
    for (offset, ch) in slice.char_indices() {
        if ch == ':' {
            colon_offset = Some(trimmed.start + offset);
            break;
        }
    }
    let colon = colon_offset?;

    let key_range = trimmed.start..colon;
    let key = source[key_range.clone()].trim().to_string();
    if key.is_empty() {
        return None;
    }

    let mut value_start = colon + 1;
    while value_start < trimmed.end {
        let ch = source[value_start..].chars().next().unwrap();
        if ch.is_whitespace() {
            value_start += ch.len_utf8();
        } else {
            break;
        }
    }

    let value_range = value_start..trimmed.end;
    Some((key, value_range))
}

fn parse_atom(source: &str, span: std::ops::Range<usize>) -> Option<String> {
    let trimmed = trim_range(source, span);
    if trimmed.is_empty() {
        return None;
    }

    let mut chars = source[trimmed.clone()].chars();
    if chars.next()? != ':' {
        return None;
    }

    let mut atom = String::new();
    for ch in chars {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            atom.push(ch);
        } else {
            break;
        }
    }

    if atom.is_empty() { None } else { Some(atom) }
}

fn parse_string_literal(source: &str, span: std::ops::Range<usize>) -> Option<ValueLiteral> {
    let trimmed = trim_range(source, span);
    if trimmed.is_empty() {
        return None;
    }

    let mut iter = source[trimmed.clone()].char_indices();
    let (offset, first) = iter.next()?;
    if offset != 0 {
        return None;
    }
    if first != '"' && first != '\'' {
        return None;
    }
    let quote = first;
    let mut escape = false;
    for (i, ch) in iter {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if ch == quote {
            let start = trimmed.start;
            let end = trimmed.start + i + quote.len_utf8();
            let inner_start = start + quote.len_utf8();
            let inner_end = trimmed.start + i;
            let value = source[inner_start..inner_end].to_string();
            return Some(ValueLiteral {
                start,
                end,
                quote,
                value,
            });
        }
    }
    None
}

fn strip_enclosing(
    source: &str,
    span: std::ops::Range<usize>,
    open: char,
    close: char,
) -> Option<std::ops::Range<usize>> {
    let trimmed = trim_range(source, span);
    if trimmed.is_empty() {
        return None;
    }

    let mut start = trimmed.start;
    let mut first = None;
    while start < trimmed.end {
        let ch = source[start..].chars().next().unwrap();
        if ch.is_whitespace() {
            start += ch.len_utf8();
        } else {
            first = Some((start, ch));
            break;
        }
    }
    let (first_idx, first_char) = first?;
    if first_char != open {
        return None;
    }
    let mut end = trimmed.end;
    let mut last = None;
    while end > first_idx {
        let ch = source[..end].chars().next_back().unwrap();
        if ch.is_whitespace() {
            end -= ch.len_utf8();
        } else {
            last = Some((end - ch.len_utf8(), ch));
            break;
        }
    }
    let (last_idx, last_char) = last?;
    if last_char != close || last_idx < first_idx {
        return None;
    }

    Some((first_idx + open.len_utf8())..last_idx)
}

fn split_top_level_ranges(
    source: &str,
    span: std::ops::Range<usize>,
) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut depth_brace = 0usize;
    let mut depth_bracket = 0usize;
    let mut depth_paren = 0usize;
    let mut in_string: Option<char> = None;
    let mut escape = false;
    let mut item_start = span.start;

    for (offset, ch) in source[span.clone()].char_indices() {
        let idx = span.start + offset;
        if let Some(q) = in_string {
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
                continue;
            }
            if ch == q {
                in_string = None;
            }
            continue;
        }

        match ch {
            '"' | '\'' => in_string = Some(ch),
            '{' => depth_brace += 1,
            '}' => depth_brace = depth_brace.saturating_sub(1),
            '[' => depth_bracket += 1,
            ']' => depth_bracket = depth_bracket.saturating_sub(1),
            '(' => depth_paren += 1,
            ')' => depth_paren = depth_paren.saturating_sub(1),
            ',' if depth_brace == 0 && depth_bracket == 0 && depth_paren == 0 => {
                let range = item_start..idx;
                if !range.is_empty() {
                    ranges.push(range);
                }
                item_start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }

    if item_start < span.end {
        ranges.push(item_start..span.end);
    }

    ranges
}

fn trim_range(source: &str, span: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let mut start = span.start;
    let mut end = span.end;

    while start < end {
        let ch = source[start..].chars().next().unwrap();
        if ch.is_whitespace() {
            start += ch.len_utf8();
        } else {
            break;
        }
    }

    while end > start {
        let ch = source[..end].chars().next_back().unwrap();
        if ch.is_whitespace() {
            end -= ch.len_utf8();
        } else {
            break;
        }
    }

    start..end
}

fn compute_requirement(old: &str, new_version: &str) -> Option<String> {
    let trimmed = old.trim();
    if trimmed == new_version {
        return None;
    }

    for op in ["~>", "==", ">=", "<=", "="] {
        if let Some(rest) = trimmed.strip_prefix(op) {
            let current = rest.trim();
            if current == new_version {
                return None;
            }
            return Some(format!("{} {}", op, new_version).trim().to_string());
        }
    }

    if trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Some(new_version.to_string());
    }

    None
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    out.components().next_back(),
                    Some(Component::RootDir | Component::Prefix(_))
                ) {
                    out.pop();
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => out.push(component),
        }
    }
    out
}

fn has_flag(args: &[String], flag: &str) -> bool {
    let prefix = format!("{flag}=");
    for arg in args {
        if arg == flag || arg.starts_with(&prefix) {
            return true;
        }
    }
    false
}

fn format_command_display(cmd: &Command) -> String {
    let mut text = cmd.get_program().to_string_lossy().into_owned();
    for arg in cmd.get_args() {
        text.push(' ');
        text.push_str(&arg.to_string_lossy());
    }
    text
}

#[cfg(test)]
mod hex_tests;
