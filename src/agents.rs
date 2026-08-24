pub(crate) mod archive;
pub(crate) mod manifest;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{MAIN_SEPARATOR_STR as SEP, Path, PathBuf};
use std::time::Duration;

use crate::http;
use crate::integrity;

pub(crate) const REPO: &str = "BrightBlock/hyprlayer-cli";
const BRANCH: &str = "master";

/// GitHub Contents/commits API responses for a single directory listing run
/// a few KB. 1 MiB is two orders of magnitude of headroom and stops a
/// hostile or misconfigured source from buffering an unbounded `String`.
/// Mirrors `MAX_RELEASE_API_BYTES` in `src/commands/self_update.rs`.
const MAX_API_RESPONSE_BYTES: u64 = 1024 * 1024;

pub(crate) fn github_api_repo_url() -> String {
    format!("https://api.github.com/repos/{REPO}")
}

pub(crate) fn github_release_download_base() -> String {
    format!("https://github.com/{REPO}/releases/download")
}

/// The bundle version an install should resolve to. Today that is always the
/// running binary's own version, so a 1.6.0 CLI installs the 1.6.0 bundle and
/// upgrading the binary is what moves the skills. The explicit pin that can
/// override this is a later phase.
fn desired_assets_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Release-asset file name for one harness at one version, as
/// `scripts/build-asset-bundles.sh` emits it and `release.yml` attaches it.
fn asset_name(harness: &str, version: &str) -> String {
    format!("hyprlayer-assets-{harness}-{version}.tar.gz")
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentTool {
    Claude,
    Copilot,
    OpenCode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OpenCodeProvider {
    GithubCopilot,
    Anthropic,
    Abacus,
}

impl fmt::Display for OpenCodeProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GithubCopilot => write!(f, "GitHub Copilot"),
            Self::Anthropic => write!(f, "Anthropic"),
            Self::Abacus => write!(f, "Abacus"),
        }
    }
}

impl OpenCodeProvider {
    pub const ALL: &[OpenCodeProvider] = &[
        OpenCodeProvider::GithubCopilot,
        OpenCodeProvider::Anthropic,
        OpenCodeProvider::Abacus,
    ];

    pub fn default_sonnet_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/claude-sonnet-5",
            Self::Anthropic => "anthropic/claude-sonnet-5",
            Self::Abacus => "abacus/claude-sonnet-5",
        }
    }

    pub fn default_opus_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/claude-opus-5",
            Self::Anthropic => "anthropic/claude-opus-5",
            Self::Abacus => "abacus/claude-opus-5",
        }
    }

    /// Abacus and GitHub Copilot route to their highest-reasoning codex
    /// variant for a true cross-model second opinion; Anthropic stays on
    /// claude-opus-5 because the Anthropic API is Claude-only.
    ///
    /// Model ids are the ones opencode resolves through the models.dev
    /// registry, so a rename upstream shows up here as an unresolvable
    /// model rather than a silent downgrade — `gpt-5-codex` was dropped
    /// from Copilot's catalog, which is why it is `gpt-5.3-codex` now.
    pub fn default_adversarial_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/gpt-5.3-codex",
            Self::Anthropic => "anthropic/claude-opus-5",
            Self::Abacus => "abacus/gpt-5.3-codex-xhigh",
        }
    }

    pub fn provider_prefix(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot",
            Self::Anthropic => "anthropic",
            Self::Abacus => "abacus",
        }
    }
}

impl fmt::Display for AgentTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claude => write!(f, "Claude Code"),
            Self::Copilot => write!(f, "GitHub Copilot"),
            Self::OpenCode => write!(f, "OpenCode"),
        }
    }
}

impl AgentTool {
    pub const ALL: &[AgentTool] = &[AgentTool::Claude, AgentTool::Copilot, AgentTool::OpenCode];

    pub(crate) fn repo_dir(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Copilot => "copilot",
            Self::OpenCode => "opencode",
        }
    }

    pub(crate) fn dest_dir(&self) -> Result<PathBuf> {
        match self {
            Self::Claude => {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
                Ok(home.join(".claude"))
            }
            Self::Copilot => {
                let config = dirs::config_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
                Ok(config.join("Code").join("User"))
            }
            Self::OpenCode => {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
                Ok(home.join(".config").join("opencode"))
            }
        }
    }

    pub fn dest_display(&self) -> String {
        match self {
            Self::Claude => format!("~{SEP}.claude{SEP}"),
            #[cfg(target_os = "linux")]
            Self::Copilot => format!("~{SEP}.config{SEP}Code{SEP}User{SEP}"),
            #[cfg(target_os = "macos")]
            Self::Copilot => {
                format!("~{SEP}Library{SEP}Application Support{SEP}Code{SEP}User{SEP}")
            }
            #[cfg(target_os = "windows")]
            Self::Copilot => format!("%APPDATA%{SEP}Code{SEP}User{SEP}"),
            Self::OpenCode => format!("~{SEP}.config{SEP}opencode{SEP}"),
        }
    }

    pub fn is_installed(&self) -> bool {
        let Ok(dest) = self.dest_dir() else {
            return false;
        };
        self.is_installed_at(&dest)
    }

    /// Looser variant: does any prior install exist at `dest_dir`, even if
    /// it predates the current sentinel-file set? Used by the auto-reinstall
    /// gate so that exactly the stale installs that need refreshing get
    /// refreshed. `is_installed` would return false for them and the auto
    /// path would never run.
    pub fn has_existing_install(&self) -> bool {
        let Ok(dest) = self.dest_dir() else {
            return false;
        };
        self.has_existing_install_at(&dest)
    }

    fn has_existing_install_at(&self, dest: &Path) -> bool {
        // Per-tool structural directories that have been part of every
        // shipped bundle. If both exist, *something* was installed here
        // by a previous `hyprlayer ai configure`.
        let (a, b) = match self {
            Self::Claude => ("skills", "agents"),
            Self::OpenCode => ("commands", "agents"),
            Self::Copilot => ("prompts", "agents"),
        };
        dest.join(a).is_dir() && dest.join(b).is_dir()
    }

    /// Test-friendly variant of `is_installed` that takes an explicit destination path.
    ///
    /// Checks for sentinel files unique to the current bundle of
    /// commands/skills/agents. An older install with the right top-level
    /// directories but missing newly added files reports not-installed, so
    /// `configure --no-force` re-runs and provisions the new bundle. Bump
    /// these whenever we ship a top-level file existing users should pick up.
    fn is_installed_at(&self, dest: &Path) -> bool {
        match self {
            // This sentinel doubles as the staged-download completeness
            // gate (`is_installed_at` failing here after a staged fetch is
            // a hard `bail!`, not a soft warning). It may therefore only
            // ever name long-lived, load-bearing files that ship on every
            // release — never a file from a branch not yet on `master`,
            // or reverting that file turns a benign rollback into every
            // user's next `ai configure`/`ai reinstall` failing outright.
            Self::Claude => {
                dest.join("skills/code_review/SKILL.md").is_file()
                    && dest.join("agents/codebase-locator.md").is_file()
            }
            Self::OpenCode => {
                dest.join("commands/code_review.md").is_file()
                    && dest.join("agents/codebase-locator.md").is_file()
            }
            Self::Copilot => {
                dest.join("prompts/code_review.prompt.md").is_file()
                    && dest.join("agents/codebase-locator.agent.md").is_file()
            }
        }
    }

    /// Print status information for this agent tool.
    /// OpenCode includes provider and model details from config.
    pub fn print_status(&self, config: &crate::config::AiConfig) {
        use colored::Colorize;

        println!("  AI Tool: {}", self.to_string().cyan());

        let status = if self.is_installed() {
            "installed".green()
        } else {
            "not installed".red()
        };
        println!("  Status: {}", status);
        println!("  Location: {}", self.dest_display().cyan());

        match self {
            Self::OpenCode => {
                println!();
                println!("  {}", "OpenCode Settings:".yellow());
                println!(
                    "    Provider: {}",
                    config
                        .opencode_provider
                        .as_ref()
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "not set".to_string())
                        .cyan()
                );
                println!(
                    "    Sonnet Model: {}",
                    config
                        .opencode_sonnet_model
                        .as_deref()
                        .unwrap_or("not set")
                        .cyan()
                );
                println!(
                    "    Opus Model: {}",
                    config
                        .opencode_opus_model
                        .as_deref()
                        .unwrap_or("not set")
                        .cyan()
                );
            }
            Self::Claude | Self::Copilot => {}
        }
    }

    pub fn status_json(&self, config: &crate::config::AiConfig) -> serde_json::Value {
        match self {
            Self::OpenCode => serde_json::json!({
                "agentTool": self.to_string(),
                "installed": self.is_installed(),
                "location": self.dest_display(),
                "opencodeProvider": config.opencode_provider.as_ref().map(|p| p.to_string()),
                "opencodeSonnetModel": config.opencode_sonnet_model.clone(),
                "opencodeOpusModel": config.opencode_opus_model.clone(),
            }),
            Self::Claude | Self::Copilot => serde_json::json!({
                "agentTool": self.to_string(),
                "installed": self.is_installed(),
                "location": self.dest_display(),
            }),
        }
    }

    /// Download agent files from GitHub and install to the destination.
    ///
    /// Downloads into a temporary staging directory first, refuses to
    /// touch `dest` at all unless the staged bundle looks complete (see
    /// `is_installed_at`), and then copies only the files that actually
    /// changed. A mid-download failure — network drop, rate limit,
    /// truncated archive — therefore can never leave `dest` half-written:
    /// either staging fails and `dest` is untouched, or staging succeeds
    /// and the sync is a pure content diff.
    ///
    /// `InstallOutcome::sha` is `Some` when we successfully captured the
    /// repo's `master` HEAD SHA *before* the download (the next 24h
    /// auto-check uses this as the freshness baseline), and `None` when
    /// the ref advertisement was unreachable but the download still
    /// succeeded — the install is still good, but we have no SHA to
    /// cache, so the next auto-check will treat the bundle as stale and
    /// refresh again. We don't fail the whole install when only ref
    /// resolution is unreachable because `hyprlayer ai configure` /
    /// `ai reinstall` must continue to work by falling back to the
    /// `master` branch name.
    pub fn install(
        &self,
        opencode_provider: Option<&OpenCodeProvider>,
        quiet: bool,
    ) -> Result<InstallOutcome> {
        let dest = self.dest_dir()?;

        // Recording a post-download SHA could mask `master`-advances that
        // happen mid-install — next-day's check would then compare against
        // an at-or-newer cache and skip the necessary re-sync.
        let sha = fetch_master_sha().ok();
        let git_ref = sha.as_deref().unwrap_or(BRANCH);

        let staging = tempfile::tempdir().context("Failed to create a staging directory")?;
        let staged = staging.path().join(self.repo_dir());
        self.fetch_into(&staged, git_ref, quiet)?;

        let changed = self.install_staged(&staged, &dest)?;

        if matches!(self, AgentTool::OpenCode)
            && let Some(provider) = opencode_provider
        {
            if !quiet {
                println!("Configuring models for {}...", provider);
            }
            let updated = update_opencode_models(&dest, provider)?;
            if !quiet {
                println!("  {:<60}", format!("Updated {} files", updated));
            }
        }

        Ok(InstallOutcome { sha, changed })
    }

    /// The post-fetch half of `install`: refuse to touch `dest` unless the
    /// staged bundle looks complete, then sync only what differs. Without
    /// the gate, a truncated download could overwrite a good install with a
    /// torn one. Split out of `install` so the rollback guarantee is
    /// testable without a network fetch.
    fn install_staged(&self, staged: &Path, dest: &Path) -> Result<usize> {
        if !self.is_installed_at(staged) {
            anyhow::bail!(
                "Downloaded {} bundle is incomplete — refusing to install. \
                 Run 'hyprlayer ai reinstall' to retry.",
                self
            );
        }

        fs::create_dir_all(dest)?;
        sync_tree(staged, dest)
    }

    /// Populate `staged` with this tool's bundle, trying each source in
    /// turn and keeping the first that works.
    ///
    /// 1. The versioned release asset, verified against the SHA256 digest
    ///    GitHub computed server-side. This is the bundle that matches the
    ///    running binary.
    /// 2. The single-request codeload archive of the frozen legacy tree at
    ///    `git_ref` (zero REST API requests, see
    ///    `archive::fetch_and_extract`).
    /// 3. The old Contents-API walk, in case codeload itself is out.
    ///
    /// The asset step is allowed to fail softly — a dev build with no
    /// matching release, a release predating the bundles, a mid-download
    /// network drop, a digest mismatch — because a legacy-tree install is
    /// still a working install. What it must never do is install an
    /// *unverified* asset, so a release that advertises no digest for the
    /// bundle drops through to the fallback rather than downloading it
    /// anyway.
    ///
    /// Every source writes into `staged`, so the completeness check and
    /// rollback guarantee in `install_staged` cover the fallbacks too: a
    /// rate-limited fallback aborts cleanly instead of leaving a partial
    /// `dest`.
    fn fetch_into(&self, staged: &Path, git_ref: &str, quiet: bool) -> Result<()> {
        if !quiet {
            println!("Downloading {} agent files...", self);
        }

        let version = desired_assets_version();
        let sources: Vec<BundleSource<'_>> = vec![
            (
                format!("the v{version} release asset"),
                Box::new(move |dest: &Path| self.fetch_asset_into(dest, version)),
            ),
            (
                format!("the {BRANCH} repo archive"),
                Box::new(move |dest: &Path| {
                    archive::fetch_and_extract(self.repo_dir(), git_ref, dest)
                }),
            ),
            (
                format!("the GitHub API walk of {BRANCH}"),
                Box::new(move |dest: &Path| {
                    let mut count = 0;
                    download_directory(self.repo_dir(), git_ref, dest, &mut count, quiet)?;
                    Ok(count)
                }),
            ),
        ];

        let (label, count) = fetch_first_available(staged, sources, quiet)?;
        if !quiet {
            println!("  {:<60}", format!("Downloaded {count} files from {label}"));
        }
        Ok(())
    }

    /// Download this tool's `hyprlayer-assets-<harness>-<version>.tar.gz`
    /// from the release tagged `v<version>`, verify it against the release
    /// API's per-asset SHA256 digest, and extract it into `staged`.
    ///
    /// Mirrors `direct_update` in `src/commands/self_update.rs`, which runs
    /// the same fetch-digest-then-verify sequence for the binary itself.
    fn fetch_asset_into(&self, staged: &Path, version: &str) -> Result<usize> {
        let asset = asset_name(self.repo_dir(), version);
        let tag = format!("v{version}");

        let api_url = format!("{}/releases/tags/{tag}", github_api_repo_url());
        let release_body =
            http::get_text_capped(&api_url, Duration::from_secs(15), MAX_API_RESPONSE_BYTES)
                .map_err(|e| anyhow::anyhow!("Unable to fetch release {tag} from GitHub: {e}"))?;
        let expected = asset_digest_from_release(&release_body, &tag, &asset)?;

        let tmp = tempfile::tempdir().context("Failed to create a temp dir for the bundle")?;
        let archive_path = tmp.path().join(&asset);
        let url = format!("{}/{tag}/{asset}", github_release_download_base());
        http::download_file_capped(
            &url,
            &archive_path,
            Duration::from_secs(30),
            Some(archive::MAX_ARCHIVE_BYTES),
        )
        .map_err(|e| anyhow::anyhow!("Failed to download {url}: {e}"))?;

        verify_and_extract_bundle(&archive_path, &asset, &expected, staged)
    }
}

/// One named way to populate a staging directory, used by `fetch_into`.
type BundleSource<'a> = (String, Box<dyn FnOnce(&Path) -> Result<usize> + 'a>);

/// Run `sources` in order and keep the first that succeeds, returning its
/// label and file count.
///
/// `staged` is cleared before each attempt, so a source that fails partway
/// through extraction can't leak files into the bundle the next source
/// produces — the completeness gate in `install_staged` would otherwise be
/// judging a mixture of two downloads.
///
/// The error from the *last* source is what propagates when every source
/// fails, which keeps the Contents-API walk's rate-limit guidance (see
/// `classify_github_error`) as the message the user actually sees.
fn fetch_first_available(
    staged: &Path,
    sources: Vec<BundleSource<'_>>,
    quiet: bool,
) -> Result<(String, usize)> {
    let mut last_error = None;
    for (label, fetch) in sources {
        if staged.exists() {
            fs::remove_dir_all(staged)
                .with_context(|| format!("Failed to clear staging dir {}", staged.display()))?;
        }
        match fetch(staged) {
            Ok(count) => return Ok((label, count)),
            Err(e) => {
                if !quiet {
                    eprintln!("  Fetching {label} failed ({e}); trying the next source.");
                }
                last_error = Some(e);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("No bundle download sources are configured")))
}

/// Pick the SHA256 digest the release advertises for `asset`.
///
/// Pure — no I/O — so the "this release carries no bundle for me" case that
/// triggers the legacy fallback is directly unit-testable. A release with no
/// digest for the asset is an error rather than an unverified download: the
/// caller falls back to the frozen legacy tree, which is an older bundle but
/// not an unverified one.
fn asset_digest_from_release(release_body: &str, tag: &str, asset: &str) -> Result<String> {
    integrity::digests_from_release_json(release_body)
        .remove(asset)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "GitHub release `{tag}` exposes no SHA256 digest for asset `{asset}`. \
                 Refusing to install an unverified bundle."
            )
        })
}

/// Verify a downloaded bundle against `expected` and, only then, extract it
/// into `staged`. Split from the download so the digest-mismatch path is
/// testable without a network fetch.
fn verify_and_extract_bundle(
    archive_path: &Path,
    asset: &str,
    expected: &str,
    staged: &Path,
) -> Result<usize> {
    integrity::verify_sha256(archive_path, expected)
        .with_context(|| format!("Integrity check failed for `{asset}`"))?;
    archive::extract_bundle(archive_path, staged)
}

/// Result of a successful `AgentTool::install`.
#[derive(Debug)]
pub struct InstallOutcome {
    /// The repo's `master` HEAD SHA captured before the download, if ref
    /// resolution succeeded. `None` means the install still succeeded but
    /// there's no fresh SHA to cache.
    pub sha: Option<String>,
    /// Number of files actually written to the destination. `0` means the
    /// staged bundle was byte-identical to what's already installed.
    pub changed: usize,
}

/// Copy every file from `src` into `dest`, creating parent directories as
/// needed, but writing a file only when it's missing from `dest` or its
/// bytes differ from what's already there. Never deletes anything from
/// `dest` that isn't present in `src` — `dest` (e.g. `~/.claude`) holds the
/// user's own files alongside ours, so install stays additive-and-overwrite
/// rather than a mirror. Returns the number of files actually written.
fn sync_tree(src: &Path, dest: &Path) -> Result<usize> {
    let mut changed = 0;
    for path in walk_files(src)? {
        let relative = path
            .strip_prefix(src)
            .expect("walk_files only yields paths under src");
        let dest_path = dest.join(relative);

        let new_bytes =
            fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        let unchanged = fs::read(&dest_path).is_ok_and(|existing| existing == new_bytes);
        if unchanged {
            continue;
        }

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        fs::write(&dest_path, &new_bytes)
            .with_context(|| format!("Failed to write {}", dest_path.display()))?;
        changed += 1;
    }
    Ok(changed)
}

/// Recursively list every regular file under `dir`. Returns an empty list
/// (not an error) when `dir` doesn't exist, so `sync_tree` on an empty
/// staged bundle is a well-defined no-op.
fn walk_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))? {
        let path = entry
            .with_context(|| format!("Failed to read an entry in {}", dir.display()))?
            .path();
        if path.is_dir() {
            out.extend(walk_files(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

/// Ref advertisements run a few KB. 1 MiB is two orders of magnitude of
/// headroom and stops a hostile or misconfigured source from streaming
/// gigabytes into a `String`.
const MAX_REFS_BYTES: u64 = 1024 * 1024;

/// Resolve `master`'s HEAD SHA via git's smart-HTTP ref advertisement.
/// This is the same data `git ls-remote` prints, over plain HTTPS, and
/// costs zero REST API requests — unlike the commits endpoint it
/// replaces, which shares the 60/hr unauthenticated bucket with
/// everything else.
pub(crate) fn fetch_master_sha() -> Result<String> {
    let url = format!("https://github.com/{REPO}.git/info/refs?service=git-upload-pack");
    let body = http::get_text_capped(&url, Duration::from_secs(10), MAX_REFS_BYTES)
        .map_err(|e| anyhow::anyhow!("Failed to resolve {BRANCH}: {e}"))?;
    parse_ref_sha(&body, &format!("refs/heads/{BRANCH}"))
}

/// Scan pkt-lines for `<40-hex-sha> <refname>`. Each line is prefixed by a
/// 4-hex length header which we skip past rather than parse — we only need
/// to locate one ref, and the SHA is fixed-width.
///
/// The match requires an exact refname at the line's end (or immediately
/// before a NUL-separated capabilities list), so `refs/heads/master` can't
/// be fooled by a line advertising `refs/heads/master-old`.
fn parse_ref_sha(body: &str, refname: &str) -> Result<String> {
    body.lines()
        .find_map(|line| {
            // Capabilities (if any) trail the refname after a NUL on the
            // first advertised ref; strip them before comparing tails.
            let line = line.split('\0').next().unwrap_or(line).trim_end();
            if !line.ends_with(refname) {
                return None;
            }
            // The SHA ends immediately before the space preceding refname.
            let idx = line.len() - refname.len();
            let sha = line.get(idx.checked_sub(41)?..idx.checked_sub(1)?)?;
            (sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit())).then(|| sha.to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("No {refname} in ref advertisement"))
}

/// Download a directory from the repo using the GitHub Contents API.
/// Recursively fetches subdirectories and downloads each file individually.
///
/// `git_ref` is the resolved commit SHA (or branch name) to pin every
/// listing + raw fetch to. Pinning across the recursion prevents a
/// mid-install `master` advance from producing a torn install where
/// some files come from commit A and others from commit B.
fn download_directory(
    repo_path: &str,
    git_ref: &str,
    dest: &Path,
    count: &mut usize,
    quiet: bool,
) -> Result<()> {
    let api_url = format!(
        "{}/contents/{repo_path}?ref={git_ref}",
        github_api_repo_url()
    );

    let json = github_get_json(&api_url, Some(15))?;

    // The API returns a JSON object with a "message" field on errors (e.g. 404).
    if let Some(err) = classify_github_error(&json, repo_path) {
        return Err(err);
    }

    let entries: Vec<GitHubEntry> =
        serde_json::from_str(&json).context("Failed to parse GitHub API response")?;

    for entry in entries {
        let dest_path = dest.join(&entry.name);
        match entry.entry_type.as_str() {
            "file" => {
                // The contents API's `download_url` is already pinned to
                // the `?ref=<git_ref>` we requested, so reusing it keeps
                // the whole download tied to a single SHA.
                let url = entry
                    .download_url
                    .ok_or_else(|| anyhow::anyhow!("No download URL for {}", entry.path))?;
                if !quiet {
                    print!("  {:<60}\r", entry.path);
                    std::io::stdout().flush().ok();
                }
                download_file_to(&url, &dest_path)?;
                *count += 1;
            }
            "dir" => {
                // No explicit `create_dir_all` here — `download_file_to`
                // creates each file's parent on demand, which covers this
                // subdir as soon as we download anything into it.
                download_directory(&entry.path, git_ref, &dest_path, count, quiet)?;
            }
            _ => {} // skip symlinks, submodules, etc.
        }
    }

    Ok(())
}

/// Inspect a Contents API response body for GitHub's error-object shape
/// (`{"message": "..."}`, e.g. on a 403 rate limit or a 404) and turn it
/// into a user-facing error. Returns `None` for a normal entry-array
/// response, so the caller can fall through to parsing it. Pure — no I/O —
/// so the rate-limit-vs-not-found distinction is directly unit-testable.
fn classify_github_error(json: &str, repo_path: &str) -> Option<anyhow::Error> {
    let err: GitHubError = serde_json::from_str(json).ok()?;
    let message = err.message?;
    if message.contains("rate limit") {
        return Some(anyhow::anyhow!(
            "GitHub API rate limit exceeded (60 requests/hour for unauthenticated \
             clients, shared across your whole network). The archive download that \
             normally avoids this was unavailable. Retry in an hour, or run \
             'hyprlayer ai reinstall' once codeload.github.com is reachable."
        ));
    }
    Some(anyhow::anyhow!(
        "Agent files for '{repo_path}' are not available on GitHub ({message})"
    ))
}

#[derive(Deserialize)]
struct GitHubError {
    message: Option<String>,
}

#[derive(Deserialize)]
struct GitHubEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
    download_url: Option<String>,
}

/// GET a URL and return the response body as a string, with the GitHub API
/// `Accept` header set. `timeout_secs` defaults to 30s.
pub(crate) fn github_get_json(url: &str, timeout_secs: Option<u32>) -> Result<String> {
    let timeout = Duration::from_secs(timeout_secs.unwrap_or(30) as u64);
    http::get_text_capped_with_headers(
        url,
        timeout,
        MAX_API_RESPONSE_BYTES,
        &[("Accept", "application/vnd.github.v3+json")],
    )
    .map_err(|e| anyhow::anyhow!("GitHub API request failed: {e}"))
}

/// Download a single file from the repo to `dest`. Pinned to `master`'s
/// HEAD SHA (or to `BRANCH` if ref resolution fails).
///
/// Used by the opencode plugin install path to restore the plugin file
/// after `telemetry off → on` without re-pulling the entire opencode/
/// bundle. `download_directory` already pins itself across an
/// install; this helper keeps single-file fetches on the same SHA-pin
/// contract so a mid-fetch `master` advance can't deliver a file that
/// references symbols not yet in the cached bundle.
pub(crate) fn download_repo_file(repo_path: &str, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let sha = fetch_master_sha().ok();
    let git_ref = sha.as_deref().unwrap_or(BRANCH);
    let url = raw_github_url(repo_path, git_ref);
    download_file_to(&url, dest)
}

/// Factored out so URL construction is unit-testable without touching the network.
fn raw_github_url(repo_path: &str, git_ref: &str) -> String {
    format!("https://raw.githubusercontent.com/{REPO}/{git_ref}/{repo_path}")
}

/// Download a single file to disk.
///
/// `ureq` returns `Err` on HTTP 4xx/5xx by default, so a 404 HTML page or
/// rate-limit JSON envelope can never be persisted as a fake "agent file" —
/// matching what `--fail-with-body` bought us. The 30s timeout caps the
/// per-file fetch so a stalled connection on the startup auto-reinstall
/// path can't hang the user's command indefinitely. `download_file_capped`
/// already removes a partial file on failure.
fn download_file_to(url: &str, dest: &Path) -> Result<()> {
    http::download_file_capped(url, dest, Duration::from_secs(30), None)
        .map_err(|e| anyhow::anyhow!("Failed to download {}: {e}", dest.display()))
}

/// Template placeholders used in OpenCode agent/command files
const SONNET_MODEL_PLACEHOLDER: &str = "{{SONNET_MODEL}}";
const OPUS_MODEL_PLACEHOLDER: &str = "{{OPUS_MODEL}}";
const ADVERSARIAL_MODEL_PLACEHOLDER: &str = "{{ADVERSARIAL_MODEL}}";

fn replace_model_placeholders(path: &Path, provider: &OpenCodeProvider) -> Result<bool> {
    let content = fs::read_to_string(path)?;

    if !content.contains(SONNET_MODEL_PLACEHOLDER)
        && !content.contains(OPUS_MODEL_PLACEHOLDER)
        && !content.contains(ADVERSARIAL_MODEL_PLACEHOLDER)
    {
        return Ok(false);
    }

    let updated = content
        .replace(SONNET_MODEL_PLACEHOLDER, provider.default_sonnet_model())
        .replace(OPUS_MODEL_PLACEHOLDER, provider.default_opus_model())
        .replace(
            ADVERSARIAL_MODEL_PLACEHOLDER,
            provider.default_adversarial_model(),
        );

    fs::write(path, updated)?;
    Ok(true)
}

/// Files use {{SONNET_MODEL}}, {{OPUS_MODEL}}, and {{ADVERSARIAL_MODEL}} placeholders.
fn update_opencode_models(dest_dir: &Path, provider: &OpenCodeProvider) -> Result<usize> {
    let dirs = ["agents", "commands"];

    dirs.iter()
        .filter_map(|dir| {
            let path = dest_dir.join(dir);
            path.is_dir().then_some(path)
        })
        .flat_map(|dir| fs::read_dir(dir).into_iter().flatten().flatten())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
        .try_fold(0, |count, entry| {
            let updated = replace_model_placeholders(&entry.path(), provider)?;
            Ok::<_, anyhow::Error>(count + usize::from(updated))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, "stub").unwrap();
    }

    #[test]
    fn sync_tree_no_op_on_identical_trees() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        touch(&src.join("a.md"));
        touch(&dest.join("a.md"));
        assert_eq!(sync_tree(&src, &dest).unwrap(), 0);
        assert_eq!(
            fs::read_to_string(dest.join("a.md")).unwrap(),
            fs::read_to_string(src.join("a.md")).unwrap()
        );
    }

    #[test]
    fn sync_tree_writes_missing_file() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        touch(&src.join("a.md"));
        fs::create_dir_all(&dest).unwrap();
        assert_eq!(sync_tree(&src, &dest).unwrap(), 1);
        assert!(dest.join("a.md").is_file());
    }

    #[test]
    fn sync_tree_overwrites_a_differing_file() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("a.md"), "new content").unwrap();
        fs::write(dest.join("a.md"), "old content").unwrap();
        assert_eq!(sync_tree(&src, &dest).unwrap(), 1);
        assert_eq!(
            fs::read_to_string(dest.join("a.md")).unwrap(),
            "new content"
        );
    }

    #[test]
    fn sync_tree_preserves_a_dest_only_file() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        touch(&src.join("a.md"));
        touch(&dest.join("personal.md"));
        assert_eq!(sync_tree(&src, &dest).unwrap(), 1);
        assert!(dest.join("a.md").is_file());
        assert!(
            dest.join("personal.md").is_file(),
            "dest-only file must survive an install"
        );
    }

    #[test]
    fn sync_tree_creates_nested_directories() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        touch(&src.join("skills/foo/bar/SKILL.md"));
        assert_eq!(sync_tree(&src, &dest).unwrap(), 1);
        assert!(dest.join("skills/foo/bar/SKILL.md").is_file());
    }

    #[test]
    fn sync_tree_empty_source_is_a_no_op() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        fs::create_dir_all(&src).unwrap();
        touch(&dest.join("personal.md"));
        assert_eq!(sync_tree(&src, &dest).unwrap(), 0);
        assert!(dest.join("personal.md").is_file());
    }

    #[test]
    fn sync_tree_missing_source_is_a_no_op() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("does-not-exist");
        let dest = temp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        assert_eq!(sync_tree(&src, &dest).unwrap(), 0);
    }

    /// Real digests from the `v1.6.0-rc.1` prerelease that Phase 3 cut, so
    /// the fixture below is the shape GitHub actually returns rather than an
    /// invented one.
    const RC_CLAUDE_DIGEST: &str =
        "42292288c4a5fc6c7f765489da159ceef5c70d0705c2c9d65d999df7bb6c60cd";
    const RC_COPILOT_DIGEST: &str =
        "d4b730a0bb9755e2bd6aa763ef5e9b24bd1644feec1eae90ca08285a26ea5575";

    /// A `/releases/tags/<tag>` body carrying the three bundles plus the
    /// binaries, abridged to the fields we read.
    fn release_json_with_all_bundles() -> String {
        format!(
            r#"{{
                "tag_name": "v1.6.0-rc.1",
                "assets": [
                    {{ "name": "hyprlayer-x86_64-unknown-linux-gnu", "digest": "sha256:{RC_COPILOT_DIGEST}" }},
                    {{ "name": "hyprlayer-assets-claude-1.6.0-rc.1.tar.gz",   "digest": "sha256:{RC_CLAUDE_DIGEST}" }},
                    {{ "name": "hyprlayer-assets-copilot-1.6.0-rc.1.tar.gz",  "digest": "sha256:{RC_COPILOT_DIGEST}" }},
                    {{ "name": "hyprlayer-assets-opencode-1.6.0-rc.1.tar.gz", "digest": "sha256:{RC_CLAUDE_DIGEST}" }}
                ]
            }}"#
        )
    }

    /// Build a minimal but *installable* Claude bundle in the release-asset
    /// shape: paths relative to the harness root, both sentinels present, a
    /// `manifest.json` alongside. Returns the tarball path.
    fn write_bundle_archive(dir: &Path) -> PathBuf {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let entries: [(&str, &[u8]); 3] = [
            ("agents/codebase-locator.md", b"---\nname: locator\n---\n"),
            (
                "skills/code_review/SKILL.md",
                b"---\nname: code_review\n---\n",
            ),
            ("manifest.json", b"{\"version\":\"1.6.0\"}"),
        ];

        let mut bytes: Vec<u8> = Vec::new();
        {
            let enc = GzEncoder::new(&mut bytes, Compression::default());
            let mut builder = tar::Builder::new(enc);
            for (path, data) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Regular);
                header.set_path(path).unwrap();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append(&header, data).unwrap();
            }
            builder.into_inner().unwrap().finish().unwrap();
        }

        let path = dir.join("hyprlayer-assets-claude-1.6.0.tar.gz");
        fs::write(&path, bytes).unwrap();
        path
    }

    fn sha256_of(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(fs::read(path).unwrap());
        hex::encode(hasher.finalize())
    }

    #[test]
    fn asset_name_matches_the_builder_output() {
        assert_eq!(
            asset_name("claude", "1.6.0"),
            "hyprlayer-assets-claude-1.6.0.tar.gz"
        );
        assert_eq!(
            asset_name(AgentTool::OpenCode.repo_dir(), "1.6.0-rc.1"),
            "hyprlayer-assets-opencode-1.6.0-rc.1.tar.gz"
        );
    }

    #[test]
    fn desired_assets_version_is_the_running_binary_version() {
        assert_eq!(desired_assets_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn asset_digest_from_release_picks_this_harness_bundle() {
        let body = release_json_with_all_bundles();
        let digest = asset_digest_from_release(
            &body,
            "v1.6.0-rc.1",
            "hyprlayer-assets-claude-1.6.0-rc.1.tar.gz",
        )
        .unwrap();
        assert_eq!(digest, RC_CLAUDE_DIGEST);
    }

    /// The fallback trigger: a release exists but carries no bundle for this
    /// harness/version (a pre-Phase-3 release, or a dev build whose version
    /// was never tagged).
    #[test]
    fn asset_digest_from_release_missing_asset_is_an_error() {
        let body = release_json_with_all_bundles();
        let err =
            asset_digest_from_release(&body, "v1.6.0-rc.1", "hyprlayer-assets-claude-9.9.9.tar.gz")
                .unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("hyprlayer-assets-claude-9.9.9.tar.gz"),
            "{text}"
        );
        assert!(text.contains("v1.6.0-rc.1"), "{text}");
    }

    /// Fail closed: an asset GitHub advertises without a digest must not be
    /// downloaded unverified.
    #[test]
    fn asset_digest_from_release_undigested_asset_is_an_error() {
        let body = r#"{ "assets": [ { "name": "hyprlayer-assets-claude-1.6.0.tar.gz" } ] }"#;
        let err = asset_digest_from_release(body, "v1.6.0", "hyprlayer-assets-claude-1.6.0.tar.gz")
            .unwrap_err();
        assert!(err.to_string().contains("unverified"), "{err}");
    }

    #[test]
    fn asset_digest_from_release_handles_a_404_body() {
        let err = asset_digest_from_release(
            r#"{"message":"Not Found"}"#,
            "v9.9.9",
            "hyprlayer-assets-claude-9.9.9.tar.gz",
        )
        .unwrap_err();
        assert!(err.to_string().contains("v9.9.9"), "{err}");
    }

    /// Positive control for `asset_digest_mismatch_aborts_and_leaves_dest_untouched`:
    /// the very same archive and destination, with the digest GitHub would
    /// have reported, installs cleanly. Without this the mismatch test could
    /// pass on an archive that was never installable in the first place.
    #[test]
    fn verified_bundle_extracts_and_installs() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_bundle_archive(tmp.path());
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("settings.json"), "user data").unwrap();

        let expected = format!("sha256:{}", sha256_of(&archive));
        let count = verify_and_extract_bundle(
            &archive,
            "hyprlayer-assets-claude-1.6.0.tar.gz",
            &expected,
            &staged,
        )
        .unwrap();
        assert_eq!(count, 3);
        assert!(
            AgentTool::Claude.is_installed_at(&staged),
            "the fixture bundle must satisfy the completeness gate"
        );

        let changed = AgentTool::Claude.install_staged(&staged, &dest).unwrap();
        assert_eq!(changed, 3);
        assert!(dest.join("skills/code_review/SKILL.md").is_file());
        assert!(dest.join("agents/codebase-locator.md").is_file());
        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "user data"
        );
    }

    #[test]
    fn asset_digest_mismatch_aborts_and_leaves_dest_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_bundle_archive(tmp.path());
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("settings.json"), "user data").unwrap();

        let wrong = "0".repeat(64);
        let err = verify_and_extract_bundle(
            &archive,
            "hyprlayer-assets-claude-1.6.0.tar.gz",
            &wrong,
            &staged,
        )
        .unwrap_err();
        assert!(
            format!("{err:#}").contains("Integrity check failed"),
            "{err:#}"
        );
        assert!(
            !staged.exists(),
            "a bundle that fails verification must never be extracted"
        );

        // Nothing staged means the completeness gate refuses before `dest`
        // is opened at all — the rollback guarantee.
        let gate = AgentTool::Claude
            .install_staged(&staged, &dest)
            .unwrap_err();
        assert!(gate.to_string().contains("incomplete"), "{gate}");
        assert_eq!(
            walk_files(&dest).unwrap(),
            vec![dest.join("settings.json")],
            "a rejected bundle must leave dest byte-identical"
        );
        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "user data"
        );
    }

    #[test]
    fn fetch_first_available_falls_back_when_the_asset_is_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");

        let sources: Vec<BundleSource<'_>> = vec![
            (
                "the v9.9.9 release asset".to_string(),
                Box::new(|_: &Path| {
                    anyhow::bail!(
                        "GitHub release `v9.9.9` exposes no SHA256 digest for asset \
                         `hyprlayer-assets-claude-9.9.9.tar.gz`."
                    )
                }),
            ),
            (
                "the master repo archive".to_string(),
                Box::new(|dest: &Path| {
                    touch(&dest.join("agents/codebase-locator.md"));
                    touch(&dest.join("skills/code_review/SKILL.md"));
                    Ok(2)
                }),
            ),
        ];

        let (label, count) = fetch_first_available(&staged, sources, true).unwrap();
        assert_eq!(label, "the master repo archive");
        assert_eq!(count, 2);
        assert!(
            AgentTool::Claude.is_installed_at(&staged),
            "the fallback's output is what gets staged"
        );
    }

    #[test]
    fn fetch_first_available_prefers_the_first_working_source() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");

        let sources: Vec<BundleSource<'_>> = vec![
            (
                "the release asset".to_string(),
                Box::new(|dest: &Path| {
                    touch(&dest.join("from-asset.md"));
                    Ok(1)
                }),
            ),
            (
                "the master repo archive".to_string(),
                Box::new(|_: &Path| panic!("later sources must not run after a success")),
            ),
        ];

        let (label, count) = fetch_first_available(&staged, sources, true).unwrap();
        assert_eq!(label, "the release asset");
        assert_eq!(count, 1);
        assert!(staged.join("from-asset.md").is_file());
    }

    /// A source that fails partway through extraction must not leave files
    /// behind for the next source to be judged on — the completeness gate
    /// would otherwise pass on a mixture of two downloads.
    #[test]
    fn fetch_first_available_discards_a_failed_sources_partial_output() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");

        let sources: Vec<BundleSource<'_>> = vec![
            (
                "the release asset".to_string(),
                Box::new(|dest: &Path| {
                    touch(&dest.join("agents/half-written.md"));
                    anyhow::bail!("connection reset mid-extraction")
                }),
            ),
            (
                "the master repo archive".to_string(),
                Box::new(|dest: &Path| {
                    touch(&dest.join("agents/complete.md"));
                    Ok(1)
                }),
            ),
        ];

        fetch_first_available(&staged, sources, true).unwrap();
        assert_eq!(
            walk_files(&staged).unwrap(),
            vec![staged.join("agents/complete.md")],
            "the failed source's leftovers must not survive into the fallback"
        );
    }

    /// The last source is the Contents-API walk, whose error carries the
    /// rate-limit guidance; that's the message the user must end up seeing.
    #[test]
    fn fetch_first_available_propagates_the_last_error() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");

        let sources: Vec<BundleSource<'_>> = vec![
            (
                "the release asset".to_string(),
                Box::new(|_: &Path| anyhow::bail!("no such release")),
            ),
            (
                "the master repo archive".to_string(),
                Box::new(|_: &Path| anyhow::bail!("codeload unreachable")),
            ),
            (
                "the GitHub API walk".to_string(),
                Box::new(|_: &Path| anyhow::bail!("GitHub API rate limit exceeded")),
            ),
        ];

        let err = fetch_first_available(&staged, sources, true).unwrap_err();
        assert!(err.to_string().contains("rate limit"), "{err}");
    }

    #[test]
    fn fetch_first_available_with_no_sources_errors() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(fetch_first_available(&tmp.path().join("staged"), vec![], true).is_err());
    }

    #[test]
    fn classify_github_error_rate_limit_names_the_rate_limit() {
        let json =
            r#"{"message":"API rate limit exceeded for 1.2.3.4.","documentation_url":"..."}"#;
        let err = classify_github_error(json, "claude").expect("should classify as an error");
        assert!(
            err.to_string().contains("rate limit"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn classify_github_error_not_found_names_the_path() {
        let json = r#"{"message":"Not Found","documentation_url":"..."}"#;
        let err = classify_github_error(json, "claude/skills/foo").expect("should classify");
        let text = err.to_string();
        assert!(
            text.contains("claude/skills/foo"),
            "unexpected error: {text}"
        );
        assert!(text.contains("Not Found"), "unexpected error: {text}");
        assert!(!text.contains("rate limit"), "unexpected error: {text}");
    }

    #[test]
    fn classify_github_error_valid_entries_is_none() {
        let json = r#"[{"name":"a.md","path":"claude/a.md","type":"file","download_url":"https://example.com/a.md"}]"#;
        assert!(classify_github_error(json, "claude").is_none());
    }

    #[test]
    fn classify_github_error_malformed_json_is_none() {
        // Not our job to diagnose a malformed body — the caller's own JSON
        // parse of the entry array will surface that error.
        assert!(classify_github_error("not json", "claude").is_none());
    }

    #[test]
    fn parse_ref_sha_happy_path() {
        // Captured shape of a real `info/refs?service=git-upload-pack`
        // advertisement (length-prefix headers included but not parsed).
        let body = "001e# service=git-upload-pack\n\
                     0000\
                     0032aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa HEAD\0multi_ack thin-pack\n\
                     003f1f7370976053d293da0718c00aab5faa78396e6a refs/heads/master\n\
                     0000";
        assert_eq!(
            parse_ref_sha(body, "refs/heads/master").unwrap(),
            "1f7370976053d293da0718c00aab5faa78396e6a"
        );
    }

    #[test]
    fn parse_ref_sha_ignores_similarly_named_refs() {
        let body = "001e# service=git-upload-pack\n\
                     003faaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa refs/heads/master-old\n\
                     003fbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/heads/master\n";
        assert_eq!(
            parse_ref_sha(body, "refs/heads/master").unwrap(),
            "b".repeat(40)
        );
    }

    #[test]
    fn parse_ref_sha_head_only_advertisement_has_no_master() {
        let body = "001e# service=git-upload-pack\n\
                     0032aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa HEAD\0multi_ack\n\
                     0000";
        let err = parse_ref_sha(body, "refs/heads/master").unwrap_err();
        assert!(err.to_string().contains("refs/heads/master"), "{err}");
    }

    #[test]
    fn parse_ref_sha_malformed_body_errors() {
        let err = parse_ref_sha("not a ref advertisement", "refs/heads/master").unwrap_err();
        assert!(err.to_string().contains("refs/heads/master"), "{err}");
    }

    #[test]
    fn parse_ref_sha_empty_body_errors() {
        assert!(parse_ref_sha("", "refs/heads/master").is_err());
    }

    #[test]
    fn parse_ref_sha_rejects_short_sha_field() {
        // Line ends with the refname but has too little text before it to
        // contain a 40-hex SHA.
        let body = "master refs/heads/master\n";
        assert!(parse_ref_sha(body, "refs/heads/master").is_err());
    }

    #[test]
    fn parse_ref_sha_rejects_non_hex_sha_field() {
        let body = format!("{} refs/heads/master\n", "z".repeat(40));
        assert!(parse_ref_sha(&body, "refs/heads/master").is_err());
    }

    #[test]
    fn dest_display_uses_platform_separator() {
        for tool in AgentTool::ALL {
            let display = tool.dest_display();
            assert!(
                !display.contains(if SEP == "/" { "\\" } else { "/" }),
                "{} dest_display contains wrong separator: {}",
                tool,
                display
            );
            assert!(
                display.ends_with(SEP),
                "{} dest_display should end with SEP: {}",
                tool,
                display
            );
        }
    }

    #[test]
    fn dest_display_claude_contains_claude_dir() {
        let display = AgentTool::Claude.dest_display();
        assert!(
            display.contains(".claude"),
            "Expected .claude in: {}",
            display
        );
    }

    #[test]
    fn dest_display_opencode_contains_opencode_dir() {
        let display = AgentTool::OpenCode.dest_display();
        assert!(
            display.contains("opencode"),
            "Expected opencode in: {}",
            display
        );
    }

    #[test]
    fn dest_display_copilot_contains_code_user() {
        let display = AgentTool::Copilot.dest_display();
        assert!(
            display.contains(&format!("Code{SEP}User")),
            "Expected Code{}User in: {}",
            SEP,
            display
        );
    }

    #[test]
    fn opencode_provider_serializes_to_kebab_case() {
        let json = serde_json::to_string(&OpenCodeProvider::GithubCopilot).unwrap();
        assert_eq!(json, "\"github-copilot\"");

        let json = serde_json::to_string(&OpenCodeProvider::Anthropic).unwrap();
        assert_eq!(json, "\"anthropic\"");

        let json = serde_json::to_string(&OpenCodeProvider::Abacus).unwrap();
        assert_eq!(json, "\"abacus\"");
    }

    #[test]
    fn opencode_provider_deserializes_from_kebab_case() {
        let provider: OpenCodeProvider = serde_json::from_str("\"github-copilot\"").unwrap();
        assert_eq!(provider, OpenCodeProvider::GithubCopilot);

        let provider: OpenCodeProvider = serde_json::from_str("\"anthropic\"").unwrap();
        assert_eq!(provider, OpenCodeProvider::Anthropic);

        let provider: OpenCodeProvider = serde_json::from_str("\"abacus\"").unwrap();
        assert_eq!(provider, OpenCodeProvider::Abacus);
    }

    #[test]
    fn opencode_provider_display_names() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.to_string(),
            "GitHub Copilot"
        );
        assert_eq!(OpenCodeProvider::Anthropic.to_string(), "Anthropic");
        assert_eq!(OpenCodeProvider::Abacus.to_string(), "Abacus");
    }

    #[test]
    fn opencode_provider_sonnet_models() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.default_sonnet_model(),
            "github-copilot/claude-sonnet-5"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_sonnet_model(),
            "anthropic/claude-sonnet-5"
        );
        assert_eq!(
            OpenCodeProvider::Abacus.default_sonnet_model(),
            "abacus/claude-sonnet-5"
        );
    }

    #[test]
    fn opencode_provider_opus_models() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.default_opus_model(),
            "github-copilot/claude-opus-5"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_opus_model(),
            "anthropic/claude-opus-5"
        );
        assert_eq!(
            OpenCodeProvider::Abacus.default_opus_model(),
            "abacus/claude-opus-5"
        );
    }

    #[test]
    fn opencode_provider_adversarial_models() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.default_adversarial_model(),
            "github-copilot/gpt-5.3-codex"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_adversarial_model(),
            "anthropic/claude-opus-5"
        );
        assert_eq!(
            OpenCodeProvider::Abacus.default_adversarial_model(),
            "abacus/gpt-5.3-codex-xhigh"
        );
    }

    #[test]
    fn opencode_provider_prefixes() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.provider_prefix(),
            "github-copilot"
        );
        assert_eq!(OpenCodeProvider::Anthropic.provider_prefix(), "anthropic");
        assert_eq!(OpenCodeProvider::Abacus.provider_prefix(), "abacus");
    }

    #[test]
    fn replace_model_placeholders_replaces_sonnet() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_sonnet_placeholder");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("test_agent.md");

        let content = "---\nmodel: {{SONNET_MODEL}}\n---\n# Agent";
        fs::write(&file_path, content).unwrap();

        let updated =
            replace_model_placeholders(&file_path, &OpenCodeProvider::GithubCopilot).unwrap();
        assert!(updated);

        let result = fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("model: github-copilot/claude-sonnet-5"));
        assert!(!result.contains("{{SONNET_MODEL}}"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn replace_model_placeholders_replaces_opus() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_opus_placeholder");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("research.md");

        let content = "---\nmodel: {{OPUS_MODEL}}\n---\n# Research";
        fs::write(&file_path, content).unwrap();

        let updated = replace_model_placeholders(&file_path, &OpenCodeProvider::Abacus).unwrap();
        assert!(updated);

        let result = fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("model: abacus/claude-opus-5"));
        assert!(!result.contains("{{OPUS_MODEL}}"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn replace_model_placeholders_replaces_adversarial() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_adversarial_placeholder");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("adversarial-reviewer.md");

        let content = "---\nmodel: {{ADVERSARIAL_MODEL}}\n---\n# Adversarial";
        fs::write(&file_path, content).unwrap();

        let updated = replace_model_placeholders(&file_path, &OpenCodeProvider::Abacus).unwrap();
        assert!(updated);

        let result = fs::read_to_string(&file_path).unwrap();
        assert!(result.contains("model: abacus/gpt-5.3-codex-xhigh"));
        assert!(!result.contains("{{ADVERSARIAL_MODEL}}"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn replace_model_placeholders_skips_files_without_placeholders() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_no_placeholder");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("no_placeholder.md");

        let content = "---\ndescription: No model field\n---\n# Test";
        fs::write(&file_path, content).unwrap();

        let updated = replace_model_placeholders(&file_path, &OpenCodeProvider::Anthropic).unwrap();
        assert!(!updated);

        let result = fs::read_to_string(&file_path).unwrap();
        assert_eq!(result, content);

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn update_opencode_models_replaces_placeholders() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_opencode_placeholders");
        let agents_dir = temp_dir.join("agents");
        let commands_dir = temp_dir.join("commands");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::create_dir_all(&commands_dir).unwrap();

        // Agent with sonnet placeholder
        fs::write(
            agents_dir.join("analyzer.md"),
            "---\nmodel: {{SONNET_MODEL}}\n---\n# Analyzer",
        )
        .unwrap();

        // Command with opus placeholder
        fs::write(
            commands_dir.join("research.md"),
            "---\nmodel: {{OPUS_MODEL}}\n---\n# Research",
        )
        .unwrap();

        // Command without placeholder (should not count)
        fs::write(
            commands_dir.join("commit.md"),
            "---\ndescription: Commit\n---\n# Commit",
        )
        .unwrap();

        let count = update_opencode_models(&temp_dir, &OpenCodeProvider::GithubCopilot).unwrap();
        assert_eq!(count, 2); // Only files with placeholders

        let agent = fs::read_to_string(agents_dir.join("analyzer.md")).unwrap();
        assert!(agent.contains("model: github-copilot/claude-sonnet-5"));

        let research = fs::read_to_string(commands_dir.join("research.md")).unwrap();
        assert!(research.contains("model: github-copilot/claude-opus-5"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn update_opencode_models_replaces_adversarial_alongside_others() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_adversarial_with_others");
        let agents_dir = temp_dir.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        fs::write(
            agents_dir.join("adversarial-reviewer.md"),
            "---\nmodel: {{ADVERSARIAL_MODEL}}\n---\n# Adversarial",
        )
        .unwrap();
        fs::write(
            agents_dir.join("analyzer.md"),
            "---\nmodel: {{SONNET_MODEL}}\n---\n# Analyzer",
        )
        .unwrap();

        let count = update_opencode_models(&temp_dir, &OpenCodeProvider::Abacus).unwrap();
        assert_eq!(count, 2);

        let adversarial = fs::read_to_string(agents_dir.join("adversarial-reviewer.md")).unwrap();
        assert!(adversarial.contains("model: abacus/gpt-5.3-codex-xhigh"));
        assert!(!adversarial.contains("{{ADVERSARIAL_MODEL}}"));

        let analyzer = fs::read_to_string(agents_dir.join("analyzer.md")).unwrap();
        assert!(analyzer.contains("model: abacus/claude-sonnet-5"));

        fs::remove_dir_all(&temp_dir).ok();
    }

    /// Round-trip test: copy the real shipped
    /// assets/opencode/agents/adversarial-reviewer.md into a tempdir and
    /// verify substitution leaves no `{{...}}` placeholders behind for any
    /// provider. Catches regressions where someone removes the placeholder
    /// from the template or adds a new placeholder without updating the
    /// substitution machinery.
    #[test]
    fn opencode_adversarial_reviewer_template_substitutes_for_all_providers() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let template = manifest_dir.join("assets/opencode/agents/adversarial-reviewer.md");
        let template_body = fs::read_to_string(&template).expect("opencode template missing");

        for provider in OpenCodeProvider::ALL {
            let temp_dir = std::env::temp_dir().join(format!(
                "hyprlayer_test_real_template_{}",
                provider.provider_prefix()
            ));
            let agents_dir = temp_dir.join("agents");
            fs::create_dir_all(&agents_dir).unwrap();
            fs::write(agents_dir.join("adversarial-reviewer.md"), &template_body).unwrap();

            update_opencode_models(&temp_dir, provider).unwrap();

            let resolved = fs::read_to_string(agents_dir.join("adversarial-reviewer.md")).unwrap();
            assert!(
                !resolved.contains("{{"),
                "{:?} substitution left a `{{{{...}}}}` placeholder in the template:\n{}",
                provider,
                resolved
            );
            assert!(
                resolved.contains(&format!("model: {}", provider.default_adversarial_model())),
                "{:?} did not produce the expected model line. Got:\n{}",
                provider,
                resolved
            );

            fs::remove_dir_all(&temp_dir).ok();
        }
    }

    #[test]
    fn claude_is_installed_requires_skills() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_claude_is_installed");
        fs::remove_dir_all(&temp_root).ok();

        let case_full = temp_root.join("full");
        touch(&case_full.join("skills/code_review/SKILL.md"));
        touch(&case_full.join("agents/codebase-locator.md"));
        assert!(AgentTool::Claude.is_installed_at(&case_full));

        // Existing install with the right top-level dirs but no sentinels —
        // configure --no-force must re-run to provision the new bundle.
        let case_dirs_only = temp_root.join("dirs_only");
        fs::create_dir_all(case_dirs_only.join("skills")).unwrap();
        fs::create_dir_all(case_dirs_only.join("agents")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_dirs_only));

        // Old layout (commands/ instead of skills/) must report not-installed.
        let case_legacy = temp_root.join("commands_and_agents");
        fs::create_dir_all(case_legacy.join("commands")).unwrap();
        fs::create_dir_all(case_legacy.join("agents")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_legacy));

        let case_skills_only = temp_root.join("skills_only");
        fs::create_dir_all(case_skills_only.join("skills")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_skills_only));

        let case_agents_only = temp_root.join("agents_only");
        fs::create_dir_all(case_agents_only.join("agents")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_agents_only));

        let case_no_agent = temp_root.join("no_locator_agent");
        touch(&case_no_agent.join("skills/code_review/SKILL.md"));
        fs::create_dir_all(case_no_agent.join("agents")).unwrap();
        assert!(!AgentTool::Claude.is_installed_at(&case_no_agent));

        fs::remove_dir_all(&temp_root).ok();
    }

    #[test]
    fn opencode_is_installed_requires_code_review_and_codebase_locator() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_opencode_is_installed");
        fs::remove_dir_all(&temp_root).ok();

        let case_full = temp_root.join("full");
        touch(&case_full.join("commands/code_review.md"));
        touch(&case_full.join("agents/codebase-locator.md"));
        assert!(AgentTool::OpenCode.is_installed_at(&case_full));

        let case_dirs_only = temp_root.join("dirs_only");
        fs::create_dir_all(case_dirs_only.join("commands")).unwrap();
        fs::create_dir_all(case_dirs_only.join("agents")).unwrap();
        assert!(!AgentTool::OpenCode.is_installed_at(&case_dirs_only));

        let case_no_agent = temp_root.join("no_locator_agent");
        touch(&case_no_agent.join("commands/code_review.md"));
        fs::create_dir_all(case_no_agent.join("agents")).unwrap();
        assert!(!AgentTool::OpenCode.is_installed_at(&case_no_agent));

        fs::remove_dir_all(&temp_root).ok();
    }

    #[test]
    fn copilot_is_installed_requires_code_review_and_codebase_locator() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_copilot_is_installed");
        fs::remove_dir_all(&temp_root).ok();

        let case_full = temp_root.join("full");
        touch(&case_full.join("prompts/code_review.prompt.md"));
        touch(&case_full.join("agents/codebase-locator.agent.md"));
        assert!(AgentTool::Copilot.is_installed_at(&case_full));

        let case_dirs_only = temp_root.join("dirs_only");
        fs::create_dir_all(case_dirs_only.join("prompts")).unwrap();
        fs::create_dir_all(case_dirs_only.join("agents")).unwrap();
        assert!(!AgentTool::Copilot.is_installed_at(&case_dirs_only));

        fs::remove_dir_all(&temp_root).ok();
    }

    /// `has_existing_install` must accept any layout that *was* a valid
    /// install at some point — sentinel files may have moved/renamed
    /// between bundles, but the structural directories haven't. A pre-
    /// `code_review` install is exactly the case the auto-reinstall path
    /// needs to refresh.
    #[test]
    fn has_existing_install_accepts_dirs_without_current_sentinels() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_has_existing_install");
        fs::remove_dir_all(&temp_root).ok();

        for (tool, dir_a, dir_b) in [
            (AgentTool::Claude, "skills", "agents"),
            (AgentTool::OpenCode, "commands", "agents"),
            (AgentTool::Copilot, "prompts", "agents"),
        ] {
            // Bare structural dirs (no sentinels) — `is_installed_at`
            // would reject this; `has_existing_install_at` must accept it.
            let dest = temp_root.join(format!("{tool:?}_dirs_only"));
            fs::create_dir_all(dest.join(dir_a)).unwrap();
            fs::create_dir_all(dest.join(dir_b)).unwrap();
            assert!(
                tool.has_existing_install_at(&dest),
                "{tool:?} should treat bare structural dirs as a prior install"
            );
            assert!(
                !tool.is_installed_at(&dest),
                "{tool:?} strict check should reject the bare-dirs case"
            );

            // Missing one of the two structural dirs — not a real install.
            let partial = temp_root.join(format!("{tool:?}_partial"));
            fs::create_dir_all(partial.join(dir_a)).unwrap();
            assert!(
                !tool.has_existing_install_at(&partial),
                "{tool:?} should not treat a half-populated dir as installed"
            );

            // Empty dest dir — never installed.
            let empty = temp_root.join(format!("{tool:?}_empty"));
            fs::create_dir_all(&empty).unwrap();
            assert!(
                !tool.has_existing_install_at(&empty),
                "{tool:?} should not treat an empty dir as installed"
            );
        }

        fs::remove_dir_all(&temp_root).ok();
    }

    #[test]
    fn raw_github_url_pins_to_sha_when_provided() {
        let url = raw_github_url("opencode/plugins/hyprlayer-telemetry.ts", "abc123def456");
        assert!(url.contains("/abc123def456/"), "missing SHA pin: {url}");
        assert!(
            url.starts_with("https://raw.githubusercontent.com/"),
            "wrong host: {url}"
        );
        assert!(url.contains(REPO), "missing repo slug: {url}");
    }

    #[test]
    fn raw_github_url_falls_back_to_branch() {
        let url = raw_github_url("opencode/plugins/hyprlayer-telemetry.ts", BRANCH);
        assert!(
            url.contains(&format!("/{BRANCH}/")),
            "missing branch ref: {url}"
        );
    }

    #[test]
    fn raw_github_url_includes_repo_path() {
        let url = raw_github_url("opencode/plugins/hyprlayer-telemetry.ts", "deadbeef");
        assert!(
            url.ends_with("/opencode/plugins/hyprlayer-telemetry.ts"),
            "wrong tail: {url}"
        );
    }

    #[test]
    fn update_opencode_models_with_different_providers() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_test_providers");
        let commands_dir = temp_dir.join("commands");
        fs::create_dir_all(&commands_dir).unwrap();

        // Test with Anthropic
        fs::write(
            commands_dir.join("test.md"),
            "---\nmodel: {{SONNET_MODEL}}\nopus: {{OPUS_MODEL}}\n---\n# Test",
        )
        .unwrap();

        update_opencode_models(&temp_dir, &OpenCodeProvider::Anthropic).unwrap();

        let result = fs::read_to_string(commands_dir.join("test.md")).unwrap();
        assert!(result.contains("model: anthropic/claude-sonnet-5"));
        assert!(result.contains("opus: anthropic/claude-opus-5"));

        fs::remove_dir_all(&temp_dir).ok();
    }
}
