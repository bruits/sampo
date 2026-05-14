use crate::errors::SampoError;
use crate::types::PackageKind;
use semver::Version;

/// Placeholder tokens supported by tag templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    Ecosystem,
    PackageName,
    Version,
}

impl Placeholder {
    fn from_token(token: &str) -> Option<Self> {
        match token {
            "ecosystem" => Some(Self::Ecosystem),
            "package_name" => Some(Self::PackageName),
            "version" => Some(Self::Version),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Literal(String),
    Placeholder(Placeholder),
}

/// A parsed git tag template (e.g. `"{ecosystem}-{package_name}-v{version}"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagTemplate {
    raw: String,
    segments: Vec<Segment>,
}

/// Captured fields from a tag string matched against a template.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TagMatch {
    pub ecosystem: Option<PackageKind>,
    pub package_name: Option<String>,
    pub version: Option<String>,
}

impl TagTemplate {
    /// Recognised placeholders: `{ecosystem}`, `{package_name}`, `{version}`.
    /// `{` / `}` are not escapable.
    pub fn parse(input: &str) -> Result<Self, SampoError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(SampoError::Config(
                "tag format template cannot be empty".to_string(),
            ));
        }

        let mut segments = Vec::new();
        let mut buffer = String::new();
        let mut chars = trimmed.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            match c {
                '{' => {
                    let rest = &trimmed[i + 1..];
                    let close_offset = rest.find('}').ok_or_else(|| {
                        SampoError::Config(format!(
                            "tag format template '{trimmed}' has an unterminated placeholder"
                        ))
                    })?;
                    let token = &rest[..close_offset];
                    let placeholder = Placeholder::from_token(token).ok_or_else(|| {
                        SampoError::Config(format!(
                            "tag format template '{trimmed}' contains unknown placeholder '{{{token}}}'. \
                             Allowed placeholders: {{ecosystem}}, {{package_name}}, {{version}}."
                        ))
                    })?;
                    if !buffer.is_empty() {
                        segments.push(Segment::Literal(std::mem::take(&mut buffer)));
                    }
                    segments.push(Segment::Placeholder(placeholder));
                    let resume = i + 1 + close_offset + 1;
                    while let Some(&(idx, _)) = chars.peek() {
                        if idx < resume {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                }
                '}' => {
                    return Err(SampoError::Config(format!(
                        "tag format template '{trimmed}' has an unmatched '}}'"
                    )));
                }
                _ => buffer.push(c),
            }
        }
        if !buffer.is_empty() {
            segments.push(Segment::Literal(buffer));
        }

        if !segments
            .iter()
            .any(|s| matches!(s, Segment::Placeholder(Placeholder::Version)))
        {
            return Err(SampoError::Config(format!(
                "tag format template '{trimmed}' must include the {{version}} placeholder"
            )));
        }

        for window in segments.windows(2) {
            if matches!(window[0], Segment::Placeholder(_))
                && matches!(window[1], Segment::Placeholder(_))
            {
                return Err(SampoError::Config(format!(
                    "tag format template '{trimmed}' has adjacent placeholders without a separator; \
                     insert a literal character (e.g. '-') between them"
                )));
            }
        }

        Ok(Self {
            raw: trimmed.to_string(),
            segments,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    /// Panics on invalid input. For hard-coded defaults only.
    pub(crate) fn from_static(input: &'static str) -> Self {
        Self::parse(input)
            .unwrap_or_else(|err| panic!("invalid built-in tag template '{input}': {err}"))
    }

    pub fn contains(&self, placeholder: Placeholder) -> bool {
        self.segments
            .iter()
            .any(|s| matches!(s, Segment::Placeholder(p) if *p == placeholder))
    }

    pub fn render(&self, kind: PackageKind, package_name: &str, version: &str) -> String {
        let mut out = String::new();
        for segment in &self.segments {
            match segment {
                Segment::Literal(text) => out.push_str(text),
                Segment::Placeholder(Placeholder::Ecosystem) => out.push_str(kind.as_str()),
                Segment::Placeholder(Placeholder::PackageName) => out.push_str(package_name),
                Segment::Placeholder(Placeholder::Version) => out.push_str(version),
            }
        }
        out
    }

    /// Returns `None` when the tag doesn't fit the template, the `{ecosystem}`
    /// capture isn't a known kind, or `{version}` isn't valid semver.
    pub fn match_tag(&self, tag: &str) -> Option<TagMatch> {
        let mut state = MatchState::default();
        if match_segments(&self.segments, tag, &mut state) {
            Some(TagMatch {
                ecosystem: state.ecosystem,
                package_name: state.package_name,
                version: state.version,
            })
        } else {
            None
        }
    }
}

#[derive(Default, Clone)]
struct MatchState {
    ecosystem: Option<PackageKind>,
    package_name: Option<String>,
    version: Option<String>,
}

fn match_segments(segments: &[Segment], input: &str, state: &mut MatchState) -> bool {
    if segments.is_empty() {
        return input.is_empty();
    }

    match &segments[0] {
        Segment::Literal(text) => {
            if let Some(rest) = input.strip_prefix(text.as_str()) {
                match_segments(&segments[1..], rest, state)
            } else {
                false
            }
        }
        Segment::Placeholder(placeholder) => {
            // `parse()` rejects adjacent placeholders, so the next segment is
            // either a bounding literal or absent (placeholder takes the rest).
            let next_literal = match segments.get(1) {
                Some(Segment::Literal(text)) => Some(text.as_str()),
                _ => None,
            };

            let (candidates, advance_after_capture) = match next_literal {
                Some(literal) => (
                    enumerate_literal_positions(input, literal),
                    Some(literal.len()),
                ),
                None => (vec![input.len()], None),
            };

            for end in candidates {
                let captured = &input[..end];
                let rest_start = end + advance_after_capture.unwrap_or(0);
                if rest_start > input.len() {
                    continue;
                }
                let rest = &input[rest_start..];

                let mut local = state.clone();
                if !apply_capture(&mut local, *placeholder, captured) {
                    continue;
                }

                let next_segments = if advance_after_capture.is_some() {
                    &segments[2..]
                } else {
                    &segments[1..]
                };

                if match_segments(next_segments, rest, &mut local) {
                    *state = local;
                    return true;
                }
            }

            false
        }
    }
}

/// Non-overlapping byte offsets of `literal` in `input`. Realistic tag
/// separators (`-`, `-v`, `/`) don't self-overlap, so this is fine.
fn enumerate_literal_positions(input: &str, literal: &str) -> Vec<usize> {
    if literal.is_empty() {
        return vec![input.len()];
    }
    let mut positions = Vec::new();
    let mut start = 0;
    while let Some(pos) = input[start..].find(literal) {
        let absolute = start + pos;
        positions.push(absolute);
        start = absolute + literal.len();
    }
    positions
}

fn apply_capture(state: &mut MatchState, placeholder: Placeholder, captured: &str) -> bool {
    if captured.is_empty() {
        return false;
    }
    match placeholder {
        Placeholder::Ecosystem => match PackageKind::parse(captured) {
            Some(kind) => {
                if let Some(existing) = state.ecosystem
                    && existing != kind
                {
                    return false;
                }
                state.ecosystem = Some(kind);
                true
            }
            None => false,
        },
        Placeholder::PackageName => {
            if let Some(existing) = &state.package_name
                && existing != captured
            {
                return false;
            }
            state.package_name = Some(captured.to_string());
            true
        }
        Placeholder::Version => {
            if Version::parse(captured).is_err() {
                return false;
            }
            if let Some(existing) = &state.version
                && existing != captured
            {
                return false;
            }
            state.version = Some(captured.to_string());
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template(s: &str) -> TagTemplate {
        TagTemplate::parse(s).unwrap()
    }

    #[test]
    fn parses_default_template() {
        let t = template("{ecosystem}-{package_name}-v{version}");
        assert_eq!(t.as_str(), "{ecosystem}-{package_name}-v{version}");
        assert!(t.contains(Placeholder::Ecosystem));
        assert!(t.contains(Placeholder::PackageName));
        assert!(t.contains(Placeholder::Version));
    }

    #[test]
    fn rejects_unknown_placeholder() {
        let err = TagTemplate::parse("{kind}-{version}").unwrap_err();
        assert!(format!("{err}").contains("unknown placeholder"));
    }

    #[test]
    fn rejects_template_without_version() {
        let err = TagTemplate::parse("{package_name}").unwrap_err();
        assert!(format!("{err}").contains("{version}"));
    }

    #[test]
    fn rejects_unterminated_placeholder() {
        let err = TagTemplate::parse("{version").unwrap_err();
        assert!(format!("{err}").contains("unterminated"));
    }

    #[test]
    fn rejects_unmatched_close_brace() {
        let err = TagTemplate::parse("v}{version}").unwrap_err();
        assert!(format!("{err}").contains("unmatched"));
    }

    #[test]
    fn renders_default_template() {
        let t = template("{ecosystem}-{package_name}-v{version}");
        assert_eq!(
            t.render(PackageKind::Cargo, "sampo-core", "1.2.3"),
            "cargo-sampo-core-v1.2.3"
        );
        assert_eq!(
            t.render(PackageKind::Npm, "@scope/foo", "0.1.0"),
            "npm-@scope/foo-v0.1.0"
        );
    }

    #[test]
    fn renders_short_template() {
        let t = template("v{version}");
        assert_eq!(
            t.render(PackageKind::Packagist, "ignored", "1.0.0"),
            "v1.0.0"
        );
    }

    #[test]
    fn matches_default_template() {
        let t = template("{ecosystem}-{package_name}-v{version}");
        let m = t.match_tag("cargo-my-pkg-v1.2.3").unwrap();
        assert_eq!(m.ecosystem, Some(PackageKind::Cargo));
        assert_eq!(m.package_name.as_deref(), Some("my-pkg"));
        assert_eq!(m.version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn matches_legacy_template_with_v_in_prerelease() {
        let t = template("{package_name}-v{version}");
        let m = t.match_tag("my-pkg-v1.2.3-v1").unwrap();
        assert_eq!(m.package_name.as_deref(), Some("my-pkg"));
        assert_eq!(m.version.as_deref(), Some("1.2.3-v1"));
    }

    #[test]
    fn matches_short_template() {
        let t = template("v{version}");
        let m = t.match_tag("v1.2.3").unwrap();
        assert_eq!(m.version.as_deref(), Some("1.2.3"));
        assert!(m.package_name.is_none());
        assert!(m.ecosystem.is_none());
    }

    #[test]
    fn rejects_invalid_semver() {
        let t = template("v{version}");
        assert!(t.match_tag("vfoo").is_none());
        assert!(t.match_tag("v1.2").is_none());
    }

    #[test]
    fn rejects_unknown_ecosystem() {
        let t = template("{ecosystem}-{package_name}-v{version}");
        assert!(t.match_tag("ruby-foo-v1.2.3").is_none());
    }

    #[test]
    fn matches_with_unusual_separator() {
        let t = template("{ecosystem}/{package_name}@{version}");
        let m = t.match_tag("npm/foo@1.0.0").unwrap();
        assert_eq!(m.ecosystem, Some(PackageKind::Npm));
        assert_eq!(m.package_name.as_deref(), Some("foo"));
        assert_eq!(m.version.as_deref(), Some("1.0.0"));
    }

    #[test]
    fn preserves_non_ascii_literal_segments() {
        let t = template("{package_name}—v{version}");
        assert_eq!(t.render(PackageKind::Cargo, "café", "1.2.3"), "café—v1.2.3");
        let m = t.match_tag("café—v1.2.3").unwrap();
        assert_eq!(m.package_name.as_deref(), Some("café"));
        assert_eq!(m.version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn rejects_empty_capture() {
        let t = template("{package_name}-v{version}");
        assert!(t.match_tag("-v1.2.3").is_none());
    }

    #[test]
    fn rejects_template_without_version_placeholder() {
        assert!(TagTemplate::parse("{package_name}-stable").is_err());
    }

    #[test]
    fn template_can_omit_optional_placeholders() {
        let t = template("release-{version}");
        assert_eq!(
            t.render(PackageKind::Cargo, "anything", "9.9.9"),
            "release-9.9.9"
        );
        let m = t.match_tag("release-9.9.9").unwrap();
        assert_eq!(m.version.as_deref(), Some("9.9.9"));
    }
}
