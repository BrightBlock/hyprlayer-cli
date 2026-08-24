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
            Self::GithubCopilot => "github-copilot/claude-sonnet-4.5",
            Self::Anthropic => "anthropic/claude-sonnet-4-5",
            Self::Abacus => "abacus/claude-sonnet-4-6",
        }
    }

    pub fn default_opus_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/claude-opus-4.5",
            Self::Anthropic => "anthropic/claude-opus-4-5",
            Self::Abacus => "abacus/claude-opus-4-6",
        }
    }

    /// Abacus routes to its highest-reasoning codex variant for a true
    /// cross-model second opinion; GitHub Copilot uses gpt-5-codex (the
    /// codex variant exposed through Copilot Chat); Anthropic stays on
    /// claude-opus-4-5 because the Anthropic API is Claude-only.
    pub fn default_adversarial_model(&self) -> &str {
        match self {
            Self::GithubCopilot => "github-copilot/gpt-5-codex",
            Self::Anthropic => "anthropic/claude-opus-4-5",
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

        // Refuse to touch the destination unless the staged bundle looks
        // complete. Without this, a truncated download could overwrite a
        // good install with a torn one.
        if !self.is_installed_at(&staged) {
            anyhow::bail!(
                "Downloaded {} bundle is incomplete — refusing to install. \
                 Run 'hyprlayer ai reinstall' to retry.",
                self
            );
        }

        fs::create_dir_all(&dest)?;
        let changed = sync_tree(&staged, &dest)?;

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

    /// Populate `staged` with this tool's bundle pinned to `git_ref`.
    ///
    /// Tries the single-request archive download first (zero REST API
    /// requests, see `archive::fetch_and_extract`); on any failure — a
    /// network hiccup, a codeload outage — falls back to the old
    /// Contents-API walk so an archive-side outage doesn't block installs.
    /// Both paths write into `staged`, so the Phase 4 completeness check
    /// and rollback guarantee cover the fallback too: a rate-limited
    /// fallback aborts cleanly instead of leaving a partial `dest`.
    fn fetch_into(&self, staged: &Path, git_ref: &str, quiet: bool) -> Result<()> {
        if !quiet {
            println!("Downloading {} agent files...", self);
        }
        match archive::fetch_and_extract(self.repo_dir(), git_ref, staged) {
            Ok(count) => {
                if !quiet {
                    println!("  {:<60}", format!("Downloaded {count} files"));
                }
                return Ok(());
            }
            Err(e) => {
                if !quiet {
                    eprintln!("  Archive download failed ({e}); falling back to the GitHub API.");
                }
            }
        }

        let mut count = 0;
        download_directory(self.repo_dir(), git_ref, staged, &mut count, quiet)?;
        if !quiet {
            println!("  {:<60}", format!("Downloaded {count} files"));
        }
        Ok(())
    }
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
            "github-copilot/claude-sonnet-4.5"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_sonnet_model(),
            "anthropic/claude-sonnet-4-5"
        );
        assert_eq!(
            OpenCodeProvider::Abacus.default_sonnet_model(),
            "abacus/claude-sonnet-4-6"
        );
    }

    #[test]
    fn opencode_provider_opus_models() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.default_opus_model(),
            "github-copilot/claude-opus-4.5"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_opus_model(),
            "anthropic/claude-opus-4-5"
        );
        assert_eq!(
            OpenCodeProvider::Abacus.default_opus_model(),
            "abacus/claude-opus-4-6"
        );
    }

    #[test]
    fn opencode_provider_adversarial_models() {
        assert_eq!(
            OpenCodeProvider::GithubCopilot.default_adversarial_model(),
            "github-copilot/gpt-5-codex"
        );
        assert_eq!(
            OpenCodeProvider::Anthropic.default_adversarial_model(),
            "anthropic/claude-opus-4-5"
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
        assert!(result.contains("model: github-copilot/claude-sonnet-4.5"));
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
        assert!(result.contains("model: abacus/claude-opus-4-6"));
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
        assert!(agent.contains("model: github-copilot/claude-sonnet-4.5"));

        let research = fs::read_to_string(commands_dir.join("research.md")).unwrap();
        assert!(research.contains("model: github-copilot/claude-opus-4.5"));

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
        assert!(analyzer.contains("model: abacus/claude-sonnet-4-6"));

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
        assert!(result.contains("model: anthropic/claude-sonnet-4-5"));
        assert!(result.contains("opus: anthropic/claude-opus-4-5"));

        fs::remove_dir_all(&temp_dir).ok();
    }
}
