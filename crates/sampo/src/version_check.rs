//! Checks whether the installed Sampo CLI is up-to-date by querying crates.io.
//!
//! Uses a simple file-based cache to avoid excessive network requests. The cache
//! stores the last-known latest version and a timestamp, and is refreshed when
//! the cache is older than a configurable TTL (default: 24 hours).

use semver::Version;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default cache TTL: 24 hours in seconds.
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

/// Timeout for the crates.io API request.
const REQUEST_TIMEOUT_SECS: u64 = 3;

/// Current CLI version (from Cargo.toml at compile time).
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Result of a version check operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionCheckResult {
    /// A newer version is available on crates.io.
    UpdateAvailable { current: String, latest: String },
    /// The current version is up-to-date.
    UpToDate,
    /// The check was skipped (e.g., due to cache or network issues).
    Skipped,
}

/// Cache entry storing the latest known version and the check timestamp.
#[derive(Debug)]
struct CacheEntry {
    version: String,
    timestamp: u64,
}

impl CacheEntry {
    fn parse(content: &str) -> Option<Self> {
        let mut lines = content.lines();
        let version = lines.next()?.trim().to_string();
        let timestamp: u64 = lines.next()?.trim().parse().ok()?;
        if version.is_empty() {
            return None;
        }
        Some(CacheEntry { version, timestamp })
    }

    fn serialize(&self) -> String {
        format!("{}\n{}", self.version, self.timestamp)
    }
}

/// Returns the path to the version cache file.
///
/// Uses the platform-appropriate cache directory:
/// - Linux: `$XDG_CACHE_HOME/sampo` or `~/.cache/sampo`
/// - macOS: `~/Library/Caches/sampo`
/// - Windows: `%LOCALAPPDATA%\sampo`
fn cache_file_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("sampo").join("version_check"))
}

/// Reads the cached version info if it exists and is still valid.
fn read_cache(ttl_secs: u64) -> Option<String> {
    let path = cache_file_path()?;
    let content = fs::read_to_string(&path).ok()?;
    let entry = CacheEntry::parse(&content)?;

    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();

    if now.saturating_sub(entry.timestamp) < ttl_secs {
        Some(entry.version)
    } else {
        None
    }
}

/// Writes the latest version to the cache file.
fn write_cache(version: &str) -> io::Result<()> {
    let Some(path) = cache_file_path() else {
        return Ok(());
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let entry = CacheEntry {
        version: version.to_string(),
        timestamp,
    };

    let mut file = fs::File::create(&path)?;
    file.write_all(entry.serialize().as_bytes())?;
    Ok(())
}

/// Fetches the latest version from crates.io.
///
/// Returns `None` on network errors, timeouts, or parse failures—these are
/// silently ignored since version checking is best-effort.
fn fetch_latest_version() -> Option<String> {
    let url = "https://crates.io/api/v1/crates/sampo";

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .user_agent(format!("sampo/{}", CURRENT_VERSION))
        .build()
        .ok()?;

    let response = client.get(url).send().ok()?;

    if !response.status().is_success() {
        return None;
    }

    let json: serde_json::Value = response.json().ok()?;
    let version = json
        .get("crate")?
        .get("max_stable_version")?
        .as_str()?
        .to_string();

    Some(version)
}

/// Compares two version strings and returns true if `latest` is greater than `current`.
fn is_newer(current: &str, latest: &str) -> bool {
    let Ok(current_ver) = Version::parse(current) else {
        return false;
    };
    let Ok(latest_ver) = Version::parse(latest) else {
        return false;
    };
    latest_ver > current_ver
}

/// Performs a version check, using the cache when available.
///
/// This function is designed to fail silently—any network or parse errors
/// result in `VersionCheckResult::Skipped` rather than propagating errors.
pub fn check_for_updates() -> VersionCheckResult {
    let current = CURRENT_VERSION;

    // First, try to use cached version info
    if let Some(cached_version) = read_cache(CACHE_TTL_SECS) {
        if is_newer(current, &cached_version) {
            return VersionCheckResult::UpdateAvailable {
                current: current.to_string(),
                latest: cached_version,
            };
        }
        return VersionCheckResult::UpToDate;
    }

    // Cache is stale or missing—fetch from crates.io
    let Some(latest) = fetch_latest_version() else {
        return VersionCheckResult::Skipped;
    };

    // Update the cache (ignore write errors)
    let _ = write_cache(&latest);

    if is_newer(current, &latest) {
        VersionCheckResult::UpdateAvailable {
            current: current.to_string(),
            latest,
        }
    } else {
        VersionCheckResult::UpToDate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_entry_roundtrip() {
        let entry = CacheEntry {
            version: "1.2.3".to_string(),
            timestamp: 1234567890,
        };
        let serialized = entry.serialize();
        let parsed = CacheEntry::parse(&serialized).expect("should parse");
        assert_eq!(parsed.version, "1.2.3");
        assert_eq!(parsed.timestamp, 1234567890);
    }

    #[test]
    fn cache_entry_parse_rejects_empty() {
        assert!(CacheEntry::parse("").is_none());
        assert!(CacheEntry::parse("\n123").is_none());
        assert!(CacheEntry::parse("1.0.0\n").is_none());
        assert!(CacheEntry::parse("1.0.0\nnot_a_number").is_none());
    }

    #[test]
    fn is_newer_compares_versions_correctly() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.0.0", "1.1.0"));
        assert!(is_newer("1.0.0", "2.0.0"));
        assert!(is_newer("0.13.0", "0.14.0"));
        assert!(is_newer("0.13.0", "1.0.0"));

        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("2.0.0", "1.0.0"));

        // Pre-release handling
        assert!(is_newer("1.0.0-alpha", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0-alpha"));
    }

    #[test]
    fn is_newer_handles_invalid_versions() {
        assert!(!is_newer("invalid", "1.0.0"));
        assert!(!is_newer("1.0.0", "invalid"));
        assert!(!is_newer("", "1.0.0"));
    }

    #[test]
    fn current_version_is_valid_semver() {
        let parsed = Version::parse(CURRENT_VERSION);
        assert!(parsed.is_ok(), "CURRENT_VERSION should be valid semver");
    }
}
