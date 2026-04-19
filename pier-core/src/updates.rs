//! Version update check against GitHub Releases.
//!
//! The settings dialog exposes a "Check for updates" button; this
//! module implements the blocking HTTP call and version comparison
//! that backs it. The UI side wraps the call in a background task so
//! the UI thread never blocks on the network.
//!
//! ## Design
//!
//! - Endpoint: `/repos/{owner}/{repo}/releases/latest`. Drafts and
//!   prereleases are skipped by GitHub's `/latest` filter, so the
//!   user never sees a half-published build.
//! - Version comparison is a tiny numeric `x.y.z` comparator — no
//!   `semver` crate dep. Suffixes like `-beta.1` are stripped on each
//!   segment before parsing, so `0.2.0-rc1` compares equal to `0.2.0`.
//!   That's a deliberate conservative choice: if you want prerelease
//!   discrimination, ship a real semver library at that point.
//! - We **don't** download or install the new version — the UI just
//!   opens the release page in the user's default browser. Auto-
//!   update requires code-signing, notarization on macOS, and
//!   per-platform update infrastructure (Sparkle / MSIX / AppImage
//!   update hooks) none of which are wired up yet.
//!
//! The repo coordinates are hard-coded rather than read from
//! `Cargo.toml`'s `repository` field — doing that at runtime would
//! need `cargo_metadata`, and we only ever ship from one place.
//!
//! ## Errors
//!
//! `check_latest_release` returns `Err` on network failure, non-2xx
//! HTTP status, malformed JSON, or a tag name that doesn't parse as a
//! version. The UI displays the error message verbatim as a caption.

use std::cmp::Ordering;
use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;

/// GitHub coordinates of the official Pier-X repository.
const REPO_OWNER: &str = "chenqi92";
const REPO_NAME: &str = "Pier-X";

/// Sent as the `User-Agent` header on every GitHub API request.
/// GitHub rejects API calls with no UA; embedding the crate version
/// also gives their abuse-detection a useful signal about which
/// client is polling.
const USER_AGENT: &str = concat!("Pier-X/", env!("CARGO_PKG_VERSION"));

/// Hard ceiling on the HTTP round-trip. Ten seconds is comfortable on
/// a healthy network and short enough that the UI can show an
/// error and let the user retry rather than appearing frozen.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Shape of the piece of GitHub's release JSON we care about.
/// The full payload is huge (assets, tarball URLs, author info, etc.);
/// by listing only these fields `serde_json` skips the rest cheaply.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
}

/// Outcome of a successful `check_latest_release` call.
#[derive(Debug, Clone)]
pub struct UpdateCheckOutcome {
    /// Version of the running binary (as passed in by the caller).
    pub current_version: String,
    /// `tag_name` from GitHub with any leading `v` stripped.
    pub latest_version: String,
    /// Human-readable release title (`name`, falls back to tag).
    pub release_name: String,
    /// Full release page URL — suitable for `open-in-browser`.
    pub release_url: String,
    /// Markdown release notes from the GitHub release body.
    pub release_notes: Option<String>,
    /// RFC-3339 timestamp of the release's publication.
    pub published_at: Option<String>,
    /// `true` iff `latest_version > current_version`.
    pub is_newer: bool,
}

/// Anything that can go wrong during a single `check_latest_release`.
#[derive(Debug, Error)]
pub enum UpdateError {
    /// Transport failure — DNS, TLS, connection reset, timeout.
    #[error("network error: {0}")]
    Http(String),
    /// GitHub returned a non-2xx status (most commonly 404 when the
    /// repo has no releases yet, or 403 for rate-limit / abuse).
    #[error("GitHub returned HTTP {status}")]
    HttpStatus {
        /// HTTP status code as received from GitHub.
        status: u16,
    },
    /// Response body didn't match our `GithubRelease` shape.
    #[error("unexpected response: {0}")]
    Json(String),
    /// `tag_name` was empty or otherwise couldn't be cleaned into a
    /// version string.
    #[error("could not parse version tag {tag:?}")]
    BadVersion {
        /// The raw `tag_name` value that failed parsing.
        tag: String,
    },
}

impl From<ureq::Error> for UpdateError {
    fn from(err: ureq::Error) -> Self {
        match err {
            ureq::Error::Status(status, _) => UpdateError::HttpStatus { status },
            other => UpdateError::Http(other.to_string()),
        }
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(err: std::io::Error) -> Self {
        UpdateError::Http(err.to_string())
    }
}

/// Query the GitHub Releases API for the latest Pier-X release and
/// compare it to `current_version` (typically `env!("CARGO_PKG_VERSION")`
/// passed from the `pier-ui-gpui` crate).
///
/// Blocks the current thread for up to `REQUEST_TIMEOUT`. Intended to
/// be called from a background executor, never the UI thread.
pub fn check_latest_release(current_version: &str) -> Result<UpdateCheckOutcome, UpdateError> {
    let url = format!("https://api.github.com/repos/{REPO_OWNER}/{REPO_NAME}/releases/latest");
    let release: GithubRelease = ureq::get(&url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .timeout(REQUEST_TIMEOUT)
        .call()?
        .into_json()
        .map_err(|e| UpdateError::Json(e.to_string()))?;

    let latest = release.tag_name.trim_start_matches('v').trim().to_string();
    if latest.is_empty() {
        return Err(UpdateError::BadVersion {
            tag: release.tag_name.clone(),
        });
    }
    let is_newer = compare_versions(current_version, &latest) == Ordering::Less;
    let release_name = release
        .name
        .clone()
        .unwrap_or_else(|| release.tag_name.clone());

    Ok(UpdateCheckOutcome {
        current_version: current_version.to_string(),
        latest_version: latest,
        release_name,
        release_url: release.html_url,
        release_notes: release.body,
        published_at: release.published_at,
        is_newer,
    })
}

/// Compare two `x.y.z`-ish version strings by numeric segment.
///
/// Suffixes on each segment (e.g. the `-rc1` in `0.2.0-rc1`) are
/// stripped before parsing, so prerelease tags collapse to their base
/// version. Missing segments are treated as `0`, so `0.2` compares
/// equal to `0.2.0`. If *both* inputs fail to produce any numeric
/// segment, they're considered equal.
fn compare_versions(a: &str, b: &str) -> Ordering {
    fn segments(raw: &str) -> Vec<u32> {
        raw.split('.')
            .map(|part| {
                let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<u32>().unwrap_or(0)
            })
            .collect()
    }
    let mut left = segments(a);
    let mut right = segments(b);
    let len = left.len().max(right.len());
    left.resize(len, 0);
    right.resize(len, 0);
    left.cmp(&right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_basic() {
        assert_eq!(compare_versions("0.1.0", "0.1.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.1.0", "0.2.0"), Ordering::Less);
        assert_eq!(compare_versions("0.2.0", "0.1.9"), Ordering::Greater);
    }

    #[test]
    fn compare_strips_prerelease_suffixes() {
        assert_eq!(compare_versions("0.2.0-rc1", "0.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.0-alpha", "0.9.0"), Ordering::Greater);
    }

    #[test]
    fn compare_treats_missing_segment_as_zero() {
        assert_eq!(compare_versions("0.2", "0.2.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.2", "0.2.1"), Ordering::Less);
    }
}
