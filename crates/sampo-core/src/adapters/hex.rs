use crate::errors::{Result, SampoError, WorkspaceError};
use crate::types::{PackageInfo, PackageKind};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tree_sitter::{Language, Node, Parser, Tree};

const MIX_MANIFEST: &str = "mix.exs";
const HEX_API_BASE: &str = "https://hex.pm/api";
const HEX_USER_AGENT: &str = concat!("sampo-core/", env!("CARGO_PKG_VERSION"));
// Hex public docs specify 100 anonymous requests per minute -> https://hexpm.docs.apiary.io/#introduction/rate-limiting
const HEX_RATE_LIMIT: Duration = Duration::from_millis(600);

static HEX_LAST_CALL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

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
        enforce_hex_rate_limit();
        let response = client.get(&url).send().map_err(|e| {
            SampoError::Publish(format!(
                "failed to query Hex registry for '{}': {}",
                name, e
            ))
        })?;

        let status_code = response.status();
        match status_code {
            StatusCode::OK => Ok(true),
            StatusCode::NOT_FOUND => Ok(false),
            StatusCode::TOO_MANY_REQUESTS => {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .map(|value| format!(" Retry-After: {}", value))
                    .unwrap_or_default();
                Err(SampoError::Publish(format!(
                    "Hex registry returned 429 Too Many Requests for '{}@{}'.{}",
                    name, version, retry_after
                )))
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(SampoError::Publish(format!(
                "Hex registry returned {} for '{}@{}'; authentication may be required",
                status_code, name, version
            ))),
            other => {
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
                    other, name, version, body_part
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

fn enforce_hex_rate_limit() {
    let lock = HEX_LAST_CALL.get_or_init(|| Mutex::new(None));
    let mut guard = match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let now = Instant::now();
    if let Some(last_call) = *guard {
        let elapsed = now.saturating_duration_since(last_call);
        if elapsed < HEX_RATE_LIMIT {
            thread::sleep(HEX_RATE_LIMIT - elapsed);
        }
    }
    *guard = Some(now);
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
    let Some(tree) = parse_mix_tree(source) else {
        return ProjectMetadata::default();
    };

    let source_bytes = source.as_bytes();
    let Some(function) = find_function_call(&tree, source_bytes, "project") else {
        return ProjectMetadata::default();
    };
    let Some(keywords) = function_body_keywords(function) else {
        return ProjectMetadata::default();
    };

    let mut metadata = ProjectMetadata::default();
    let mut cursor = keywords.walk();
    for pair in keywords.named_children(&mut cursor) {
        if pair.kind() != "pair" {
            continue;
        }
        let Some((key_node, value_node)) = pair_key_value(pair) else {
            continue;
        };
        let Some(key) = keyword_name(source_bytes, key_node) else {
            continue;
        };
        match key.as_str() {
            "app" => {
                if let Some(app) = parse_atom_node(source_bytes, value_node) {
                    metadata.app = Some(app);
                }
            }
            "version" => {
                if let Some(literal) = parse_string_literal_node(source, value_node) {
                    metadata.version = Some(literal.value);
                }
            }
            "apps_path" => {
                if let Some(literal) = parse_string_literal_node(source, value_node) {
                    metadata.apps_path = Some(PathBuf::from(literal.value));
                }
            }
            _ => {}
        }
    }

    metadata
}

fn locate_project_version_literal(source: &str) -> Option<ValueLiteral> {
    let tree = parse_mix_tree(source)?;
    let source_bytes = source.as_bytes();
    let function = find_function_call(&tree, source_bytes, "project")?;
    let keywords = function_body_keywords(function)?;

    let mut cursor = keywords.walk();
    for pair in keywords.named_children(&mut cursor) {
        if pair.kind() != "pair" {
            continue;
        }
        let Some((key_node, value_node)) = pair_key_value(pair) else {
            continue;
        };
        let key = keyword_name(source_bytes, key_node)?;
        if key == "version" {
            return parse_string_literal_node(source, value_node);
        }
    }

    None
}

fn collect_dependencies(source: &str, manifest_dir: &Path) -> Vec<ParsedDependency> {
    let Some(tree) = parse_mix_tree(source) else {
        return Vec::new();
    };

    let source_bytes = source.as_bytes();
    let mut function_calls = find_function_calls(&tree, source_bytes, "deps");
    if function_calls.is_empty() {
        return Vec::new();
    }

    function_calls.sort_by_key(Node::start_byte);

    let mut deps = Vec::new();
    for function in function_calls {
        let Some(list) = function_body_list(function) else {
            continue;
        };
        let mut cursor = list.walk();
        for item in list.named_children(&mut cursor) {
            if item.kind() != "tuple" {
                continue;
            }
            if let Some(dep) = parse_dependency_tuple(item, source, manifest_dir) {
                deps.push(dep);
            }
        }
    }

    deps
}

fn compute_requirement(old: &str, new_version: &str) -> Option<String> {
    let trimmed = old.trim();
    if trimmed == new_version {
        return None;
    }

    if contains_requirement_conjunction(trimmed) {
        return None;
    }

    const OPERATORS: [&str; 7] = ["~>", "==", ">=", "<=", ">", "<", "="];
    for op in OPERATORS {
        if let Some(rest) = trimmed.strip_prefix(op) {
            let current = rest.trim_start();
            if current.is_empty() {
                return None;
            }
            if !is_single_version_token(current) {
                return None;
            }
            if current == new_version {
                return None;
            }
            return Some(format!("{} {}", op, new_version).trim().to_string());
        }
    }

    if is_single_version_token(trimmed) {
        return Some(new_version.to_string());
    }

    None
}

fn contains_requirement_conjunction(input: &str) -> bool {
    let lowered = input.to_ascii_lowercase();
    lowered.contains(" and ") || lowered.contains(" or ")
}

fn is_single_version_token(candidate: &str) -> bool {
    !candidate.is_empty()
        && !candidate.contains(char::is_whitespace)
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
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

fn parse_mix_tree(source: &str) -> Option<Tree> {
    let language = elixir_language();
    let mut parser = Parser::new();
    parser.set_language(language).ok()?;
    parser.parse(source, None)
}

fn elixir_language() -> &'static Language {
    static LANGUAGE: OnceLock<Language> = OnceLock::new();
    LANGUAGE.get_or_init(|| tree_sitter_elixir::LANGUAGE.into())
}

fn find_function_call<'tree>(
    tree: &'tree Tree,
    source_bytes: &[u8],
    function_name: &str,
) -> Option<Node<'tree>> {
    let mut calls = find_function_calls(tree, source_bytes, function_name);
    calls.sort_by_key(Node::start_byte);
    calls.into_iter().next()
}

fn find_function_calls<'tree>(
    tree: &'tree Tree,
    source_bytes: &[u8],
    function_name: &str,
) -> Vec<Node<'tree>> {
    let mut matches = Vec::new();
    let mut stack = Vec::new();
    stack.push(tree.root_node());

    while let Some(node) = stack.pop() {
        if node.kind() == "call"
            && let Some(target) = call_target(node)
            && let Ok(target_text) = target.utf8_text(source_bytes)
            && matches!(target_text, "def" | "defp")
            && is_function_call_named(node, source_bytes, function_name)
        {
            matches.push(node);
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }

    matches
}

fn is_function_call_named(node: Node<'_>, source_bytes: &[u8], function_name: &str) -> bool {
    let Some(arguments) = first_named_child(node, "arguments") else {
        return false;
    };
    let mut cursor = arguments.walk();
    for child in arguments.named_children(&mut cursor) {
        if child.kind() == "identifier" && child.utf8_text(source_bytes) == Ok(function_name) {
            return true;
        }
    }
    false
}

fn function_body_keywords(function: Node<'_>) -> Option<Node<'_>> {
    let list = function_body_list(function)?;
    find_named_descendant_by_kind(list, "keywords")
}

fn function_body_list(function: Node<'_>) -> Option<Node<'_>> {
    let do_block = first_named_child(function, "do_block")?;
    find_named_descendant_by_kind(do_block, "list")
}

fn find_named_descendant_by_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut stack = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        stack.push(child);
    }

    while let Some(candidate) = stack.pop() {
        if candidate.kind() == kind {
            return Some(candidate);
        }
        let mut child_cursor = candidate.walk();
        for child in candidate.named_children(&mut child_cursor) {
            stack.push(child);
        }
    }

    None
}

fn keyword_name(source_bytes: &[u8], keyword: Node<'_>) -> Option<String> {
    let text = keyword.utf8_text(source_bytes).ok()?;
    let trimmed = text.trim_end();
    let without_colon = trimmed.strip_suffix(':')?;
    Some(without_colon.trim().to_string())
}

fn call_target<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    first_named_child_any(node)
}

fn first_named_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn first_named_child_any<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    let mut iter = node.named_children(&mut cursor);
    iter.next()
}

fn pair_key_value<'tree>(pair: Node<'tree>) -> Option<(Node<'tree>, Node<'tree>)> {
    let mut cursor = pair.walk();
    let mut key = None;
    let mut value = None;
    for child in pair.named_children(&mut cursor) {
        match child.kind() {
            "keyword" | "quoted_keyword" => key = Some(child),
            _ => value = Some(child),
        }
    }
    match (key, value) {
        (Some(k), Some(v)) => Some((k, v)),
        _ => None,
    }
}

fn parse_string_literal_node(source: &str, node: Node<'_>) -> Option<ValueLiteral> {
    if node.kind() != "string" {
        return None;
    }
    let start = node.start_byte();
    let end = node.end_byte();
    if end <= start || end > source.len() {
        return None;
    }
    let text = &source[start..end];
    let mut chars = text.chars();
    let quote = chars.next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let last = text.chars().last()?;
    if last != quote {
        return None;
    }
    let inner_start = start + quote.len_utf8();
    let inner_end = end - quote.len_utf8();
    if inner_end < inner_start || inner_end > source.len() {
        return None;
    }
    let value = source[inner_start..inner_end].to_string();
    Some(ValueLiteral {
        start,
        end,
        quote,
        value,
    })
}

fn parse_atom_node(source_bytes: &[u8], node: Node<'_>) -> Option<String> {
    if node.kind() != "atom" {
        return None;
    }
    let text = node.utf8_text(source_bytes).ok()?;
    let trimmed = text.trim();
    let value = trimmed.strip_prefix(':')?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_dependency_tuple(
    tuple: Node<'_>,
    source: &str,
    manifest_dir: &Path,
) -> Option<ParsedDependency> {
    let source_bytes = source.as_bytes();
    let mut cursor = tuple.walk();
    let children: Vec<Node<'_>> = tuple.named_children(&mut cursor).collect();
    if children.is_empty() {
        return None;
    }

    let name_node = children[0];
    let name = parse_atom_node(source_bytes, name_node)?;

    let mut requirement = None;
    let mut path = None;
    for child in children.into_iter().skip(1) {
        match child.kind() {
            "string" if requirement.is_none() => {
                requirement = parse_string_literal_node(source, child);
            }
            "keywords" => {
                if path.is_none() {
                    path = path_from_keywords(child, source, manifest_dir);
                }
            }
            _ => {}
        }
    }

    Some(ParsedDependency {
        name,
        requirement,
        path,
    })
}

fn path_from_keywords(keywords: Node<'_>, source: &str, manifest_dir: &Path) -> Option<PathBuf> {
    let source_bytes = source.as_bytes();
    let mut cursor = keywords.walk();
    for pair in keywords.named_children(&mut cursor) {
        if pair.kind() != "pair" {
            continue;
        }
        let Some((key_node, value_node)) = pair_key_value(pair) else {
            continue;
        };
        if keyword_name(source_bytes, key_node)?.as_str() != "path" {
            continue;
        }
        if let Some(literal) = parse_string_literal_node(source, value_node) {
            return Some(manifest_dir.join(literal.value));
        }
    }
    None
}
