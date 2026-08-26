//! `hyprlayer ai versions` — the releases that carry an assets bundle for
//! both Claude and Codex bundles, and which of them is installed or pinned. The
//! list the desktop drives its rollback picker from, and the input to
//! `ai reinstall --version`.
//!
//! **On demand only, never on startup.** `{api}/releases` is a REST call
//! against GitHub's 60 requests/hour unauthenticated bucket. The on-disk
//! cache below keeps a desktop that polls `--json` to one request an hour.

use anyhow::{Context, Result};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::agents;
use crate::cli::AiVersionsArgs;
use crate::config::HyprlayerConfig;
use crate::http;
use crate::telemetry::unix_now;

/// Same budget the release fetch in `agents::fetch_asset_into` gets.
const RELEASES_TIMEOUT: Duration = Duration::from_secs(15);

/// A releases page runs a few hundred KB at `per_page=100`. Matches the cap
/// every other GitHub API read in the CLI uses.
const MAX_RELEASES_RESPONSE_BYTES: u64 = 1024 * 1024;

/// How long a cached listing stands in for a fetch. Releases appear a few
/// times a month, so an hour of staleness costs nothing and is what keeps
/// repeated `ai versions` calls off the rate limit.
const CACHE_TTL_SECS: u64 = 60 * 60;

const CACHE_DIR: &str = "cache";

/// One release that actually carries a bundle for the harness in question.
/// Distilled from the (much larger) API response before caching: the raw
/// page is mostly uploader metadata and binary asset entries we never read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRelease {
    /// Release version without the tag's leading `v`, as the asset names
    /// and `agentsPinnedVersion` spell it.
    pub version: String,
    /// RFC3339 publication timestamp, as GitHub reports it. `None` for a
    /// release that has never been published.
    pub published_at: Option<String>,
}

pub fn versions(args: AiVersionsArgs) -> Result<()> {
    let AiVersionsArgs {
        json,
        limit,
        config,
    } = args;

    let hyprlayer_config = config.load_if_exists()?.unwrap_or_default();
    let releases = bundle_releases(limit)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&versions_json(&hyprlayer_config, &releases))?
        );
    } else {
        print_versions(&hyprlayer_config, &releases, limit);
    }

    Ok(())
}

/// The releases carrying the atomic Claude + Codex bundle pair, from the cache when it is
/// fresh and from GitHub otherwise.
///
/// Every cache failure — unreadable, unparseable, unwritable — degrades to
/// a plain fetch rather than an error. The cache is a rate-limit courtesy,
/// not a source of truth.
fn bundle_releases(limit: u32) -> Result<Vec<BundleRelease>> {
    let now = unix_now();
    let path = cache_path("claude-codex").ok();

    if let Some(path) = &path
        && let Some(cached) = cached_releases(path, limit, now)
    {
        return Ok(cached);
    }

    let body = fetch_release_page(limit)?;
    let releases = releases_with_bundle_pair(&body)?;

    if let Some(path) = &path {
        write_cache(path, &ReleaseCache::new(now, limit, releases.clone()));
    }

    Ok(releases)
}

fn fetch_release_page(limit: u32) -> Result<String> {
    let url = format!(
        "{}/releases?per_page={limit}",
        agents::github_api_repo_url()
    );
    http::get_text_capped(&url, RELEASES_TIMEOUT, MAX_RELEASES_RESPONSE_BYTES)
        .map_err(|e| anyhow::anyhow!("Unable to fetch the release list from GitHub: {e}"))
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GitHubReleaseAsset>,
}

#[derive(Deserialize)]
struct GitHubReleaseAsset {
    name: String,
}

/// GitHub's error-object shape, which a rate-limited or otherwise failed
/// request returns in place of the array.
#[derive(Deserialize)]
struct GitHubErrorBody {
    message: Option<String>,
}

/// Keep only releases that carry both version-matched Claude and Codex
/// assets, exactly the set `ai reinstall --version` can install atomically.
/// Drafts are skipped because their assets are not downloadable.
///
/// Pure — no I/O — so the filtering is unit-testable against a real
/// response body.
fn releases_with_bundle_pair(body: &str) -> Result<Vec<BundleRelease>> {
    let releases: Vec<GitHubRelease> = serde_json::from_str(body).map_err(|e| {
        classify_release_list_error(body)
            .unwrap_or_else(|| anyhow::anyhow!("Failed to parse the GitHub release list: {e}"))
    })?;

    Ok(releases
        .into_iter()
        .filter_map(|release| {
            if release.draft {
                return None;
            }
            let version = release.tag_name.trim_start_matches('v').to_string();
            let claude = agents::asset_name("claude", &version);
            let codex = agents::asset_name("codex", &version);
            if !release.assets.iter().any(|a| a.name == claude)
                || !release.assets.iter().any(|a| a.name == codex)
            {
                return None;
            }
            Some(BundleRelease {
                version,
                published_at: release.published_at,
            })
        })
        .collect())
}

/// Turn GitHub's error object into a user-facing error, or `None` when the
/// body is not one (a genuinely malformed array, say).
///
/// The rate-limit branch points at the cache rather than at `ai reinstall`:
/// the answer is to wait.
fn classify_release_list_error(body: &str) -> Option<anyhow::Error> {
    let message = serde_json::from_str::<GitHubErrorBody>(body)
        .ok()?
        .message?;
    if message.contains("rate limit") {
        return Some(anyhow::anyhow!(
            "GitHub API rate limit exceeded (60 requests/hour for unauthenticated \
             clients, shared across your whole network). Retry in an hour; \
             'hyprlayer ai versions' caches its results for that long, so the \
             listing itself is not what exhausted it."
        ));
    }
    Some(anyhow::anyhow!(
        "GitHub would not list the releases ({message})"
    ))
}

/// A distilled listing plus when it was fetched and for which `limit`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseCache {
    fetched_at: u64,
    /// The `per_page` the listing was fetched with. A cache taken at one
    /// limit cannot answer for another: a smaller one would over-report,
    /// and a larger one would hide releases the caller asked to see.
    limit: u32,
    releases: Vec<BundleRelease>,
}

impl ReleaseCache {
    fn new(fetched_at: u64, limit: u32, releases: Vec<BundleRelease>) -> Self {
        Self {
            fetched_at,
            limit,
            releases,
        }
    }
}

fn cache_path(harness: &str) -> Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    Ok(base
        .join("hyprlayer")
        .join(CACHE_DIR)
        .join(format!("releases-{harness}.json")))
}

/// The cached listing, if there is one, it was taken for this `limit`, and
/// it is still inside the TTL. `now` is a parameter so expiry is testable
/// without waiting an hour.
///
/// A `fetched_at` in the future (a clock that went backwards) reads as
/// expired rather than as permanently fresh — `saturating_sub` makes the
/// age `0`, so the comparison below is the one that decides, and a stale
/// entry is refetched on the next run either way.
fn cached_releases(path: &Path, limit: u32, now: u64) -> Option<Vec<BundleRelease>> {
    let cache: ReleaseCache = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    if cache.limit != limit || now.saturating_sub(cache.fetched_at) >= CACHE_TTL_SECS {
        return None;
    }
    Some(cache.releases)
}

/// Best-effort: a cache we cannot write just means the next call fetches
/// again, which is not worth failing an otherwise-successful listing over.
fn write_cache(path: &Path, cache: &ReleaseCache) {
    if let Err(e) = write_cache_inner(path, cache) {
        eprintln!(
            "warning: could not cache the release list at {}: {e}",
            path.display()
        );
    }
}

fn write_cache_inner(path: &Path, cache: &ReleaseCache) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let body = serde_json::to_string(cache).context("Failed to serialize the release cache")?;
    fs::write(path, body).with_context(|| format!("Failed to write {}", path.display()))
}

/// Whether a listed version is the one installed, and whether it is the pin.
/// Both, neither, or either: a pin that has not been installed yet is a real
/// state (the install failed), and so is an installed version with no pin.
fn marks(config: &HyprlayerConfig, version: &str) -> (bool, bool) {
    (
        config.agents_installed_version.as_deref() == Some(version),
        config.agents_pinned_version.as_deref() == Some(version),
    )
}

/// The `--json` payload. The three top-level version fields mirror
/// `ai status --json`, so a consumer reading one does not have to call the
/// other to learn what is installed.
fn versions_json(config: &HyprlayerConfig, releases: &[BundleRelease]) -> serde_json::Value {
    let versions: Vec<serde_json::Value> = releases
        .iter()
        .map(|release| {
            let (current, pinned) = marks(config, &release.version);
            serde_json::json!({
                "version": release.version,
                "publishedAt": release.published_at,
                "current": current,
                "pinned": pinned,
            })
        })
        .collect();

    serde_json::json!({
        "harness": "claude-codex",
        "assetsVersion": config.agents_installed_version,
        "pinnedVersion": config.agents_pinned_version,
        "binaryVersion": env!("CARGO_PKG_VERSION"),
        "versions": versions,
    })
}

/// Just the date from an RFC3339 timestamp — the time of day says nothing
/// useful about a release, and dropping it keeps the column narrow.
fn published_date(release: &BundleRelease) -> &str {
    release
        .published_at
        .as_deref()
        .and_then(|ts| ts.split('T').next())
        .unwrap_or("unpublished")
}

fn print_versions(config: &HyprlayerConfig, releases: &[BundleRelease], limit: u32) {
    println!();
    println!("  {} bundle versions:", "Claude + Codex".cyan());
    println!();

    if releases.is_empty() {
        println!(
            "  {}",
            format!("None of the {limit} most recent releases carry one.").bright_black()
        );
        return;
    }

    for release in releases {
        let (current, pinned) = marks(config, &release.version);
        let marker = match (current, pinned) {
            (true, true) => " (installed, pinned)",
            (true, false) => " (installed)",
            (false, true) => " (pinned)",
            (false, false) => "",
        };
        println!(
            "  {} {}{}",
            format!("{:<16}", release.version).cyan(),
            published_date(release).bright_black(),
            marker.green()
        );
    }

    println!();
    println!(
        "  {}",
        "Pin one with 'hyprlayer ai reinstall --version <version>'.".bright_black()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HyprlayerConfig;

    /// Shaped like a real `GET /releases?per_page=N` page: `v1.6.0-rc.1`
    /// carries all three bundles, `v1.5.9` predates them and carries only
    /// binaries, and the draft carries a claude bundle that is not yet
    /// downloadable. Trimmed to the fields the parser reads.
    const RELEASES_PAGE: &str = r#"[
        {
            "tag_name": "v1.6.0-rc.1",
            "name": "v1.6.0-rc.1",
            "draft": false,
            "prerelease": true,
            "published_at": "2026-08-24T01:06:31Z",
            "assets": [
                { "name": "hyprlayer-x86_64-unknown-linux-gnu.tar.gz" },
                { "name": "hyprlayer-assets-claude-1.6.0-rc.1.tar.gz" },
                { "name": "hyprlayer-assets-codex-1.6.0-rc.1.tar.gz" }
            ]
        },
        {
            "tag_name": "v1.5.9",
            "name": "v1.5.9",
            "draft": false,
            "prerelease": false,
            "published_at": "2026-08-20T04:04:09Z",
            "assets": [
                { "name": "hyprlayer-x86_64-unknown-linux-gnu.tar.gz" },
                { "name": "hyprlayer-aarch64-apple-darwin.tar.gz" }
            ]
        },
        {
            "tag_name": "v1.6.0-rc.2",
            "name": "v1.6.0-rc.2",
            "draft": true,
            "prerelease": true,
            "published_at": null,
            "assets": [
                { "name": "hyprlayer-assets-claude-1.6.0-rc.2.tar.gz" }
            ]
        }
    ]"#;

    fn versions_in(body: &str) -> Vec<String> {
        releases_with_bundle_pair(body)
            .unwrap()
            .into_iter()
            .map(|r| r.version)
            .collect()
    }

    #[test]
    fn versions_keeps_only_releases_carrying_both_assets() {
        assert_eq!(versions_in(RELEASES_PAGE), vec!["1.6.0-rc.1"]);
    }

    #[test]
    fn versions_records_the_publication_date() {
        let releases = releases_with_bundle_pair(RELEASES_PAGE).unwrap();
        assert_eq!(
            releases[0].published_at.as_deref(),
            Some("2026-08-24T01:06:31Z")
        );
        assert_eq!(published_date(&releases[0]), "2026-08-24");
    }

    #[test]
    fn versions_skips_draft_releases() {
        // The draft is the only release carrying a 1.6.0-rc.2 bundle.
        assert!(!versions_in(RELEASES_PAGE).contains(&"1.6.0-rc.2".to_string()));
    }

    #[test]
    fn versions_require_the_atomic_pair() {
        let body = r#"[
            {
                "tag_name": "v1.6.0",
                "draft": false,
                "published_at": "2026-09-01T00:00:00Z",
                "assets": [{ "name": "hyprlayer-assets-claude-1.6.0.tar.gz" }]
            }
        ]"#;
        assert!(versions_in(body).is_empty());
    }

    #[test]
    fn versions_ignores_an_asset_from_a_different_version() {
        // A bundle whose name doesn't match its own release's version is
        // not installable at that version, so it must not be listed.
        let body = r#"[
            {
                "tag_name": "v1.6.1",
                "draft": false,
                "published_at": "2026-09-02T00:00:00Z",
                "assets": [{ "name": "hyprlayer-assets-claude-1.6.0.tar.gz" }]
            }
        ]"#;
        assert!(versions_in(body).is_empty());
    }

    #[test]
    fn versions_reports_a_rate_limited_response() {
        let body = r#"{ "message": "API rate limit exceeded for 203.0.113.7.",
                        "documentation_url": "https://docs.github.com/rest" }"#;
        let err = releases_with_bundle_pair(body).unwrap_err();
        assert!(
            err.to_string().contains("rate limit"),
            "expected rate-limit guidance, got: {err}"
        );
    }

    #[test]
    fn versions_reports_other_github_errors() {
        let body = r#"{ "message": "Not Found" }"#;
        let err = releases_with_bundle_pair(body).unwrap_err();
        assert!(err.to_string().contains("Not Found"), "got: {err}");
    }

    #[test]
    fn versions_reject_a_body_that_is_not_json() {
        assert!(releases_with_bundle_pair("<html>502</html>").is_err());
    }

    fn write_cache_fixture(dir: &Path, fetched_at: u64, limit: u32) -> PathBuf {
        let path = dir.join("releases-claude.json");
        write_cache(
            &path,
            &ReleaseCache::new(
                fetched_at,
                limit,
                vec![BundleRelease {
                    version: "1.6.0-rc.1".to_string(),
                    published_at: Some("2026-08-24T01:06:31Z".to_string()),
                }],
            ),
        );
        path
    }

    #[test]
    fn versions_cache_is_reused_inside_the_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_cache_fixture(dir.path(), 1_000_000, 10);
        let cached = cached_releases(&path, 10, 1_000_000 + CACHE_TTL_SECS - 1).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].version, "1.6.0-rc.1");
    }

    #[test]
    fn versions_cache_expires_after_the_ttl() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_cache_fixture(dir.path(), 1_000_000, 10);
        assert!(cached_releases(&path, 10, 1_000_000 + CACHE_TTL_SECS).is_none());
    }

    #[test]
    fn versions_cache_is_not_reused_for_a_different_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_cache_fixture(dir.path(), 1_000_000, 10);
        assert!(cached_releases(&path, 25, 1_000_000).is_none());
    }

    #[test]
    fn versions_cache_miss_on_a_missing_or_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cached_releases(&dir.path().join("absent.json"), 10, 0).is_none());
        let corrupt = dir.path().join("releases-claude.json");
        fs::write(&corrupt, "{ not json").unwrap();
        assert!(cached_releases(&corrupt, 10, 0).is_none());
    }

    fn config_with(installed: Option<&str>, pinned: Option<&str>) -> HyprlayerConfig {
        HyprlayerConfig {
            agents_installed_version: installed.map(str::to_string),
            agents_pinned_version: pinned.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn versions_json_marks_the_installed_and_pinned_entries() {
        let releases = releases_with_bundle_pair(RELEASES_PAGE).unwrap();
        let config = config_with(Some("1.6.0-rc.1"), Some("1.6.0-rc.1"));

        let value = versions_json(&config, &releases);

        assert_eq!(value["harness"], "claude-codex");
        assert_eq!(value["assetsVersion"], "1.6.0-rc.1");
        assert_eq!(value["pinnedVersion"], "1.6.0-rc.1");
        assert_eq!(value["binaryVersion"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["versions"][0]["version"], "1.6.0-rc.1");
        assert_eq!(value["versions"][0]["publishedAt"], "2026-08-24T01:06:31Z");
        assert_eq!(value["versions"][0]["current"], true);
        assert_eq!(value["versions"][0]["pinned"], true);
    }

    #[test]
    fn versions_json_reports_an_unpinned_install_as_current_only() {
        let releases = releases_with_bundle_pair(RELEASES_PAGE).unwrap();
        let config = config_with(Some("1.6.0-rc.1"), None);

        let value = versions_json(&config, &releases);

        assert_eq!(value["pinnedVersion"], serde_json::Value::Null);
        assert_eq!(value["versions"][0]["current"], true);
        assert_eq!(value["versions"][0]["pinned"], false);
    }

    #[test]
    fn versions_json_marks_a_pin_that_is_not_installed_yet() {
        let releases = releases_with_bundle_pair(RELEASES_PAGE).unwrap();
        let config = config_with(Some("1.5.9"), Some("1.6.0-rc.1"));

        let value = versions_json(&config, &releases);

        assert_eq!(value["versions"][0]["current"], false);
        assert_eq!(value["versions"][0]["pinned"], true);
    }
}
