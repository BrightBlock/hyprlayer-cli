pub(crate) mod archive;
pub(crate) mod manifest;

use anyhow::{Context, Result};
use fs2::FileExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{MAIN_SEPARATOR_STR as SEP, Path, PathBuf};
use std::time::Duration;

use self::manifest::{BundleManifest, MANIFEST_FILE_NAME, ManifestEntry};
use crate::http;
use crate::integrity;

pub(crate) const REPO: &str = "BrightBlock/hyprlayer-cli";
const BRANCH: &str = "master";

/// Record of the bundle currently installed in a harness directory, written
/// after every successful asset install. It is the previous-state the next
/// install diffs against: which files are ours, and what they hashed to when
/// we put them there.
///
/// Distinct from the bundle's own `manifest.json`, which never reaches
/// `dest` — see `install_staged`. Dotted so it stays out of the harness's
/// own file globs.
const INSTALLED_MANIFEST_FILE: &str = ".hyprlayer-manifest.json";

/// Claude and Codex are installed as one supported bundle set: the shared skill source remains
/// `~/.claude/skills`, while standalone Codex also receives custom-agent
/// definitions and a native-root bridge into that source tree.
const CLAUDE_REPO_DIR: &str = "assets/claude";
const CODEX_HARNESS: &str = "codex";
const CODEX_REPO_DIR: &str = "assets/codex";
const AGENT_STORE_DIR: &str = "agents";

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

/// The bundle version an install should resolve to: the explicit pin when
/// one is set, otherwise the running binary's own version — so an unpinned
/// CLI installs its matching bundle and upgrading the binary is what moves
/// the skills, while a pin survives that upgrade.
///
/// The single place this resolution happens; `HyprlayerConfig::
/// desired_assets_version` delegates here rather than restating it.
pub(crate) fn resolve_assets_version(pinned: Option<&str>) -> &str {
    pinned.unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// Release-asset file name for one harness at one version, as
/// `scripts/build-asset-bundles.sh` emits it and `release.yml` attaches it.
///
/// Also what `ai versions` matches release assets against, to keep the
/// listed versions to the ones that can actually be installed.
pub(crate) fn asset_name(harness: &str, version: &str) -> String {
    format!("hyprlayer-assets-{harness}-{version}.tar.gz")
}

/// Internal helper for the Claude half of the pair. This is deliberately
/// private: there is no user-selectable harness installation anymore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentTool {
    Claude,
}

impl fmt::Display for AgentTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Claude => write!(f, "Claude Code"),
        }
    }
}

impl AgentTool {
    #[cfg(test)]
    const ALL: &[AgentTool] = &[AgentTool::Claude];

    fn harness_slug(&self) -> &str {
        match self {
            Self::Claude => "claude",
        }
    }

    fn live_repo_dir(&self) -> &str {
        match self {
            Self::Claude => CLAUDE_REPO_DIR,
        }
    }

    /// The frozen legacy tree as a file list, standing in for the record a
    /// pre-1.6.0 install never wrote.
    ///
    /// Those clients download `claude/` from the repo root, and that tree
    /// has been frozen since 1.6.0 (see
    /// `assets/FROZEN.md`), so what they put in `dest` is known exactly.
    /// `scripts/build-frozen-manifests.sh` generates the lists embedded
    /// here, and `frozen_manifests_match_the_frozen_trees` holds them to
    /// the trees.
    ///
    /// This is used for one thing: deciding what a migration install may
    /// delete. Ownership by digest is what makes deleting a retired
    /// workflow safe — a file that no longer hashes to what the frozen tree
    /// shipped has been edited since, so it is the user's and stays. It is
    /// deliberately not consulted for the overwrite decision: a machine
    /// that last refreshed before the freeze holds older bytes for files we
    /// still ship, and treating those as "not ours" would freeze it on them
    /// forever.
    ///
    /// Embedded JSON that fails to parse would be a bug the unit tests
    /// catch, so a failure here degrades to "nothing is known to be ours" —
    /// an install that cleans nothing up — rather than aborting an
    /// otherwise-good install.
    fn frozen_manifest(&self) -> Vec<ManifestEntry> {
        let json = include_str!("agents/frozen/claude.json");
        serde_json::from_str(json).unwrap_or_else(|e| {
            eprintln!(
                "warning: the built-in {self} file list did not parse ({e}); \
                       skipping cleanup of files retired before 1.6.0"
            );
            Vec::new()
        })
    }

    pub(crate) fn dest_dir(&self) -> Result<PathBuf> {
        match self {
            Self::Claude => {
                let home = dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
                Ok(home.join(".claude"))
            }
        }
    }

    pub fn dest_display(&self) -> String {
        match self {
            Self::Claude => format!("~{SEP}.claude{SEP}"),
        }
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
        // by a previous hyprlayer asset install.
        let (a, b) = ("skills", "agents");
        dest.join(a).is_dir() && dest.join(b).is_dir()
    }

    /// Test-friendly variant of `is_installed` that takes an explicit destination path.
    ///
    /// Checks for sentinel files unique to the current bundle of
    /// commands/skills/agents. An older install with the right top-level
    /// directories but missing newly added files reports not-installed, so
    /// automatic provisioning installs the new bundle. Bump these whenever
    /// we ship a top-level file existing users should pick up.
    ///
    /// **Scope, since 1.6.0**: as a completeness gate this now governs only
    /// the frozen-legacy-tree fallback. An asset install carries a
    /// `manifest.json` and is gated on that instead — every listed file
    /// present and hashing correctly — which is a far stronger check than
    /// two hardcoded paths. This remains the gate for the `master`-tree
    /// fallback, which has no manifest, and remains the "is anything
    /// installed here" probe for both. See `install_staged`.
    fn is_installed_at(&self, dest: &Path) -> bool {
        match self {
            // This sentinel doubles as the staged-download completeness
            // gate for the legacy fallback (`is_installed_at` failing there
            // after a staged fetch is a hard `bail!`, not a soft warning).
            // It may therefore only ever name long-lived, load-bearing
            // files that ship on every release — never a file from a branch
            // not yet on `master`, or reverting that file turns a benign
            // rollback into every user's next automatic install/`ai reinstall`
            // failing outright. The frozen trees make that hazard mostly
            // historical: they no longer change. Both paths below are
            // present in the freeze and in every asset bundle.
            Self::Claude => {
                dest.join("skills/code_review/SKILL.md").is_file()
                    && dest.join("agents/codebase-locator.md").is_file()
            }
        }
    }

    /// Download agent files from GitHub and install to the destination.
    ///
    /// Downloads into a temporary staging directory first, refuses to
    /// touch `dest` at all unless the staged bundle looks complete (see
    /// `is_installed_at`). Claude then populates the versioned central store
    /// and reconciles the Claude/Codex link farms as one rollback-safe
    /// activation. A mid-download failure — network drop, rate limit,
    /// truncated archive — therefore cannot leave a live harness tree
    /// half-downloaded.
    ///
    /// `InstallOutcome::sha` is `Some` when we successfully captured the
    /// repo's `master` HEAD SHA *before* the download (the next 24h
    /// auto-check uses this as the freshness baseline), and `None` when
    /// the ref advertisement was unreachable but the download still
    /// succeeded — the install is still good, but we have no SHA to
    /// cache, so the next auto-check will treat the bundle as stale and
    /// refresh again. We don't fail the whole install when only ref
    /// resolution is unreachable because `ai reinstall` can continue by
    /// falling back to the `master` branch name.
    ///
    /// `pinned_version` is the caller's `agentsPinnedVersion`, if any: it
    /// selects which release asset to fetch and, because a pinned bundle may
    /// have been cut for a newer CLI than this one, is what
    /// `verify_pin_is_supported` gates on.
    fn install(&self, pinned_version: Option<&str>, quiet: bool) -> Result<InstallOutcome> {
        let dest = self.dest_dir()?;

        // Recording a post-download SHA could mask `master`-advances that
        // happen mid-install — next-day's check would then compare against
        // an at-or-newer cache and skip the necessary re-sync.
        let sha = fetch_master_sha().ok();
        let git_ref = sha.as_deref().unwrap_or(BRANCH);

        let staging = tempfile::tempdir().context("Failed to create a staging directory")?;
        let staged = staging.path().join(self.harness_slug());
        self.fetch_into(&staged, git_ref, pinned_version, quiet)?;

        // Fetch and validate the same-version companion before touching the
        // store or either harness directory. A release carrying only one
        // half is not an installable generation.
        let codex_staged = staging.path().join("codex-companion");
        let primary_is_release = staged.join(MANIFEST_FILE_NAME).is_file();
        fetch_codex_companion_into(
            &codex_staged,
            git_ref,
            pinned_version,
            primary_is_release,
            quiet,
        )?;
        let version = resolve_assets_version(pinned_version);
        let report = install_claude_bundle_set(
            &staged,
            &codex_staged,
            &dest,
            &codex_dest_dir()?,
            &agent_store_root()?,
            &codex_skill_bridge_dir()?,
            pinned_version,
            version,
            quiet,
        )?;
        if !quiet {
            report.print();
        }
        let changed = report.changed;

        Ok(InstallOutcome { sha, changed })
    }

    /// The post-fetch half of `install`: refuse to touch `dest` unless the
    /// staged bundle looks complete, then sync only what differs. Without
    /// the gate, a truncated download could overwrite a good install with a
    /// torn one. Split out of `install` so the rollback guarantee is
    /// testable without a network fetch.
    ///
    /// A staged release asset carries a `manifest.json`, and that is what
    /// drives everything: completeness (every listed file present and
    /// hashing to what was recorded), which files in `dest` are ours to
    /// overwrite, and which of the previous bundle's files are now orphans.
    /// The legacy `master`-tree fallback has no manifest, so it keeps the
    /// hardcoded sentinel gate and the historical additive-only sync.
    ///
    /// Orphan removal reads one more record: the frozen tree
    /// (`frozen_manifest`), which is what pre-1.6.0 installs wrote and no
    /// manifest describes. Either way the install finishes by clearing the
    /// `.hyprlayer-backup` copies 1.6.0 left in the content directories
    /// (`clean_backups`).
    #[cfg(test)]
    fn install_staged(
        &self,
        staged: &Path,
        dest: &Path,
        pinned_version: Option<&str>,
    ) -> Result<SyncReport> {
        let bundle = read_staged_manifest(staged)?;
        match &bundle {
            Some(manifest) => {
                verify_pin_is_supported(manifest, pinned_version)?;
                verify_staged_completeness(*self, staged, manifest)?;
                // The manifest describes the bundle rather than being one
                // of the files it owns — the builder leaves it out of
                // `files` — so drop it from the staged tree before the
                // sync. `dest` gets the record under
                // `.hyprlayer-manifest.json`; a stray `manifest.json` in
                // `~/.claude` would be ours by nobody's reckoning, and
                // orphan removal could never clean it up.
                let path = staged.join(MANIFEST_FILE_NAME);
                fs::remove_file(&path)
                    .with_context(|| format!("Failed to remove {}", path.display()))?;
            }
            None => {
                if !self.is_installed_at(staged) {
                    anyhow::bail!(
                        "Downloaded {} bundle is incomplete — refusing to install. \
                         Run 'hyprlayer ai reinstall' to retry.",
                        self
                    );
                }
            }
        }

        let previous = read_installed_manifest(dest);

        fs::create_dir_all(dest)?;
        let mut report = sync_tree(staged, dest, previous.as_ref())?;

        let frozen = self.frozen_manifest();

        if let Some(manifest) = &bundle {
            let mut removed = match &previous {
                Some(previous) => remove_orphans(dest, &previous.files, manifest),
                None => Vec::new(),
            };
            // The workflows retired before 1.6.0 appear in no recorded
            // manifest — the installs that wrote them never wrote one — so
            // the frozen tree is the only thing that can prove they are
            // ours. Consulted on every install rather than only the
            // migration one: the machines that already took 1.6.0 have a
            // record now, and `ci_commit` and friends are still sitting in
            // their skills directory next to the skills that replaced them.
            removed.extend(remove_orphans(dest, &frozen, manifest));
            report.removed = removed;
            write_installed_manifest(dest, manifest)?;
        }

        // Every list of files we have ever owned here, because a backup
        // could have been written beside any of them: what the last install
        // recorded, what a pre-1.6.0 one left, and what this bundle ships.
        // A path in more than one list is swept once — the second look
        // finds no file.
        report.cleaned_backups = clean_backups(dest, &frozen)
            + previous
                .as_ref()
                .map_or(0, |previous| clean_backups(dest, &previous.files))
            + bundle
                .as_ref()
                .map_or(0, |manifest| clean_backups(dest, &manifest.files));

        Ok(report)
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
    /// For an unpinned install the asset step may fail softly — a dev build with no
    /// matching release, a release predating the bundles, a mid-download
    /// network drop, a digest mismatch — because a legacy-tree install is
    /// still a working install. What it must never do is install an
    /// *unverified* asset, so a release that advertises no digest for the
    /// bundle drops through to the fallback rather than downloading it
    /// anyway. An explicit pin has no fallbacks: it resolves to that exact
    /// release asset or fails.
    ///
    /// Every source writes into `staged`, so the completeness check and
    /// rollback guarantee in `install_staged` cover the fallbacks too: a
    /// rate-limited fallback aborts cleanly instead of leaving a partial
    /// `dest`.
    fn fetch_into(
        &self,
        staged: &Path,
        git_ref: &str,
        pinned_version: Option<&str>,
        quiet: bool,
    ) -> Result<()> {
        if !quiet {
            println!("Downloading {} agent files...", self);
        }

        let version = resolve_assets_version(pinned_version);
        let mut sources: Vec<BundleSource<'_>> = vec![(
            format!("the v{version} release asset"),
            Box::new(move |dest: &Path| self.fetch_asset_into(dest, version)),
        )];
        // A pin is a request for one exact asset version. Falling through
        // to master would silently install current files while recording
        // the requested old version, and could pair Claude with Codex from
        // different revisions. Unpinned development builds retain the
        // legacy frozen-tree fallbacks.
        if pinned_version.is_none() {
            sources.push((
                format!("the {BRANCH} repo archive"),
                Box::new(move |dest: &Path| {
                    archive::fetch_and_extract(self.live_repo_dir(), git_ref, dest)
                }),
            ));
            sources.push((
                format!("the GitHub API walk of {BRANCH}"),
                Box::new(move |dest: &Path| {
                    let mut count = 0;
                    download_directory(self.live_repo_dir(), git_ref, dest, &mut count, quiet)?;
                    Ok(count)
                }),
            ));
        }

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
        fetch_harness_asset_into(self.harness_slug(), staged, version)
    }
}

fn fetch_harness_asset_into(harness: &str, staged: &Path, version: &str) -> Result<usize> {
    let asset = asset_name(harness, version);
    let tag = format!("v{version}");

    let api_url = format!("{}/releases/tags/{tag}", github_api_repo_url());
    let release_body =
        http::get_text_capped(&api_url, Duration::from_secs(15), MAX_API_RESPONSE_BYTES)
            .map_err(|e| anyhow::anyhow!("Unable to fetch release {tag} from GitHub: {e}"))?;
    fetch_harness_asset_from_release(&release_body, &tag, &asset, staged)
}

fn fetch_harness_asset_from_release(
    release_body: &str,
    tag: &str,
    asset: &str,
    staged: &Path,
) -> Result<usize> {
    let expected = asset_digest_from_release(release_body, tag, asset)?;

    let tmp = tempfile::tempdir().context("Failed to create a temp dir for the bundle")?;
    let archive_path = tmp.path().join(asset);
    let url = format!("{}/{tag}/{asset}", github_release_download_base());
    http::download_file_capped(
        &url,
        &archive_path,
        Duration::from_secs(30),
        Some(archive::MAX_ARCHIVE_BYTES),
    )
    .map_err(|e| anyhow::anyhow!("Failed to download {url}: {e}"))?;

    verify_and_extract_bundle(&archive_path, asset, &expected, staged)
}

fn codex_dest_dir() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    Ok(home.join(".codex"))
}

fn agent_store_root() -> Result<PathBuf> {
    let config = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    Ok(config.join("hyprlayer").join(AGENT_STORE_DIR))
}

fn acquire_bundle_lock() -> Result<fs::File> {
    let store_root = agent_store_root()?;
    let config_root = store_root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Agent store has no configuration parent"))?;
    fs::create_dir_all(config_root)
        .with_context(|| format!("Failed to create {}", config_root.display()))?;
    let lock_path = config_root.join(".agents.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("Failed to open bundle lock {}", lock_path.display()))?;
    lock.lock_exclusive()
        .with_context(|| format!("Failed to lock {}", lock_path.display()))?;
    Ok(lock)
}

/// Install or update the supported Claude/Codex pair under one process-wide
/// lock. Both assets are staged and validated before either live namespace
/// is changed.
pub(crate) fn install_bundle_set(
    pinned_version: Option<&str>,
    quiet: bool,
) -> Result<InstallOutcome> {
    let _lock = acquire_bundle_lock()?;
    AgentTool::Claude.install(pinned_version, quiet)
}

fn codex_skill_bridge_dir() -> Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    Ok(home.join(".agents").join("skills"))
}

/// Fetch the Codex companion from the same version and source as Claude.
/// Missing companion assets are hard errors: the supported install is an
/// atomic pair, including for old explicit pins.
fn fetch_codex_companion_into(
    staged: &Path,
    git_ref: &str,
    pinned_version: Option<&str>,
    primary_is_release: bool,
    quiet: bool,
) -> Result<()> {
    if !quiet {
        println!("Downloading Codex companion agent files...");
    }
    let version = resolve_assets_version(pinned_version);
    if pinned_version.is_some() {
        let asset = asset_name(CODEX_HARNESS, version);
        let tag = format!("v{version}");
        let api_url = format!("{}/releases/tags/{tag}", github_api_repo_url());
        let release_body =
            http::get_text_capped(&api_url, Duration::from_secs(15), MAX_API_RESPONSE_BYTES)
                .map_err(|e| anyhow::anyhow!("Unable to fetch release {tag} from GitHub: {e}"))?;
        if !release_lists_asset(&release_body, &asset)? {
            anyhow::bail!(
                "Release {tag} does not carry the required Codex companion asset `{asset}`. \
                 Choose a release that includes both Claude and Codex bundles."
            );
        }
        let count = fetch_harness_asset_from_release(&release_body, &tag, &asset, staged)?;
        codex_staged_manifest(staged, pinned_version, version)?;
        if !quiet {
            println!(
                "  {:<60}",
                format!("Downloaded {count} files from the {tag} release asset")
            );
        }
        return Ok(());
    }

    // Keep both halves on one origin as well as one version: a verified
    // Claude release asset pairs only with the Codex release asset, while a
    // development/frozen-tree fallback pairs only with the same repo SHA.
    let sources: Vec<BundleSource<'_>> = if primary_is_release {
        vec![(
            format!("the v{version} release asset"),
            Box::new(move |dest: &Path| fetch_harness_asset_into(CODEX_HARNESS, dest, version)),
        )]
    } else {
        vec![
            (
                format!("the {BRANCH} repo archive"),
                Box::new(move |dest: &Path| {
                    archive::fetch_and_extract(CODEX_REPO_DIR, git_ref, dest)
                }),
            ),
            (
                format!("the GitHub API walk of {BRANCH}"),
                Box::new(move |dest: &Path| {
                    let mut count = 0;
                    download_directory(CODEX_REPO_DIR, git_ref, dest, &mut count, quiet)?;
                    Ok(count)
                }),
            ),
        ]
    };

    let (label, count) = fetch_first_available(staged, sources, quiet)?;
    // Validate now, before the caller mutates ~/.claude. `install_codex_staged`
    // repeats the check at the write boundary so this preflight cannot drift
    // away from the actual install gate.
    codex_staged_manifest(staged, pinned_version, version)?;
    if !quiet {
        println!("  {:<60}", format!("Downloaded {count} files from {label}"));
    }
    Ok(())
}

fn release_lists_asset(release_body: &str, asset: &str) -> Result<bool> {
    let release: serde_json::Value =
        serde_json::from_str(release_body).context("Failed to parse GitHub release response")?;
    let assets = release
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("GitHub release response has no assets array"))?;
    Ok(assets
        .iter()
        .any(|entry| entry.get("name").and_then(serde_json::Value::as_str) == Some(asset)))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn manifest_for_tree(staged: &Path, version: &str, harness: &str) -> Result<BundleManifest> {
    let mut paths = walk_files(staged)?;
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let relative = path
            .strip_prefix(staged)
            .expect("walk_files only returns descendants");
        let Some(key) = manifest::relative_key(relative) else {
            continue;
        };
        if key == MANIFEST_FILE_NAME {
            continue;
        }
        let bytes =
            fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        files.push(ManifestEntry {
            path: key,
            sha256: sha256_bytes(&bytes),
        });
    }
    Ok(BundleManifest {
        version: version.to_string(),
        harness: harness.to_string(),
        min_cli_version: env!("CARGO_PKG_VERSION").to_string(),
        files,
    })
}

fn codex_staged_manifest(
    staged: &Path,
    pinned_version: Option<&str>,
    version: &str,
) -> Result<BundleManifest> {
    let manifest =
        read_staged_manifest(staged)?.unwrap_or(manifest_for_tree(staged, version, CODEX_HARNESS)?);
    if manifest.harness != CODEX_HARNESS {
        anyhow::bail!(
            "Downloaded Codex companion bundle identifies itself as {:?}",
            manifest.harness
        );
    }
    if manifest.version != version {
        anyhow::bail!(
            "Downloaded Codex companion bundle is version {:?}, expected {version:?}",
            manifest.version
        );
    }
    let has_agent = manifest.files.iter().any(|entry| {
        let path = Path::new(&entry.path);
        path.starts_with("agents") && path.extension().is_some_and(|ext| ext == "toml")
    });
    if !has_agent {
        anyhow::bail!(
            "Downloaded Codex companion bundle is incomplete — it contains no agents/*.toml"
        );
    }
    verify_pin_is_supported(&manifest, pinned_version)?;
    verify_staged_completeness_for("Codex companion", staged, &manifest)?;
    Ok(manifest)
}

#[cfg(test)]
fn install_codex_staged(
    staged: &Path,
    dest: &Path,
    pinned_version: Option<&str>,
    version: &str,
) -> Result<SyncReport> {
    let manifest = codex_staged_manifest(staged, pinned_version, version)?;
    let bundled_manifest = staged.join(MANIFEST_FILE_NAME);
    if bundled_manifest.is_file() {
        fs::remove_file(&bundled_manifest)
            .with_context(|| format!("Failed to remove {}", bundled_manifest.display()))?;
    }

    let previous = read_installed_manifest(dest);
    // Codex is newly managed. With no prior ownership record, preserve any
    // differing custom agent already at the same path rather than applying
    // the legacy-harness overwrite migration policy.
    let empty_previous = BundleManifest {
        version: String::new(),
        harness: CODEX_HARNESS.to_string(),
        min_cli_version: String::new(),
        files: Vec::new(),
    };
    fs::create_dir_all(dest)?;
    let mut report = sync_tree(
        staged,
        dest,
        Some(previous.as_ref().unwrap_or(&empty_previous)),
    )?;
    if let Some(previous) = &previous {
        report.removed = remove_orphans(dest, &previous.files, &manifest);
    }
    write_installed_manifest(dest, &manifest)?;
    Ok(report)
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

/// Result of a successful Claude/Codex bundle-set install.
#[derive(Debug)]
pub struct InstallOutcome {
    /// The repo's `master` HEAD SHA captured before the download, if ref
    /// resolution succeeded. `None` means the install still succeeded but
    /// there's no fresh SHA to cache.
    pub sha: Option<String>,
    /// Number of store files, mutable settings/records, or links changed.
    /// `0` means the requested layout was already current.
    pub changed: usize,
}

/// What an install did to `dest`, beyond the file count `InstallOutcome`
/// reports. Paths are manifest-form keys, relative to the harness root.
#[derive(Debug, Default)]
struct SyncReport {
    /// Files or links actually created, overwritten, or repointed.
    changed: usize,
    /// Files left alone because the bytes in `dest` are the user's work,
    /// not the previous bundle's.
    preserved: Vec<String>,
    /// Files deleted because the previous bundle owned them and this one
    /// dropped them. Always empty without a previous manifest.
    removed: Vec<String>,
    /// Leftover `<name>.hyprlayer-backup` copies an earlier install wrote
    /// into the content directories, cleared out by this one.
    cleaned_backups: usize,
}

impl SyncReport {
    /// One line per file we deliberately did not overwrite and per orphan
    /// removed. Both are decisions the user should be able to see: an
    /// install that skipped their edited `settings.json`, or deleted a
    /// skill they still had, must not do it silently. Leftover backups get
    /// one summary line — the count is worth seeing, fifty paths are not.
    fn print(&self) {
        for path in &self.preserved {
            println!("  {:<60}", format!("Kept your modified {path}"));
        }
        for path in &self.removed {
            println!(
                "  {:<60}",
                format!("Removed {path} (no longer in this bundle)")
            );
        }
        if self.cleaned_backups > 0 {
            let plural = if self.cleaned_backups == 1 { "" } else { "s" };
            println!(
                "  {:<60}",
                format!(
                    "Cleared {} leftover {BACKUP_SUFFIX} file{plural} from an earlier install",
                    self.cleaned_backups
                )
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillBridgeOutcome {
    Created,
    AlreadyCorrect,
    SkippedExisting,
}

/// Suffix 1.6.0's first-manifest install used for the copy it took of each
/// file it overwrote. No install writes one any more: inside a skill
/// directory a `SKILL.md.hyprlayer-backup` is a second file sitting exactly
/// where the harness scans for skills, and an upgrade from a pre-1.6.0 tree
/// produced one for nearly every skill we ship. The suffix survives only so
/// `clean_backups` can clear what earlier installs already left on disk.
const BACKUP_SUFFIX: &str = ".hyprlayer-backup";

/// Append `suffix` to the whole file name, extension included, so
/// `settings.json` becomes `settings.json.hyprlayer-backup`. `with_extension`
/// would replace `.json` instead and collide across sibling files.
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Is `relative` the harness's own config rather than hyprlayer content?
///
/// Every bundle keeps its content in subdirectories — `agents/`, `skills/`,
/// `commands/`, `prompts/`, `plugins/` — so the only thing sitting at the
/// harness root is its config file (`~/.claude/settings.json`). We ship one
/// to give a fresh install something to start from, but an existing one
/// holds the user's permissions, hooks and env, none of which a bundle can
/// put back.
fn is_harness_config(relative: &Path) -> bool {
    relative == Path::new("settings.json")
}

/// Copy every file from `src` into `dest`, creating parent directories as
/// needed, but writing a file only when it's missing from `dest` or its
/// bytes differ from what's already there. Never deletes anything from
/// `dest` that isn't present in `src` — `dest` (e.g. `~/.claude`) holds the
/// user's own files alongside ours, so the sync stays
/// additive-and-overwrite rather than a mirror; dropping a file the
/// previous bundle owned is `remove_orphans`' job.
///
/// `previous` is the manifest the last install recorded. A file in `dest`
/// whose bytes match neither the incoming file nor the digest `previous`
/// recorded for that path is the user's work: it is left alone and
/// reported. That is what stops the bundled `settings.json` clobbering
/// `~/.claude/settings.json` on every install.
///
/// With no `previous` — a legacy `master`-tree install, or the first
/// manifest install on top of a pre-1.6.0 one — there is no way to tell our
/// own files from the user's, and skipping every differing file would leave
/// every pre-1.6.0 user frozen on the bundle they already have. Content is
/// therefore replaced outright, with nothing copied aside: the skills and
/// agents we ship are ours to keep current, and anyone who wants an older
/// one pins that bundle version (`ai reinstall --version`) rather than
/// editing files in place. Harness config is the exception — a
/// `settings.json` already on disk is the user's, so it is preserved and
/// our starter copy dropped. The manifest that install writes is what lets
/// every later install tell the two apart by digest instead.
#[cfg(test)]
fn sync_tree(src: &Path, dest: &Path, previous: Option<&BundleManifest>) -> Result<SyncReport> {
    let owned = previous.map(|manifest| manifest.digests());
    let mut report = SyncReport::default();

    for path in walk_files(src)? {
        let relative = path
            .strip_prefix(src)
            .expect("walk_files only yields paths under src");
        let dest_path = dest.join(relative);

        if has_symlink_ancestor(dest, &dest_path) {
            report.preserved.push(
                manifest::relative_key(relative).unwrap_or_else(|| relative.display().to_string()),
            );
            continue;
        }

        let new_bytes =
            fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
        let existing = match fs::symlink_metadata(&dest_path) {
            Ok(metadata) if metadata_is_link(&metadata) || !metadata.is_file() => {
                report.preserved.push(
                    manifest::relative_key(relative)
                        .unwrap_or_else(|| relative.display().to_string()),
                );
                continue;
            }
            Ok(_) => Some(
                fs::read(&dest_path)
                    .with_context(|| format!("Failed to read {}", dest_path.display()))?,
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to inspect {}", dest_path.display()));
            }
        };

        if let Some(existing) = existing {
            if existing == new_bytes {
                continue;
            }
            let key = manifest::relative_key(relative);
            let ours = match &owned {
                Some(owned) => key
                    .as_deref()
                    .and_then(|key| owned.get(key).copied())
                    .is_some_and(|recorded| integrity::bytes_match_sha256(&existing, recorded)),
                // Nothing proves the file either way, so fall back to what
                // kind of file it is: content is ours, config is theirs.
                None => !is_harness_config(relative),
            };
            if !ours {
                report
                    .preserved
                    .push(key.unwrap_or_else(|| relative.display().to_string()));
                continue;
            }
        }

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        fs::write(&dest_path, &new_bytes)
            .with_context(|| format!("Failed to write {}", dest_path.display()))?;
        report.changed += 1;
    }

    Ok(report)
}

/// Clear the `.hyprlayer-backup` copies 1.6.0's first-manifest install left
/// behind.
///
/// That install wrote one beside every file it replaced, which on an upgrade
/// from a pre-1.6.0 tree is most of the bundle: `~/.claude/skills/` came out
/// of it with a `SKILL.md.hyprlayer-backup` inside nearly every skill
/// directory, one directory-mate too many for something the harness parses
/// by scanning. Nothing writes them any more, so an install clears the ones
/// already on disk — including on the daily refresh, so an affected machine
/// heals without anyone running `ai reinstall`.
///
/// Driven by `owned` — a manifest's file list — rather than by walking
/// `dest` for the suffix. Only `<owned path>.hyprlayer-backup` is ever
/// removed, which is exactly the set that install could have written, so a
/// file of the user's that happens to carry the suffix is not ours to
/// delete however it got there. Walking instead would also have followed
/// directory symlinks out of `dest` entirely — `~/.claude/skills/mine`
/// pointing at a dotfiles checkout is an ordinary thing to do — and deleted
/// matching files there.
///
/// Harness config is skipped: `settings.json.hyprlayer-backup` may be the
/// only copy left of a user's own settings, so it stays for them to merge
/// back.
///
/// Nothing here fails the install. It runs after the bundle and the
/// manifest are already on disk, so returning an error would report a
/// failed install that had in fact succeeded, and skip the caller's
/// bookkeeping with it. A leftover we could not delete is worth a warning
/// and nothing more.
fn clean_backups(dest: &Path, owned: &[ManifestEntry]) -> usize {
    let mut cleaned = 0;

    for entry in owned {
        let relative = Path::new(&entry.path);
        if manifest::relative_key(relative).is_none() || is_harness_config(relative) {
            continue;
        }
        let Some(path) = manifest::resolve_under(dest, &entry.path) else {
            continue;
        };
        let backup = append_suffix(&path, BACKUP_SUFFIX);
        if has_symlink_ancestor(dest, &backup) {
            continue;
        }
        // `symlink_metadata`, so a symlink wearing the suffix is left alone
        // rather than unlinked out from under whatever made it.
        if !fs::symlink_metadata(&backup).is_ok_and(|meta| meta.is_file()) {
            continue;
        }
        if let Err(e) = fs::remove_file(&backup) {
            eprintln!("warning: could not remove {}: {e}", backup.display());
            continue;
        }
        cleaned += 1;
        if let Some(parent) = backup.parent() {
            prune_empty_dirs(dest, parent);
        }
    }

    cleaned
}

/// Delete the files the previous bundle owned that this one dropped.
///
/// A file is removed only when all three of these hold:
///
/// 1. `previous` — what the last install left behind, either the manifest it
///    recorded or, for a pre-1.6.0 install, the frozen tree standing in for
///    the record it never wrote — lists it, so we are the ones who put it
///    there;
/// 2. `next` does not list it, so it is genuinely gone from the bundle;
/// 3. it still hashes to what `previous` recorded, so the user has not
///    touched it since we wrote it.
///
/// Anything else stays. A file we never owned is not ours to delete, and a
/// file the user edited is theirs even if we shipped it originally. A
/// removal that fails (permissions, a directory in the way) is a warning
/// rather than an error: the sync already succeeded, and leaving a stale
/// skill behind is not worth failing an otherwise-good install over.
///
/// Directories emptied by a removal are pruned, bounded by `dest`.
#[cfg(test)]
fn remove_orphans(dest: &Path, previous: &[ManifestEntry], next: &BundleManifest) -> Vec<String> {
    let kept = next.digests();
    let mut removed = Vec::new();

    for entry in previous {
        // An entry naming anything but a plain relative path under the
        // harness root is not something we wrote, and is never resolved to
        // a real path, let alone deleted.
        let (Some(key), Some(path)) = (
            manifest::relative_key(Path::new(&entry.path)),
            manifest::resolve_under(dest, &entry.path),
        ) else {
            continue;
        };
        if kept.contains_key(&key) || has_symlink_ancestor(dest, &path) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata_is_link(&metadata) || !metadata.is_file() {
            continue;
        }
        if integrity::verify_sha256(&path, &entry.sha256).is_err() {
            // Modified since we installed it (or unreadable) — the user's.
            continue;
        }
        if let Err(e) = fs::remove_file(&path) {
            eprintln!("warning: could not remove {}: {e}", path.display());
            continue;
        }
        if let Some(parent) = path.parent() {
            prune_empty_dirs(dest, parent);
        }
        removed.push(key);
    }

    removed
}

/// Remove directories left empty by an orphan deletion, walking up from
/// `from` and stopping at `dest`. `fs::remove_dir` refuses a non-empty
/// directory, so this can never take one that still holds anything —
/// including the user's own files.
fn prune_empty_dirs(dest: &Path, from: &Path) {
    let mut current = from;
    while current != dest && current.starts_with(dest) {
        if fs::remove_dir(current).is_err() {
            return;
        }
        let Some(parent) = current.parent() else {
            return;
        };
        current = parent;
    }
}

fn claude_staged_manifest(
    staged: &Path,
    pinned_version: Option<&str>,
    version: &str,
) -> Result<BundleManifest> {
    let manifest = match read_staged_manifest(staged)? {
        Some(manifest) => manifest,
        None => {
            if !AgentTool::Claude.is_installed_at(staged) {
                anyhow::bail!(
                    "Downloaded Claude Code bundle is incomplete — refusing to install. \
                     Run 'hyprlayer ai reinstall' to retry."
                );
            }
            manifest_for_tree(staged, version, AgentTool::Claude.harness_slug())?
        }
    };
    if manifest.harness != AgentTool::Claude.harness_slug() {
        anyhow::bail!(
            "Downloaded Claude Code bundle identifies itself as {:?}",
            manifest.harness
        );
    }
    if manifest.version != version {
        anyhow::bail!(
            "Downloaded Claude Code bundle is version {:?}, expected {version:?}",
            manifest.version
        );
    }
    verify_pin_is_supported(&manifest, pinned_version)?;
    verify_staged_completeness(AgentTool::Claude, staged, &manifest)?;
    Ok(manifest)
}

fn store_version_dir(store_root: &Path, version: &str) -> Result<PathBuf> {
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
        || manifest::relative_key(Path::new(version)).as_deref() != Some(version)
    {
        anyhow::bail!("Asset version {version:?} is not safe for the agent store");
    }
    Ok(store_root.join(version))
}

/// A no-symlink snapshot used both to compare a populated store generation
/// and to decide whether a legacy copied directory is wholly bundle-owned.
/// Directory entries are included so an extra empty personal directory is
/// not silently discarded during migration.
fn tree_snapshot(root: &Path) -> Result<BTreeMap<String, String>> {
    fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<()> {
        for entry in
            fs::read_dir(dir).with_context(|| format!("Failed to read {}", dir.display()))?
        {
            let path = entry
                .with_context(|| format!("Failed to read an entry in {}", dir.display()))?
                .path();
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("Failed to inspect {}", path.display()))?;
            let relative = path.strip_prefix(root).expect("walk stays below root");
            let key = manifest::relative_key(relative)
                .ok_or_else(|| anyhow::anyhow!("Agent bundle path is not a safe relative path"))?;
            if metadata_is_link(&metadata) {
                anyhow::bail!("Agent bundle contains a symlink at {}", path.display());
            } else if metadata.is_dir() {
                out.insert(key, "dir".to_string());
                visit(root, &path, out)?;
            } else if metadata.is_file() {
                let bytes = fs::read(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                out.insert(key, format!("file:{}", sha256_bytes(&bytes)));
            } else {
                anyhow::bail!(
                    "Agent bundle contains an unsupported entry at {}",
                    path.display()
                );
            }
        }
        Ok(())
    }

    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() && !metadata_is_link(&metadata) => {
            let mut out = BTreeMap::new();
            visit(root, root, &mut out)?;
            Ok(out)
        }
        Ok(_) => anyhow::bail!("Expected a regular directory at {}", root.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error).with_context(|| format!("Failed to inspect {}", root.display())),
    }
}

fn copy_regular_tree(source: &Path, dest: &Path) -> Result<usize> {
    fn visit(source_root: &Path, source: &Path, dest: &Path, count: &mut usize) -> Result<()> {
        for entry in
            fs::read_dir(source).with_context(|| format!("Failed to read {}", source.display()))?
        {
            let source_path = entry
                .with_context(|| format!("Failed to read an entry in {}", source.display()))?
                .path();
            let metadata = fs::symlink_metadata(&source_path)
                .with_context(|| format!("Failed to inspect {}", source_path.display()))?;
            let relative = source_path
                .strip_prefix(source_root)
                .expect("walk stays below source root");
            let dest_path = dest.join(relative);
            if metadata_is_link(&metadata) {
                anyhow::bail!("Refusing to copy bundle symlink {}", source_path.display());
            } else if metadata.is_dir() {
                fs::create_dir_all(&dest_path)
                    .with_context(|| format!("Failed to create {}", dest_path.display()))?;
                visit(source_root, &source_path, dest, count)?;
            } else if metadata.is_file() {
                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("Failed to create {}", parent.display()))?;
                }
                fs::copy(&source_path, &dest_path)
                    .with_context(|| format!("Failed to copy {}", source_path.display()))?;
                *count += 1;
            } else {
                anyhow::bail!("Unsupported bundle entry {}", source_path.display());
            }
        }
        Ok(())
    }

    fs::create_dir_all(dest).with_context(|| format!("Failed to create {}", dest.display()))?;
    let mut count = 0;
    visit(source, source, dest, &mut count)?;
    Ok(count)
}

fn remove_path_no_follow(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Failed to inspect {}", path.display()))?;
    if metadata_is_link(&metadata) {
        remove_link_no_follow(path, &metadata)
    } else if metadata.is_file() {
        fs::remove_file(path).with_context(|| format!("Failed to remove {}", path.display()))
    } else if metadata.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("Failed to remove {}", path.display()))
    } else {
        anyhow::bail!("Unsupported filesystem entry at {}", path.display())
    }
}

/// A complete Claude/Codex store generation prepared beside its final path.
/// The temporary path is removed unless activation renames it into place.
struct PreparedGeneration {
    generation: PathBuf,
    staged: Option<PathBuf>,
    claude_layout: PathBuf,
    codex_layout: PathBuf,
    changed: usize,
}

impl Drop for PreparedGeneration {
    fn drop(&mut self) {
        if let Some(path) = self.staged.take()
            && fs::symlink_metadata(&path).is_ok()
        {
            let _ = remove_path_no_follow(&path);
        }
    }
}

fn populate_prepared_bundle(
    staged: &Path,
    dest: &Path,
    manifest: &BundleManifest,
) -> Result<usize> {
    let mut count = copy_regular_tree(staged, dest)?;
    let manifest_path = dest.join(MANIFEST_FILE_NAME);
    if !manifest_path.is_file() {
        let body = serde_json::to_string_pretty(manifest)
            .context("Failed to serialize the agent-store manifest")?;
        fs::write(&manifest_path, body)
            .with_context(|| format!("Failed to write {}", manifest_path.display()))?;
        count += 1;
    }
    Ok(count)
}

/// Build both harnesses beneath one same-parent temporary directory. The
/// whole version directory is the replacement boundary, including when an
/// explicit reinstall replaces an existing generation with the same name.
fn prepare_store_generation(
    claude_staged: &Path,
    codex_staged: &Path,
    store_root: &Path,
    version: &str,
    claude_manifest: &BundleManifest,
    codex_manifest: &BundleManifest,
) -> Result<PreparedGeneration> {
    fs::create_dir_all(store_root)
        .with_context(|| format!("Failed to create agent store {}", store_root.display()))?;
    let temp = tempfile::Builder::new()
        .prefix(".hyprlayer-generation-")
        .tempdir_in(store_root)
        .context("Failed to stage the Claude/Codex store generation")?;
    let claude_layout = temp.path().join(AgentTool::Claude.harness_slug());
    let codex_layout = temp.path().join(CODEX_HARNESS);
    let mut changed = populate_prepared_bundle(claude_staged, &claude_layout, claude_manifest)?;
    changed += populate_prepared_bundle(codex_staged, &codex_layout, codex_manifest)?;

    let generation = store_version_dir(store_root, version)?;
    let prepared_snapshot = tree_snapshot(temp.path())?;
    if tree_snapshot(&generation).is_ok_and(|current| current == prepared_snapshot) {
        return Ok(PreparedGeneration {
            claude_layout: generation.join(AgentTool::Claude.harness_slug()),
            codex_layout: generation.join(CODEX_HARNESS),
            generation,
            staged: None,
            changed: 0,
        });
    }

    let staged = temp.keep();
    Ok(PreparedGeneration {
        generation,
        claude_layout,
        codex_layout,
        staged: Some(staged),
        changed,
    })
}

fn known_bundle_digests<'a>(
    manifests: impl IntoIterator<Item = &'a [ManifestEntry]>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut known: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entries in manifests {
        for entry in entries {
            let Some(key) = manifest::relative_key(Path::new(&entry.path)) else {
                continue;
            };
            known.entry(key).or_default().insert(entry.sha256.clone());
        }
    }
    known
}

fn legacy_entry_is_owned(
    path: &Path,
    logical_path: &Path,
    known: &BTreeMap<String, BTreeSet<String>>,
) -> Result<bool> {
    fn visit(
        path: &Path,
        logical_path: &Path,
        known: &BTreeMap<String, BTreeSet<String>>,
    ) -> Result<Option<usize>> {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("Failed to inspect {}", path.display()))?;
        if metadata_is_link(&metadata) {
            return Ok(None);
        }
        if metadata.is_file() {
            let Some(key) = manifest::relative_key(logical_path) else {
                return Ok(None);
            };
            let bytes =
                fs::read(path).with_context(|| format!("Failed to read {}", path.display()))?;
            return Ok(known
                .get(&key)
                .is_some_and(|digests| digests.contains(&sha256_bytes(&bytes)))
                .then_some(1));
        }
        if !metadata.is_dir() {
            return Ok(None);
        }

        let prefix = manifest::relative_key(logical_path)
            .map(|key| format!("{key}/"))
            .unwrap_or_default();
        if !known.keys().any(|key| key.starts_with(&prefix)) {
            return Ok(None);
        }
        let mut matched_files = 0;
        for entry in
            fs::read_dir(path).with_context(|| format!("Failed to read {}", path.display()))?
        {
            let entry =
                entry.with_context(|| format!("Failed to read an entry in {}", path.display()))?;
            let Some(child_files) =
                visit(&entry.path(), &logical_path.join(entry.file_name()), known)?
            else {
                return Ok(None);
            };
            matched_files += child_files;
        }
        // An empty directory proves no shipped digest. It may be a user's
        // placeholder at a bundled name and must never be adopted.
        Ok((matched_files > 0).then_some(matched_files))
    }

    Ok(visit(path, logical_path, known)?.is_some())
}

fn absolute_link_target(link: &Path) -> Result<PathBuf> {
    let target =
        fs::read_link(link).with_context(|| format!("Failed to read link {}", link.display()))?;
    if target.is_absolute() {
        Ok(target)
    } else {
        Ok(link.parent().unwrap_or_else(|| Path::new(".")).join(target))
    }
}

fn link_targets_store(link: &Path, store_root: &Path) -> bool {
    if let (Ok(target), Ok(store)) = (fs::canonicalize(link), fs::canonicalize(store_root)) {
        return target.starts_with(store);
    }
    // Hyprlayer creates absolute, parent-free targets. This also recognizes
    // its dangling links after a store generation was removed, without
    // treating `store/../personal` as owned.
    absolute_link_target(link).is_ok_and(|target| {
        target.is_absolute()
            && !target
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            && target.starts_with(store_root)
    })
}

fn link_targets_exactly(link: &Path, target: &Path) -> bool {
    absolute_link_target(link).is_ok_and(|actual| actual == target)
        || matches!(
            (fs::canonicalize(link), fs::canonicalize(target)),
            (Ok(actual), Ok(expected)) if actual == expected
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedLinkKind {
    File,
    Directory,
}

fn managed_link_kind(path: &Path) -> Result<ManagedLinkKind> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Failed to inspect link target {}", path.display()))?;
    if metadata_is_link(&metadata) {
        anyhow::bail!("Agent store contains a link at {}", path.display());
    }
    if metadata.is_dir() {
        Ok(ManagedLinkKind::Directory)
    } else if metadata.is_file() {
        Ok(ManagedLinkKind::File)
    } else {
        anyhow::bail!("Unsupported agent-store entry at {}", path.display())
    }
}

#[cfg(any(windows, test))]
fn windows_file_symlink_error(error: &std::io::Error, parent: &Path) -> String {
    if error.raw_os_error() == Some(1314) {
        format!(
            "Windows could not create the required agent file symlink in {} (error 1314). \
             Enable Windows Developer Mode or run hyprlayer from an elevated terminal, then retry. \
             No agent links were changed.",
            parent.display()
        )
    } else {
        format!(
            "Windows could not create an agent file symlink in {}: {error}",
            parent.display()
        )
    }
}

fn create_managed_link(target: &Path, link: &Path, kind: ManagedLinkKind) -> Result<()> {
    #[cfg(unix)]
    {
        let _ = kind;
        std::os::unix::fs::symlink(target, link)
            .with_context(|| format!("Failed to link {} to {}", link.display(), target.display()))
    }

    #[cfg(windows)]
    {
        let parent = link
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Link path has no parent: {}", link.display()))?;
        match kind {
            ManagedLinkKind::File => std::os::windows::fs::symlink_file(target, link)
                .map_err(|error| anyhow::anyhow!(windows_file_symlink_error(&error, parent)))
                .with_context(|| {
                    format!("Failed to link {} to {}", link.display(), target.display())
                }),
            ManagedLinkKind::Directory => match std::os::windows::fs::symlink_dir(target, link) {
                Ok(()) => Ok(()),
                Err(error) if error.raw_os_error() == Some(1314) => junction::create(target, link)
                    .with_context(|| {
                        format!(
                            "Failed to create directory junction {} -> {}",
                            link.display(),
                            target.display()
                        )
                    }),
                Err(error) => Err(error).with_context(|| {
                    format!("Failed to link {} to {}", link.display(), target.display())
                }),
            },
        }
    }
}

fn remove_managed_link(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Failed to inspect link {}", path.display()))?;
    if !metadata_is_link(&metadata) {
        anyhow::bail!("Refusing to unlink non-link path {}", path.display());
    }
    remove_link_no_follow(path, &metadata)
}

struct PathMutation {
    live: PathBuf,
    backup_root: Option<PathBuf>,
    installed: bool,
}

#[derive(Default)]
struct ActivationTransaction {
    mutations: Vec<PathMutation>,
    created_dirs: Vec<PathBuf>,
}

impl ActivationTransaction {
    fn ensure_dir(&mut self, path: &Path) -> Result<()> {
        if fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
            return Ok(());
        }

        let mut missing = Vec::new();
        let mut current = path;
        loop {
            match fs::symlink_metadata(current) {
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing.push(current.to_path_buf());
                    current = current.parent().ok_or_else(|| {
                        anyhow::anyhow!("Directory has no existing ancestor: {}", path.display())
                    })?;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to inspect {}", current.display()));
                }
            }
        }
        fs::create_dir_all(path).with_context(|| format!("Failed to create {}", path.display()))?;
        if !fs::metadata(path).is_ok_and(|metadata| metadata.is_dir()) {
            anyhow::bail!("Expected a directory at {}", path.display());
        }
        missing.reverse();
        self.created_dirs.extend(missing);
        Ok(())
    }

    fn backup_existing(&self, live: &Path) -> Result<Option<PathBuf>> {
        match fs::symlink_metadata(live) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to inspect {}", live.display()));
            }
        }
        let parent = live
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Activation path has no parent: {}", live.display()))?;
        let backup_root = tempfile::Builder::new()
            .prefix(".hyprlayer-activation-")
            .tempdir_in(parent)
            .with_context(|| format!("Failed to stage rollback beside {}", live.display()))?
            .keep();
        let backup = backup_root.join("previous");
        if let Err(error) = fs::rename(live, &backup) {
            let _ = fs::remove_dir(&backup_root);
            return Err(error)
                .with_context(|| format!("Failed to stage rollback for {}", live.display()));
        }
        Ok(Some(backup_root))
    }

    fn restore_immediately(live: &Path, backup_root: Option<&Path>) -> Result<()> {
        if let Some(root) = backup_root {
            fs::rename(root.join("previous"), live)
                .with_context(|| format!("Failed to restore {}", live.display()))?;
            fs::remove_dir(root)
                .with_context(|| format!("Failed to clear rollback area {}", root.display()))?;
        }
        Ok(())
    }

    fn replace_with_link(
        &mut self,
        live: &Path,
        target: &Path,
        kind: ManagedLinkKind,
    ) -> Result<()> {
        let backup_root = self.backup_existing(live)?;
        if let Err(error) = create_managed_link(target, live, kind) {
            if let Err(rollback) = Self::restore_immediately(live, backup_root.as_deref()) {
                return Err(error.context(format!(
                    "The link failed and restoring the previous entry also failed: {rollback:#}"
                )));
            }
            return Err(error);
        }
        self.mutations.push(PathMutation {
            live: live.to_path_buf(),
            backup_root,
            installed: true,
        });
        Ok(())
    }

    fn replace_with_path(&mut self, staged: &Path, live: &Path) -> Result<()> {
        let backup_root = self.backup_existing(live)?;
        if let Err(error) = fs::rename(staged, live) {
            if let Err(rollback) = Self::restore_immediately(live, backup_root.as_deref()) {
                return Err(anyhow::Error::new(error).context(format!(
                    "Installing {} failed and restoring it also failed: {rollback:#}",
                    live.display()
                )));
            }
            return Err(error).with_context(|| format!("Failed to install {}", live.display()));
        }
        self.mutations.push(PathMutation {
            live: live.to_path_buf(),
            backup_root,
            installed: true,
        });
        Ok(())
    }

    fn replace_with_bytes(&mut self, live: &Path, bytes: &[u8]) -> Result<()> {
        let parent = live
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Activation file has no parent: {}", live.display()))?;
        self.ensure_dir(parent)?;
        let staged = tempfile::Builder::new()
            .prefix(".hyprlayer-file-")
            .tempdir_in(parent)
            .with_context(|| format!("Failed to stage {}", live.display()))?;
        let staged_file = staged.path().join("replacement");
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staged_file)
            .with_context(|| format!("Failed to stage {}", live.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("Failed to stage {}", live.display()))?;
        file.sync_all()
            .with_context(|| format!("Failed to sync {}", live.display()))?;
        drop(file);
        self.replace_with_path(&staged_file, live)
    }

    fn remove_path(&mut self, live: &Path) -> Result<()> {
        let Some(backup_root) = self.backup_existing(live)? else {
            return Ok(());
        };
        self.mutations.push(PathMutation {
            live: live.to_path_buf(),
            backup_root: Some(backup_root),
            installed: false,
        });
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for mutation in self.mutations.iter().rev() {
            if mutation.installed {
                match fs::symlink_metadata(&mutation.live) {
                    Ok(_) => {
                        if let Err(error) = remove_path_no_follow(&mutation.live) {
                            errors.push(format!("remove {}: {error:#}", mutation.live.display()));
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        errors.push(format!("inspect {}: {error}", mutation.live.display()))
                    }
                }
            }
            if let Some(root) = &mutation.backup_root {
                let backup = root.join("previous");
                if fs::symlink_metadata(&backup).is_ok()
                    && let Err(error) = fs::rename(&backup, &mutation.live)
                {
                    errors.push(format!("restore {}: {error}", mutation.live.display()));
                }
                if let Err(error) = fs::remove_dir(root) {
                    errors.push(format!("clear {}: {error}", root.display()));
                }
            }
        }
        self.mutations.clear();
        for dir in self.created_dirs.iter().rev() {
            match fs::remove_dir(dir) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => {}
                Err(error) => errors.push(format!("remove {}: {error}", dir.display())),
            }
        }
        self.created_dirs.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            anyhow::bail!("activation rollback was incomplete: {}", errors.join("; "))
        }
    }

    fn commit(mut self) {
        for mutation in self.mutations.drain(..).rev() {
            let Some(root) = mutation.backup_root else {
                continue;
            };
            let backup = root.join("previous");
            if fs::symlink_metadata(&backup).is_ok()
                && let Err(error) = remove_path_no_follow(&backup)
            {
                eprintln!(
                    "warning: could not clear old activation entry {}: {error}",
                    backup.display()
                );
                continue;
            }
            if let Err(error) = fs::remove_dir(&root) {
                eprintln!(
                    "warning: could not clear activation directory {}: {error}",
                    root.display()
                );
            }
        }
    }
}

#[derive(Debug)]
enum FarmChange {
    Link {
        live: PathBuf,
        target: PathBuf,
        preflight_target: PathBuf,
        kind: ManagedLinkKind,
    },
    Remove {
        live: PathBuf,
        logical: String,
    },
}

#[derive(Debug)]
struct FarmPlan {
    dest: PathBuf,
    active: bool,
    changes: Vec<FarmChange>,
    preserved: Vec<String>,
}

fn plan_link_farm(
    layout_source: Option<&Path>,
    target_source: Option<&Path>,
    dest: &Path,
    logical_prefix: &str,
    store_root: &Path,
    known: &BTreeMap<String, BTreeSet<String>>,
) -> Result<FarmPlan> {
    let mut desired = BTreeMap::new();
    if let (Some(layout_source), Some(target_source)) = (layout_source, target_source) {
        for entry in fs::read_dir(layout_source)
            .with_context(|| format!("Failed to read agent store {}", layout_source.display()))?
        {
            let entry = entry.with_context(|| {
                format!("Failed to read an entry in {}", layout_source.display())
            })?;
            let name = entry.file_name().into_string().map_err(|_| {
                anyhow::anyhow!(
                    "Agent-store entry is not valid UTF-8: {}",
                    entry.path().display()
                )
            })?;
            let kind = managed_link_kind(&entry.path())?;
            desired.insert(
                name.clone(),
                (target_source.join(&name), entry.path(), kind),
            );
        }
    }

    if desired.is_empty() && fs::symlink_metadata(dest).is_err() {
        return Ok(FarmPlan {
            dest: dest.to_path_buf(),
            active: false,
            changes: Vec::new(),
            preserved: Vec::new(),
        });
    }
    let active = match fs::symlink_metadata(dest) {
        Ok(metadata) if metadata.is_dir() && !metadata_is_link(&metadata) => true,
        Ok(_) => {
            return Ok(FarmPlan {
                dest: dest.to_path_buf(),
                active: false,
                changes: Vec::new(),
                preserved: vec![logical_prefix.to_string()],
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to inspect {}", dest.display()));
        }
    };

    let mut changes = Vec::new();
    let mut preserved = Vec::new();
    for (name, (target, preflight_target, kind)) in &desired {
        let link = dest.join(name);
        let logical = Path::new(logical_prefix).join(name);
        match fs::symlink_metadata(&link) {
            Ok(metadata) if metadata_is_link(&metadata) => {
                if !link_targets_store(&link, store_root) {
                    preserved.push(logical.display().to_string());
                } else if !link_targets_exactly(&link, target) {
                    changes.push(FarmChange::Link {
                        live: link,
                        target: target.clone(),
                        preflight_target: preflight_target.clone(),
                        kind: *kind,
                    });
                }
            }
            Ok(_) => {
                if legacy_entry_is_owned(&link, &logical, known)? {
                    changes.push(FarmChange::Link {
                        live: link,
                        target: target.clone(),
                        preflight_target: preflight_target.clone(),
                        kind: *kind,
                    });
                } else {
                    preserved.push(logical.display().to_string());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                changes.push(FarmChange::Link {
                    live: link,
                    target: target.clone(),
                    preflight_target: preflight_target.clone(),
                    kind: *kind,
                });
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to inspect {}", link.display()));
            }
        }
    }

    if fs::symlink_metadata(dest).is_ok() {
        for entry in
            fs::read_dir(dest).with_context(|| format!("Failed to read {}", dest.display()))?
        {
            let entry =
                entry.with_context(|| format!("Failed to read an entry in {}", dest.display()))?;
            let Ok(name) = entry.file_name().into_string() else {
                // Mixed namespace: an unrelated personal entry can use any
                // filename the platform supports. It is never ours to reject.
                continue;
            };
            if desired.contains_key(&name) {
                continue;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("Failed to inspect {}", path.display()))?;
            if metadata_is_link(&metadata) && link_targets_store(&path, store_root) {
                changes.push(FarmChange::Remove {
                    live: path,
                    logical: format!("{logical_prefix}/{name}"),
                });
            }
        }
    }

    preserved.sort();
    Ok(FarmPlan {
        dest: dest.to_path_buf(),
        active,
        changes,
        preserved,
    })
}

fn allocate_preflight_path(parent: &Path) -> Result<PathBuf> {
    for attempt in 0..100u32 {
        let candidate = parent.join(format!(
            ".hyprlayer-link-preflight-{}-{attempt}",
            std::process::id()
        ));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(_) => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to inspect {}", candidate.display()));
            }
        }
    }
    anyhow::bail!(
        "Could not allocate a link preflight path inside {}",
        parent.display()
    )
}

fn preflight_managed_link(target: &Path, parent: &Path, kind: ManagedLinkKind) -> Result<()> {
    let candidate = allocate_preflight_path(parent)?;
    create_managed_link(target, &candidate, kind)?;
    remove_managed_link(&candidate)
        .context("The agent-link preflight succeeded but its temporary link could not be removed")
}

fn preflight_farm(plan: &FarmPlan) -> Result<()> {
    if !plan.active {
        return Ok(());
    }
    let mut file_done = false;
    let mut directory_done = false;
    for change in &plan.changes {
        let FarmChange::Link {
            preflight_target,
            kind,
            ..
        } = change
        else {
            continue;
        };
        let already_done = match kind {
            ManagedLinkKind::File => &mut file_done,
            ManagedLinkKind::Directory => &mut directory_done,
        };
        if *already_done {
            continue;
        }
        preflight_managed_link(preflight_target, &plan.dest, *kind)?;
        *already_done = true;
    }
    Ok(())
}

fn apply_farm(plan: &FarmPlan, transaction: &mut ActivationTransaction) -> Result<SyncReport> {
    let mut report = SyncReport {
        preserved: plan.preserved.clone(),
        ..SyncReport::default()
    };
    if !plan.active {
        return Ok(report);
    }
    for change in &plan.changes {
        match change {
            FarmChange::Link {
                live, target, kind, ..
            } => transaction.replace_with_link(live, target, *kind)?,
            FarmChange::Remove { live, logical } => {
                transaction.remove_path(live)?;
                report.removed.push(logical.clone());
            }
        }
        report.changed += 1;
    }
    report.removed.sort();
    Ok(report)
}

#[cfg(test)]
fn reconcile_link_farm(
    source: Option<&Path>,
    dest: &Path,
    logical_prefix: &str,
    store_root: &Path,
    known: &BTreeMap<String, BTreeSet<String>>,
) -> Result<SyncReport> {
    let plan = plan_link_farm(source, source, dest, logical_prefix, store_root, known)?;
    let mut transaction = ActivationTransaction::default();
    if plan.active {
        transaction.ensure_dir(dest)?;
    }
    let mut report = match preflight_farm(&plan).and_then(|()| apply_farm(&plan, &mut transaction))
    {
        Ok(report) => report,
        Err(error) => {
            if let Err(rollback) = transaction.rollback() {
                return Err(error.context(format!("Link-farm rollback also failed: {rollback:#}")));
            }
            return Err(error);
        }
    };
    report.removed.sort();
    transaction.commit();
    Ok(report)
}

fn merge_sync_report(into: &mut SyncReport, mut from: SyncReport) {
    into.changed += from.changed;
    into.preserved.append(&mut from.preserved);
    into.removed.append(&mut from.removed);
    into.cleaned_backups += from.cleaned_backups;
}

fn has_symlink_ancestor(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    let mut current = root.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        let std::path::Component::Normal(part) = component else {
            return true;
        };
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata_is_link(&metadata) => return true,
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
            Err(_) => return true,
        }
    }
    false
}

fn metadata_is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        metadata.file_type().is_symlink()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
}

fn remove_link_no_follow(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        let _ = metadata;
        fs::remove_file(path).with_context(|| format!("Failed to unlink {}", path.display()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
        if metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0 {
            fs::remove_dir(path).with_context(|| format!("Failed to unlink {}", path.display()))
        } else {
            fs::remove_file(path).with_context(|| format!("Failed to unlink {}", path.display()))
        }
    }
}

/// Remove only digest-matched legacy *copies*. Symlinks are deliberately
/// excluded: link-farm ownership is decided by their target instead.
fn remove_legacy_copies(
    dest: &Path,
    previous: &[ManifestEntry],
    next: Option<&BundleManifest>,
) -> Vec<String> {
    let kept = next.map(BundleManifest::digests).unwrap_or_default();
    let mut removed = Vec::new();
    for entry in previous {
        let (Some(key), Some(path)) = (
            manifest::relative_key(Path::new(&entry.path)),
            manifest::resolve_under(dest, &entry.path),
        ) else {
            continue;
        };
        if kept.contains_key(&key) || has_symlink_ancestor(dest, &path) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata_is_link(&metadata) || !metadata.is_file() {
            continue;
        }
        if integrity::verify_sha256(&path, &entry.sha256).is_err() {
            continue;
        }
        if let Err(error) = fs::remove_file(&path) {
            eprintln!("warning: could not remove {}: {error}", path.display());
            continue;
        }
        if let Some(parent) = path.parent() {
            prune_empty_dirs(dest, parent);
        }
        removed.push(key);
    }
    removed
}

fn root_only_manifest(manifest: &BundleManifest) -> BundleManifest {
    BundleManifest {
        version: manifest.version.clone(),
        harness: manifest.harness.clone(),
        min_cli_version: manifest.min_cli_version.clone(),
        files: manifest
            .files
            .iter()
            .filter(|entry| is_harness_config(Path::new(&entry.path)))
            .cloned()
            .collect(),
    }
}

struct RootFilesPlan {
    replacements: Vec<(PathBuf, Vec<u8>)>,
    preserved: Vec<String>,
}

fn plan_claude_root_files(
    source: &Path,
    dest: &Path,
    current: &BundleManifest,
    previous: Option<&BundleManifest>,
) -> Result<RootFilesPlan> {
    // `settings.json` is the deliberate store-mode exception on every OS: it is a
    // mutable harness config, so it stays a digest-guarded regular copy
    // rather than a link to immutable bundle defaults.
    let previous_digests = previous.map(BundleManifest::digests);
    let mut replacements = Vec::new();
    let mut preserved = Vec::new();
    let root = root_only_manifest(current);

    for entry in &root.files {
        let source_path = manifest::resolve_under(source, &entry.path)
            .ok_or_else(|| anyhow::anyhow!("Invalid root bundle path {:?}", entry.path))?;
        let dest_path = manifest::resolve_under(dest, &entry.path)
            .ok_or_else(|| anyhow::anyhow!("Invalid root destination path {:?}", entry.path))?;
        let bytes = fs::read(&source_path)
            .with_context(|| format!("Failed to read {}", source_path.display()))?;
        match fs::symlink_metadata(&dest_path) {
            Ok(metadata) if metadata.is_file() && !metadata_is_link(&metadata) => {
                let existing = fs::read(&dest_path)
                    .with_context(|| format!("Failed to read {}", dest_path.display()))?;
                if existing == bytes {
                    continue;
                }
                let ours = previous_digests.as_ref().is_some_and(|digests| {
                    digests
                        .get(&entry.path)
                        .is_some_and(|digest| integrity::bytes_match_sha256(&existing, digest))
                });
                if !ours {
                    preserved.push(entry.path.clone());
                    continue;
                }
            }
            Ok(_) => {
                preserved.push(entry.path.clone());
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to inspect {}", dest_path.display()));
            }
        }
        replacements.push((dest_path, bytes));
    }
    Ok(RootFilesPlan {
        replacements,
        preserved,
    })
}

fn apply_claude_root_files(
    plan: &RootFilesPlan,
    transaction: &mut ActivationTransaction,
) -> Result<SyncReport> {
    let mut report = SyncReport {
        preserved: plan.preserved.clone(),
        ..SyncReport::default()
    };
    for (path, bytes) in &plan.replacements {
        transaction.replace_with_bytes(path, bytes)?;
        report.changed += 1;
    }
    Ok(report)
}

#[cfg(test)]
fn sync_claude_root_files(
    source: &Path,
    dest: &Path,
    current: &BundleManifest,
    previous: Option<&BundleManifest>,
) -> Result<SyncReport> {
    let plan = plan_claude_root_files(source, dest, current, previous)?;
    let mut transaction = ActivationTransaction::default();
    let report = match apply_claude_root_files(&plan, &mut transaction) {
        Ok(report) => report,
        Err(error) => {
            if let Err(rollback) = transaction.rollback() {
                return Err(error.context(format!("Settings rollback also failed: {rollback:#}")));
            }
            return Err(error);
        }
    };
    transaction.commit();
    Ok(report)
}

fn paths_resolve_same(a: &Path, b: &Path) -> bool {
    matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

struct SkillBridgePlan {
    source: PathBuf,
    link: PathBuf,
    preflight_target: PathBuf,
    outcome: SkillBridgeOutcome,
}

fn plan_skill_bridge(
    source: &Path,
    link: &Path,
    preflight_target: &Path,
) -> Result<SkillBridgePlan> {
    let outcome = match fs::symlink_metadata(link) {
        Ok(_) if paths_resolve_same(link, source) || link_targets_exactly(link, source) => {
            SkillBridgeOutcome::AlreadyCorrect
        }
        Ok(_) => SkillBridgeOutcome::SkippedExisting,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SkillBridgeOutcome::Created,
        Err(error) => {
            return Err(error).with_context(|| format!("Failed to inspect {}", link.display()));
        }
    };
    Ok(SkillBridgePlan {
        source: source.to_path_buf(),
        link: link.to_path_buf(),
        preflight_target: preflight_target.to_path_buf(),
        outcome,
    })
}

fn preflight_skill_bridge(plan: &SkillBridgePlan) -> Result<()> {
    if plan.outcome != SkillBridgeOutcome::Created {
        return Ok(());
    }
    let parent = plan
        .link
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Codex skill bridge has no parent"))?;
    preflight_managed_link(&plan.preflight_target, parent, ManagedLinkKind::Directory)
}

fn apply_skill_bridge(
    plan: &SkillBridgePlan,
    transaction: &mut ActivationTransaction,
) -> Result<SkillBridgeOutcome> {
    if plan.outcome != SkillBridgeOutcome::Created {
        return Ok(plan.outcome);
    }
    if !plan.source.is_dir() {
        anyhow::bail!(
            "Codex skill bridge source does not exist: {}",
            plan.source.display()
        );
    }
    transaction.replace_with_link(&plan.link, &plan.source, ManagedLinkKind::Directory)?;
    Ok(SkillBridgeOutcome::Created)
}

#[cfg(test)]
fn ensure_skill_bridge(source: &Path, link: &Path) -> Result<SkillBridgeOutcome> {
    let plan = plan_skill_bridge(source, link, source)?;
    let mut transaction = ActivationTransaction::default();
    if plan.outcome == SkillBridgeOutcome::Created {
        transaction.ensure_dir(
            link.parent()
                .ok_or_else(|| anyhow::anyhow!("Codex skill bridge has no parent"))?,
        )?;
    }
    let outcome = match preflight_skill_bridge(&plan)
        .and_then(|()| apply_skill_bridge(&plan, &mut transaction))
    {
        Ok(outcome) => outcome,
        Err(error) => {
            if let Err(rollback) = transaction.rollback() {
                return Err(
                    error.context(format!("Skill bridge rollback also failed: {rollback:#}"))
                );
            }
            return Err(error);
        }
    };
    transaction.commit();
    Ok(outcome)
}

fn warn_if_bridge_skipped(outcome: SkillBridgeOutcome, source: &Path, link: &Path) {
    if outcome == SkillBridgeOutcome::SkippedExisting {
        eprintln!(
            "warning: left existing {} untouched; standalone Codex skills remain unbridged to {}",
            link.display(),
            source.display()
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationBoundary {
    Generation,
    ClaudeSkills,
    ClaudeAgents,
    CodexAgents,
    Bridge,
    Settings,
    Manifest,
}

fn inject_activation_failure(
    requested: Option<ActivationBoundary>,
    boundary: ActivationBoundary,
) -> Result<()> {
    if requested == Some(boundary) {
        anyhow::bail!("injected activation failure after {boundary:?}");
    }
    Ok(())
}

fn file_needs_replacement(path: &Path, bytes: &[u8]) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata_is_link(&metadata) => Ok(fs::read(path)
            .with_context(|| format!("Failed to read {}", path.display()))?
            != bytes),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

/// Install the same-version Claude/Codex pair through one rollback log.
/// Bundle bytes first become one immutable store generation; the generation,
/// all three mixed-namespace link farms, the skill bridge, mutable
/// `settings.json`, and the ownership record are then committed together.
#[allow(clippy::too_many_arguments)]
fn install_claude_bundle_set(
    claude_staged: &Path,
    codex_staged: &Path,
    claude_dest: &Path,
    codex_dest: &Path,
    store_root: &Path,
    bridge: &Path,
    pinned_version: Option<&str>,
    version: &str,
    quiet: bool,
) -> Result<SyncReport> {
    install_claude_bundle_set_with_failure(
        claude_staged,
        codex_staged,
        claude_dest,
        codex_dest,
        store_root,
        bridge,
        pinned_version,
        version,
        quiet,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn install_claude_bundle_set_with_failure(
    claude_staged: &Path,
    codex_staged: &Path,
    claude_dest: &Path,
    codex_dest: &Path,
    store_root: &Path,
    bridge: &Path,
    pinned_version: Option<&str>,
    version: &str,
    quiet: bool,
    failure: Option<ActivationBoundary>,
) -> Result<SyncReport> {
    // These gates all run before the first destination mutation.
    let claude_manifest = claude_staged_manifest(claude_staged, pinned_version, version)?;
    let codex_manifest = codex_staged_manifest(codex_staged, pinned_version, version)?;
    let previous_claude = read_installed_manifest(claude_dest);
    let previous_codex = read_installed_manifest(codex_dest);
    let frozen = AgentTool::Claude.frozen_manifest();
    let mut claude_known_sources: Vec<&[ManifestEntry]> =
        vec![claude_manifest.files.as_slice(), frozen.as_slice()];
    if let Some(previous) = &previous_claude {
        claude_known_sources.push(previous.files.as_slice());
    }
    let claude_known = known_bundle_digests(claude_known_sources);
    let mut codex_known_sources: Vec<&[ManifestEntry]> = vec![codex_manifest.files.as_slice()];
    if let Some(previous) = &previous_codex {
        codex_known_sources.push(previous.files.as_slice());
    }
    let codex_known = known_bundle_digests(codex_known_sources);

    let prepared = prepare_store_generation(
        claude_staged,
        codex_staged,
        store_root,
        version,
        &claude_manifest,
        &codex_manifest,
    )?;
    let claude_store = prepared.generation.join(AgentTool::Claude.harness_slug());
    let codex_store = prepared.generation.join(CODEX_HARNESS);
    let claude_skills = plan_link_farm(
        Some(&prepared.claude_layout.join("skills")),
        Some(&claude_store.join("skills")),
        &claude_dest.join("skills"),
        "skills",
        store_root,
        &claude_known,
    )?;
    let claude_agents = plan_link_farm(
        Some(&prepared.claude_layout.join("agents")),
        Some(&claude_store.join("agents")),
        &claude_dest.join("agents"),
        "agents",
        store_root,
        &claude_known,
    )?;
    let codex_agents = plan_link_farm(
        Some(&prepared.codex_layout.join("agents")),
        Some(&codex_store.join("agents")),
        &codex_dest.join("agents"),
        "agents",
        store_root,
        &codex_known,
    )?;
    let root_files = plan_claude_root_files(
        &prepared.claude_layout,
        claude_dest,
        &claude_manifest,
        previous_claude.as_ref(),
    )?;
    let bridge_source = claude_dest.join("skills");
    let bridge_plan = plan_skill_bridge(
        &bridge_source,
        bridge,
        &prepared.claude_layout.join("skills"),
    )?;
    let root_manifest = root_only_manifest(&claude_manifest);
    let root_manifest_bytes = serde_json::to_string_pretty(&root_manifest)
        .context("Failed to serialize the bundle manifest")?;
    let claude_record = claude_dest.join(INSTALLED_MANIFEST_FILE);
    let replace_claude_record =
        file_needs_replacement(&claude_record, root_manifest_bytes.as_bytes())?;
    let codex_record = codex_dest.join(INSTALLED_MANIFEST_FILE);
    let remove_codex_record = fs::symlink_metadata(&codex_record)
        .is_ok_and(|metadata| metadata.is_file() && !metadata_is_link(&metadata));

    let mut transaction = ActivationTransaction::default();
    let activation = (|| -> Result<(SyncReport, SkillBridgeOutcome)> {
        for plan in [&claude_skills, &claude_agents, &codex_agents] {
            if plan.active {
                transaction.ensure_dir(&plan.dest)?;
            }
        }
        if bridge_plan.outcome == SkillBridgeOutcome::Created {
            transaction.ensure_dir(
                bridge
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Codex skill bridge has no parent"))?,
            )?;
        }
        preflight_farm(&claude_skills)?;
        preflight_farm(&claude_agents)?;
        preflight_farm(&codex_agents)?;
        preflight_skill_bridge(&bridge_plan)?;

        if let Some(staged) = prepared.staged.as_deref() {
            transaction.replace_with_path(staged, &prepared.generation)?;
        }
        inject_activation_failure(failure, ActivationBoundary::Generation)?;

        let mut report = SyncReport {
            changed: prepared.changed,
            ..SyncReport::default()
        };
        merge_sync_report(&mut report, apply_farm(&claude_skills, &mut transaction)?);
        inject_activation_failure(failure, ActivationBoundary::ClaudeSkills)?;
        merge_sync_report(&mut report, apply_farm(&claude_agents, &mut transaction)?);
        inject_activation_failure(failure, ActivationBoundary::ClaudeAgents)?;
        merge_sync_report(&mut report, apply_farm(&codex_agents, &mut transaction)?);
        inject_activation_failure(failure, ActivationBoundary::CodexAgents)?;

        let bridge_outcome = apply_skill_bridge(&bridge_plan, &mut transaction)?;
        if bridge_outcome == SkillBridgeOutcome::Created {
            report.changed += 1;
        }
        inject_activation_failure(failure, ActivationBoundary::Bridge)?;

        merge_sync_report(
            &mut report,
            apply_claude_root_files(&root_files, &mut transaction)?,
        );
        inject_activation_failure(failure, ActivationBoundary::Settings)?;

        if replace_claude_record {
            transaction.replace_with_bytes(&claude_record, root_manifest_bytes.as_bytes())?;
            report.changed += 1;
        }
        if remove_codex_record {
            transaction.remove_path(&codex_record)?;
            report.changed += 1;
        }
        inject_activation_failure(failure, ActivationBoundary::Manifest)?;
        Ok((report, bridge_outcome))
    })();

    let (mut report, bridge_outcome) = match activation {
        Ok(success) => success,
        Err(error) => {
            if let Err(rollback) = transaction.rollback() {
                return Err(error.context(format!(
                    "Claude/Codex activation failed and rollback was incomplete: {rollback:#}"
                )));
            }
            return Err(error);
        }
    };
    transaction.commit();

    // Cleanup is deliberately after the transactional commit and is always
    // best-effort. It cannot turn a successful activation into a reported
    // failure, nor can it remove bytes needed to roll an activation back.
    report.cleaned_backups += clean_backups(claude_dest, &frozen);
    if let Some(previous) = &previous_claude {
        report.cleaned_backups += clean_backups(claude_dest, &previous.files);
        report.removed.extend(remove_legacy_copies(
            claude_dest,
            &previous.files,
            Some(&claude_manifest),
        ));
    }
    report.removed.extend(remove_legacy_copies(
        claude_dest,
        &frozen,
        Some(&claude_manifest),
    ));
    if let Some(previous) = &previous_codex {
        report.removed.extend(remove_legacy_copies(
            codex_dest,
            &previous.files,
            Some(&codex_manifest),
        ));
    }

    warn_if_bridge_skipped(bridge_outcome, &bridge_source, bridge);
    if bridge_outcome == SkillBridgeOutcome::Created && !quiet {
        println!(
            "  {:<60}",
            format!(
                "Linked standalone Codex skills {} -> {}",
                bridge.display(),
                bridge_source.display()
            )
        );
    }

    report.preserved.sort();
    report.preserved.dedup();
    report.removed.sort();
    report.removed.dedup();
    Ok(report)
}

struct StoredBundleSet {
    claude: PathBuf,
    codex: PathBuf,
    claude_manifest: BundleManifest,
    codex_manifest: BundleManifest,
}

/// Resolve a complete, verified store generation without mutating it. A
/// missing, partial, or corrupt generation is a cache miss, so callers can
/// fall back to fetching the pair.
fn stored_bundle_set(version: &str) -> Result<Option<StoredBundleSet>> {
    let store_root = agent_store_root()?;
    let generation = store_version_dir(&store_root, version)?;
    let claude = generation.join(AgentTool::Claude.harness_slug());
    let codex = generation.join(CODEX_HARNESS);

    for path in [&claude, &codex] {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return Ok(None);
        };
        if !metadata.is_dir() || metadata_is_link(&metadata) {
            return Ok(None);
        }
    }

    let Ok(claude_manifest) = claude_staged_manifest(&claude, None, version) else {
        return Ok(None);
    };
    let Ok(codex_manifest) = codex_staged_manifest(&codex, None, version) else {
        return Ok(None);
    };
    Ok(Some(StoredBundleSet {
        claude,
        codex,
        claude_manifest,
        codex_manifest,
    }))
}

fn link_farm_matches(source: &Path, dest: &Path, store_root: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(dest) else {
        return false;
    };
    if !metadata.is_dir() || metadata_is_link(&metadata) {
        return false;
    }

    let Ok(source_entries) = fs::read_dir(source) else {
        return false;
    };
    let mut desired = BTreeMap::new();
    for entry in source_entries {
        let Ok(entry) = entry else {
            return false;
        };
        let Ok(name) = entry.file_name().into_string() else {
            return false;
        };
        desired.insert(name, entry.path());
    }
    if desired.is_empty() {
        return false;
    }

    for (name, target) in &desired {
        let link = dest.join(name);
        let Ok(metadata) = fs::symlink_metadata(&link) else {
            return false;
        };
        if !metadata_is_link(&metadata) || !link_targets_exactly(&link, target) {
            return false;
        }
    }

    let Ok(dest_entries) = fs::read_dir(dest) else {
        return false;
    };
    for entry in dest_entries {
        let Ok(entry) = entry else {
            return false;
        };
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return false;
        };
        if metadata_is_link(&metadata)
            && link_targets_store(&path, store_root)
            && !entry
                .file_name()
                .to_str()
                .is_some_and(|name| desired.contains_key(name))
        {
            return false;
        }
    }
    true
}

/// Per-platform health plus the shared skill bridge. This is version-aware
/// so a config that says "current" cannot accept farms still targeting an
/// older store generation.
fn bundle_set_health(version: &str) -> (bool, bool, bool) {
    let Ok(Some(stored)) = stored_bundle_set(version) else {
        return (false, false, false);
    };
    let (Ok(claude_dest), Ok(codex_dest), Ok(bridge)) = (
        AgentTool::Claude.dest_dir(),
        codex_dest_dir(),
        codex_skill_bridge_dir(),
    ) else {
        return (false, false, false);
    };
    let Ok(store_root) = agent_store_root() else {
        return (false, false, false);
    };

    let claude_healthy = link_farm_matches(
        &stored.claude.join("skills"),
        &claude_dest.join("skills"),
        &store_root,
    ) && link_farm_matches(
        &stored.claude.join("agents"),
        &claude_dest.join("agents"),
        &store_root,
    );
    let codex_healthy = link_farm_matches(
        &stored.codex.join("agents"),
        &codex_dest.join("agents"),
        &store_root,
    );

    let bridge_healthy = paths_resolve_same(&bridge, &claude_dest.join("skills"));
    (claude_healthy, codex_healthy, bridge_healthy)
}

/// Whether every managed link points at the exact requested generation.
pub(crate) fn bundle_set_is_installed(version: &str) -> bool {
    let (claude, codex, bridge) = bundle_set_health(version);
    claude && codex && bridge
}

/// Looser migration/startup gate: does either managed platform already have
/// a recognizable installation, regardless of generation health?
pub(crate) fn bundle_set_has_existing_install() -> bool {
    AgentTool::Claude.has_existing_install()
        || codex_dest_dir().is_ok_and(|dest| dest.join("agents").is_dir())
        || codex_skill_bridge_dir().is_ok_and(|bridge| fs::symlink_metadata(bridge).is_ok())
}

/// Repoint a complete local generation without network access. Returns
/// `None` when that exact generation is absent or incomplete.
pub(crate) fn repair_bundle_set_links(version: &str) -> Result<Option<usize>> {
    let _lock = acquire_bundle_lock()?;
    let Some(stored) = stored_bundle_set(version)? else {
        return Ok(None);
    };
    let claude_dest = AgentTool::Claude.dest_dir()?;
    let codex_dest = codex_dest_dir()?;
    let store_root = agent_store_root()?;
    let bridge = codex_skill_bridge_dir()?;
    let previous_claude = read_installed_manifest(&claude_dest);
    let previous_codex = read_installed_manifest(&codex_dest);
    let frozen = AgentTool::Claude.frozen_manifest();
    let mut claude_sources: Vec<&[ManifestEntry]> =
        vec![stored.claude_manifest.files.as_slice(), frozen.as_slice()];
    if let Some(previous) = &previous_claude {
        claude_sources.push(previous.files.as_slice());
    }
    let claude_known = known_bundle_digests(claude_sources);
    let mut codex_sources: Vec<&[ManifestEntry]> = vec![stored.codex_manifest.files.as_slice()];
    if let Some(previous) = &previous_codex {
        codex_sources.push(previous.files.as_slice());
    }
    let codex_known = known_bundle_digests(codex_sources);
    let claude_skills = plan_link_farm(
        Some(&stored.claude.join("skills")),
        Some(&stored.claude.join("skills")),
        &claude_dest.join("skills"),
        "skills",
        &store_root,
        &claude_known,
    )?;
    let claude_agents = plan_link_farm(
        Some(&stored.claude.join("agents")),
        Some(&stored.claude.join("agents")),
        &claude_dest.join("agents"),
        "agents",
        &store_root,
        &claude_known,
    )?;
    let codex_agents = plan_link_farm(
        Some(&stored.codex.join("agents")),
        Some(&stored.codex.join("agents")),
        &codex_dest.join("agents"),
        "agents",
        &store_root,
        &codex_known,
    )?;
    let bridge_source = claude_dest.join("skills");
    let bridge_plan = plan_skill_bridge(&bridge_source, &bridge, &stored.claude.join("skills"))?;

    let mut transaction = ActivationTransaction::default();
    let activation = (|| -> Result<(SyncReport, SkillBridgeOutcome)> {
        for plan in [&claude_skills, &claude_agents, &codex_agents] {
            if plan.active {
                transaction.ensure_dir(&plan.dest)?;
            }
        }
        if bridge_plan.outcome == SkillBridgeOutcome::Created {
            transaction.ensure_dir(
                bridge
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Codex skill bridge has no parent"))?,
            )?;
        }
        preflight_farm(&claude_skills)?;
        preflight_farm(&claude_agents)?;
        preflight_farm(&codex_agents)?;
        preflight_skill_bridge(&bridge_plan)?;

        let mut report = apply_farm(&claude_skills, &mut transaction)?;
        merge_sync_report(&mut report, apply_farm(&claude_agents, &mut transaction)?);
        merge_sync_report(&mut report, apply_farm(&codex_agents, &mut transaction)?);
        let bridge_outcome = apply_skill_bridge(&bridge_plan, &mut transaction)?;
        if bridge_outcome == SkillBridgeOutcome::Created {
            report.changed += 1;
        }
        Ok((report, bridge_outcome))
    })();
    let (report, _bridge_outcome) = match activation {
        Ok(success) => success,
        Err(error) => {
            if let Err(rollback) = transaction.rollback() {
                return Err(error.context(format!(
                    "Local Claude/Codex repair failed and rollback was incomplete: {rollback:#}"
                )));
            }
            return Err(error);
        }
    };
    transaction.commit();
    Ok(Some(report.changed))
}

pub(crate) fn bundle_set_status_json(config: &crate::config::HyprlayerConfig) -> serde_json::Value {
    let desired = config.desired_assets_version();
    let (claude_healthy, codex_agents_healthy, bridge_healthy) = bundle_set_health(desired);
    let codex_healthy = codex_agents_healthy && bridge_healthy;
    let installed = claude_healthy && codex_healthy;
    let claude_location = AgentTool::Claude
        .dest_dir()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| AgentTool::Claude.dest_display());
    let codex_location = codex_dest_dir()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| format!("~{SEP}.codex{SEP}"));
    let store_location = agent_store_root()
        .ok()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    serde_json::json!({
        "agentTool": "Claude + Codex",
        "installed": installed,
        "location": store_location,
        "platforms": [
            {
                "id": "claude",
                "name": "Claude Code",
                "installed": claude_healthy,
                "location": claude_location,
            },
            {
                "id": "codex",
                "name": "Codex",
                "installed": codex_healthy,
                "location": codex_location,
            },
        ],
        "assetsVersion": config.agents_installed_version,
        "pinnedVersion": config.agents_pinned_version,
        "desiredVersion": desired,
        "binaryVersion": env!("CARGO_PKG_VERSION"),
    })
}

pub(crate) fn print_bundle_set_status(config: &crate::config::HyprlayerConfig) {
    use colored::Colorize;

    let desired = config.desired_assets_version();
    let status = if bundle_set_is_installed(desired) {
        "installed".green()
    } else {
        "needs repair".red()
    };
    println!("  AI Platforms: {}", "Claude Code + Codex".cyan());
    println!("  Status: {status}");
    println!("  Claude: {}", AgentTool::Claude.dest_display().cyan());
    println!("  Codex: {}", format!("~{SEP}.codex{SEP}").cyan());
    println!("  Desired assets: {}", desired.cyan());
}

/// Parse a staged bundle's own `manifest.json`, or `None` for a legacy
/// `master`-tree download, which has none.
///
/// A manifest that exists but does not parse is a hard error: it is the
/// completeness gate's only evidence, and refusing here leaves `dest`
/// untouched, which is the same rollback guarantee a torn download gets.
fn read_staged_manifest(staged: &Path) -> Result<Option<BundleManifest>> {
    let path = staged.join(MANIFEST_FILE_NAME);
    let Ok(body) = fs::read_to_string(&path) else {
        return Ok(None);
    };
    BundleManifest::parse(&body)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("Downloaded bundle carries an unusable manifest: {e}"))
}

/// The manifest the last install recorded in `dest`, or `None` when there
/// is none — a first install, or one done by a pre-manifest CLI.
///
/// A record that will not parse is also `None`, with a warning: we then
/// know nothing about which files are ours, which is exactly the
/// pre-manifest situation, and that degrades to the historical behaviour
/// rather than failing the install. It is replaced at the end of this
/// install.
fn read_installed_manifest(dest: &Path) -> Option<BundleManifest> {
    let path = dest.join(INSTALLED_MANIFEST_FILE);
    let body = fs::read_to_string(&path).ok()?;
    match BundleManifest::parse(&body) {
        Ok(manifest) => Some(manifest),
        Err(e) => {
            eprintln!(
                "warning: ignoring unreadable install record {}: {e}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
fn write_installed_manifest(dest: &Path, manifest: &BundleManifest) -> Result<()> {
    let path = dest.join(INSTALLED_MANIFEST_FILE);
    let body = serde_json::to_string_pretty(manifest)
        .context("Failed to serialize the bundle manifest")?;
    fs::create_dir_all(dest).with_context(|| format!("Failed to create {}", dest.display()))?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".hyprlayer-manifest-")
        .tempfile_in(dest)
        .with_context(|| format!("Failed to stage {}", path.display()))?;
    temporary
        .write_all(body.as_bytes())
        .with_context(|| format!("Failed to stage {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("Failed to sync {}", path.display()))?;
    temporary
        .persist(&path)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// The forward-pin guard: refuse a pinned bundle whose manifest declares a
/// `min_cli_version` newer than the binary about to install it.
///
/// Only pins are checked. An unpinned install resolves to the binary's own
/// version (`resolve_assets_version`), so its bundle's floor is satisfied by
/// construction; a pin is the one way to ask for a bundle cut for a CLI that
/// does not exist here yet, whose skills would then reference commands this
/// binary has no idea about.
///
/// This is a hard error rather than a fallback: `fetch_into` treats a failed
/// asset fetch as "try the legacy tree", and silently installing something
/// other than what was pinned is worse than refusing. `dest` is untouched
/// either way — the check runs before the sync.
fn verify_pin_is_supported(manifest: &BundleManifest, pinned_version: Option<&str>) -> Result<()> {
    let Some(pinned) = pinned_version else {
        return Ok(());
    };
    let cli = env!("CARGO_PKG_VERSION");
    if manifest.supports_cli_version(cli) {
        return Ok(());
    }
    anyhow::bail!(
        "The pinned {} assets bundle ({pinned}) needs hyprlayer {} or newer, but this \
         binary is {cli}. Upgrade hyprlayer, or clear `agentsPinnedVersion` in your \
         hyprlayer config to go back to the bundle that matches this binary.",
        manifest.harness,
        manifest.min_cli_version
    )
}

/// The manifest-driven completeness gate: every file the staged bundle
/// claims to own must be there and must hash to what the manifest recorded.
///
/// This is what `is_installed_at`'s two hardcoded sentinels approximated
/// for asset installs — a torn or corrupted download must never overwrite a
/// good install — except that it covers the whole bundle rather than two
/// files, and catches corruption as well as absence.
fn verify_staged_completeness(
    tool: AgentTool,
    staged: &Path,
    manifest: &BundleManifest,
) -> Result<()> {
    verify_staged_completeness_for(&tool.to_string(), staged, manifest)
}

fn verify_staged_completeness_for(
    label: &str,
    staged: &Path,
    manifest: &BundleManifest,
) -> Result<()> {
    for entry in &manifest.files {
        let path = manifest::resolve_under(staged, &entry.path).ok_or_else(|| {
            anyhow::anyhow!(
                "Downloaded {label} bundle is incomplete — its manifest lists {:?}, \
                 which is not a path inside the bundle.",
                entry.path
            )
        })?;
        if !path.is_file() {
            anyhow::bail!(
                "Downloaded {label} bundle is incomplete — its manifest lists `{}`, \
                 which the bundle does not contain. \
                 Run 'hyprlayer ai reinstall' to retry.",
                entry.path
            );
        }
        integrity::verify_sha256(&path, &entry.sha256).map_err(|e| {
            anyhow::anyhow!(
                "Downloaded {label} bundle is incomplete — `{}` does not match the \
                 digest its manifest records ({e}). \
                 Run 'hyprlayer ai reinstall' to retry.",
                entry.path
            )
        })?;
    }
    Ok(())
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

/// Download a single bundle member to disk.
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
        assert_eq!(sync_tree(&src, &dest, None).unwrap().changed, 0);
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
        assert_eq!(sync_tree(&src, &dest, None).unwrap().changed, 1);
        assert!(dest.join("a.md").is_file());
    }

    #[test]
    fn sync_tree_overwrites_a_differing_file() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        fs::create_dir_all(src.join("agents")).unwrap();
        fs::create_dir_all(dest.join("agents")).unwrap();
        fs::write(src.join("agents/a.md"), "new content").unwrap();
        fs::write(dest.join("agents/a.md"), "old content").unwrap();
        assert_eq!(sync_tree(&src, &dest, None).unwrap().changed, 1);
        assert_eq!(
            fs::read_to_string(dest.join("agents/a.md")).unwrap(),
            "new content"
        );
    }

    /// Same shape one directory up: a file at the harness root is its
    /// config, and with no manifest to prove we wrote it, it is the user's.
    #[test]
    fn sync_tree_preserves_a_differing_root_file() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("settings.json"), "ours").unwrap();
        fs::write(dest.join("settings.json"), "theirs").unwrap();
        let report = sync_tree(&src, &dest, None).unwrap();
        assert_eq!(report.changed, 0);
        assert_eq!(report.preserved, vec!["settings.json"]);
        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "theirs"
        );
    }

    #[test]
    fn sync_tree_preserves_a_dest_only_file() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        touch(&src.join("a.md"));
        touch(&dest.join("personal.md"));
        assert_eq!(sync_tree(&src, &dest, None).unwrap().changed, 1);
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
        assert_eq!(sync_tree(&src, &dest, None).unwrap().changed, 1);
        assert!(dest.join("skills/foo/bar/SKILL.md").is_file());
    }

    #[test]
    fn sync_tree_empty_source_is_a_no_op() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        fs::create_dir_all(&src).unwrap();
        touch(&dest.join("personal.md"));
        assert_eq!(sync_tree(&src, &dest, None).unwrap().changed, 0);
        assert!(dest.join("personal.md").is_file());
    }

    #[test]
    fn sync_tree_missing_source_is_a_no_op() {
        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("does-not-exist");
        let dest = temp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        assert_eq!(sync_tree(&src, &dest, None).unwrap().changed, 0);
    }

    #[test]
    #[cfg(unix)]
    fn copy_sync_never_follows_leaf_or_parent_links_outside_dest() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let src = temp.path().join("src");
        let dest = temp.path().join("dest");
        let outside_leaf = temp.path().join("outside-leaf.md");
        let outside_dir = temp.path().join("outside-dir");
        fs::create_dir_all(src.join("agents/parent")).unwrap();
        fs::write(src.join("agents/leaf.md"), "new leaf\n").unwrap();
        fs::write(src.join("agents/parent/child.md"), "new child\n").unwrap();
        fs::create_dir_all(dest.join("agents")).unwrap();
        fs::write(&outside_leaf, "personal leaf\n").unwrap();
        fs::create_dir_all(&outside_dir).unwrap();
        fs::write(outside_dir.join("child.md"), "personal child\n").unwrap();
        symlink(&outside_leaf, dest.join("agents/leaf.md")).unwrap();
        symlink(&outside_dir, dest.join("agents/parent")).unwrap();

        let report = sync_tree(&src, &dest, None).unwrap();
        assert_eq!(report.changed, 0);
        assert_eq!(
            fs::read_to_string(&outside_leaf).unwrap(),
            "personal leaf\n"
        );
        assert_eq!(
            fs::read_to_string(outside_dir.join("child.md")).unwrap(),
            "personal child\n"
        );
        let mut preserved = report.preserved;
        preserved.sort();
        assert_eq!(preserved, vec!["agents/leaf.md", "agents/parent/child.md"]);
    }

    /// Real digests from the `v1.6.0-rc.1` prerelease that Phase 3 cut, so
    /// the fixture below is the shape GitHub actually returns rather than an
    /// invented one.
    const RC_CLAUDE_DIGEST: &str =
        "42292288c4a5fc6c7f765489da159ceef5c70d0705c2c9d65d999df7bb6c60cd";
    const RC_BINARY_DIGEST: &str =
        "d4b730a0bb9755e2bd6aa763ef5e9b24bd1644feec1eae90ca08285a26ea5575";

    /// A `/releases/tags/<tag>` body carrying the supported pair plus the
    /// binaries, abridged to the fields we read.
    fn release_json_with_all_bundles() -> String {
        format!(
            r#"{{
                "tag_name": "v1.6.0-rc.1",
                "assets": [
                    {{ "name": "hyprlayer-x86_64-unknown-linux-gnu", "digest": "sha256:{RC_BINARY_DIGEST}" }},
                    {{ "name": "hyprlayer-assets-claude-1.6.0-rc.1.tar.gz",   "digest": "sha256:{RC_CLAUDE_DIGEST}" }},
                    {{ "name": "hyprlayer-assets-codex-1.6.0-rc.1.tar.gz", "digest": "sha256:{RC_BINARY_DIGEST}" }}
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

        let files: [(&str, &[u8]); 2] = [
            ("agents/codebase-locator.md", b"---\nname: locator\n---\n"),
            (
                "skills/code_review/SKILL.md",
                b"---\nname: code_review\n---\n",
            ),
        ];
        let manifest = serde_json::to_vec_pretty(&manifest_for("1.6.0", &files)).unwrap();
        let mut entries: Vec<(&str, &[u8])> = files.to_vec();
        entries.push((MANIFEST_FILE_NAME, &manifest));

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

    fn sha256_of_bytes(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    /// A manifest describing exactly `files`, the way
    /// `scripts/build-asset-bundles.sh` would: real digests, paths relative
    /// to the harness root, and no entry for `manifest.json` itself.
    fn manifest_for_harness(
        harness: &str,
        version: &str,
        files: &[(&str, &[u8])],
    ) -> BundleManifest {
        BundleManifest {
            version: version.to_string(),
            harness: harness.to_string(),
            min_cli_version: "1.6.0".to_string(),
            files: files
                .iter()
                .map(|(path, data)| manifest::ManifestEntry {
                    path: (*path).to_string(),
                    sha256: sha256_of_bytes(data),
                })
                .collect(),
        }
    }

    fn manifest_for(version: &str, files: &[(&str, &[u8])]) -> BundleManifest {
        manifest_for_harness("claude", version, files)
    }

    /// Lay out an extracted release-asset bundle in `staged`: the files
    /// plus the `manifest.json` that describes them. This is what
    /// `install_staged` sees after `archive::extract_bundle`.
    fn stage_bundle(staged: &Path, version: &str, files: &[(&str, &[u8])]) {
        for (path, data) in files {
            let dest = staged.join(path);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::write(dest, data).unwrap();
        }
        let manifest = serde_json::to_string_pretty(&manifest_for(version, files)).unwrap();
        fs::write(staged.join(MANIFEST_FILE_NAME), manifest).unwrap();
    }

    fn stage_codex_bundle(staged: &Path, version: &str, files: &[(&str, &[u8])]) {
        for (path, data) in files {
            let dest = staged.join(path);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::write(dest, data).unwrap();
        }
        let manifest =
            serde_json::to_string_pretty(&manifest_for_harness(CODEX_HARNESS, version, files))
                .unwrap();
        fs::write(staged.join(MANIFEST_FILE_NAME), manifest).unwrap();
    }

    #[test]
    fn codex_companion_sync_is_idempotent_and_removal_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("codex");
        fs::create_dir_all(dest.join("agents")).unwrap();
        fs::write(
            dest.join("agents/adjudicator.toml"),
            "name = \"my-adjudicator\"\n",
        )
        .unwrap();
        fs::write(dest.join("agents/personal.toml"), "name = \"personal\"\n").unwrap();

        let v1: [(&str, &[u8]); 2] = [
            (
                "agents/adjudicator.toml",
                b"name = \"adjudicator\"\n" as &[u8],
            ),
            ("agents/cartographer.toml", b"name = \"cartographer\"\n"),
        ];
        let staged_v1 = tmp.path().join("v1");
        stage_codex_bundle(&staged_v1, "1.6.1", &v1);
        let first = install_codex_staged(&staged_v1, &dest, None, "1.6.1").unwrap();
        assert_eq!(first.changed, 1);
        assert_eq!(first.preserved, vec!["agents/adjudicator.toml"]);
        assert_eq!(
            fs::read_to_string(dest.join("agents/adjudicator.toml")).unwrap(),
            "name = \"my-adjudicator\"\n",
            "a pre-existing custom agent at a generated path is never clobbered"
        );

        let staged_same = tmp.path().join("same");
        stage_codex_bundle(&staged_same, "1.6.1", &v1);
        let second = install_codex_staged(&staged_same, &dest, None, "1.6.1").unwrap();
        assert_eq!(second.changed, 0, "a repeated sync writes no agent files");

        let v2: [(&str, &[u8]); 1] = [(
            "agents/adjudicator.toml",
            b"name = \"adjudicator\"\n" as &[u8],
        )];
        let staged_v2 = tmp.path().join("v2");
        stage_codex_bundle(&staged_v2, "1.6.2", &v2);
        let third = install_codex_staged(&staged_v2, &dest, None, "1.6.2").unwrap();
        assert_eq!(third.removed, vec!["agents/cartographer.toml"]);
        assert!(!dest.join("agents/cartographer.toml").exists());
        assert!(dest.join("agents/personal.toml").is_file());
        assert_eq!(
            fs::read_to_string(dest.join("agents/adjudicator.toml")).unwrap(),
            "name = \"my-adjudicator\"\n"
        );
    }

    #[test]
    fn codex_companion_never_removes_an_owned_file_the_user_modified() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("codex");
        let v1: [(&str, &[u8]); 1] = [("agents/gone.toml", b"name = \"gone\"\n" as &[u8])];
        let staged_v1 = tmp.path().join("v1");
        stage_codex_bundle(&staged_v1, "1.6.1", &v1);
        install_codex_staged(&staged_v1, &dest, None, "1.6.1").unwrap();
        fs::write(dest.join("agents/gone.toml"), "my edited agent\n").unwrap();

        let v2: [(&str, &[u8]); 1] = [("agents/current.toml", b"name = \"current\"\n" as &[u8])];
        let staged_v2 = tmp.path().join("v2");
        stage_codex_bundle(&staged_v2, "1.6.2", &v2);
        let report = install_codex_staged(&staged_v2, &dest, None, "1.6.2").unwrap();
        assert!(report.removed.is_empty());
        assert_eq!(
            fs::read_to_string(dest.join("agents/gone.toml")).unwrap(),
            "my edited agent\n"
        );
    }

    #[test]
    fn store_version_is_one_safe_component() {
        let root = Path::new("/store");
        assert_eq!(
            store_version_dir(root, "1.6.1-rc.1").unwrap(),
            root.join("1.6.1-rc.1")
        );
        for invalid in ["", ".", "..", "../escape", "a/b", "a\\b", "v 1"] {
            assert!(
                store_version_dir(root, invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn release_asset_presence_requires_the_supported_pair() {
        let body = release_json_with_all_bundles();
        assert!(release_lists_asset(&body, "hyprlayer-assets-claude-1.6.0-rc.1.tar.gz").unwrap());
        assert!(release_lists_asset(&body, "hyprlayer-assets-codex-1.6.0-rc.1.tar.gz").unwrap());
        assert!(!release_lists_asset(&body, "hyprlayer-assets-unknown-1.6.0-rc.1.tar.gz").unwrap());
        assert!(release_lists_asset("{}", "anything").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn standalone_skill_bridge_creates_noops_and_never_clobbers_a_real_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("claude/skills");
        touch(&source.join("example/SKILL.md"));
        let link = tmp.path().join("agents/skills");

        assert_eq!(
            ensure_skill_bridge(&source, &link).unwrap(),
            SkillBridgeOutcome::Created
        );
        assert_eq!(fs::read_link(&link).unwrap(), source);
        assert!(link.join("example/SKILL.md").is_file());
        assert_eq!(
            ensure_skill_bridge(&source, &link).unwrap(),
            SkillBridgeOutcome::AlreadyCorrect
        );

        let planted = tmp.path().join("other-agents/skills");
        touch(&planted.join("personal/SKILL.md"));
        assert_eq!(
            ensure_skill_bridge(&source, &planted).unwrap(),
            SkillBridgeOutcome::SkippedExisting
        );
        assert!(planted.join("personal/SKILL.md").is_file());
        assert!(
            !fs::symlink_metadata(&planted)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    #[cfg(unix)]
    fn store_install_is_idempotent_and_repoints_each_link_on_version_change() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dest = tmp.path().join("home/.claude");
        let codex_dest = tmp.path().join("home/.codex");
        let store = tmp.path().join("config/hyprlayer/agents");
        let bridge = tmp.path().join("home/.agents/skills");
        touch(&claude_dest.join("skills/personal/SKILL.md"));

        let claude_v1: [(&str, &[u8]); 5] = [
            ("settings.json", b"{}\n"),
            ("agents/codebase-locator.md", b"locator v1\n"),
            ("agents/retired.md", b"retired\n"),
            ("skills/code_review/SKILL.md", b"review v1\n"),
            ("skills/_thoughts/reference.md", b"reference v1\n"),
        ];
        let codex_v1: [(&str, &[u8]); 2] = [
            (
                "agents/codebase-locator.toml",
                b"name = \"codebase-locator\"\n",
            ),
            ("agents/retired.toml", b"name = \"retired\"\n"),
        ];
        let staged_claude_v1 = tmp.path().join("staged-claude-v1");
        let staged_codex_v1 = tmp.path().join("staged-codex-v1");
        stage_bundle(&staged_claude_v1, "1.6.1", &claude_v1);
        stage_codex_bundle(&staged_codex_v1, "1.6.1", &codex_v1);

        let first = install_claude_bundle_set(
            &staged_claude_v1,
            &staged_codex_v1,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            None,
            "1.6.1",
            true,
        )
        .unwrap();
        assert!(first.changed > 0);
        assert_eq!(
            fs::read_link(claude_dest.join("skills/code_review")).unwrap(),
            store.join("1.6.1/claude/skills/code_review")
        );
        assert_eq!(
            fs::read_link(codex_dest.join("agents/codebase-locator.toml")).unwrap(),
            store.join("1.6.1/codex/agents/codebase-locator.toml")
        );
        assert!(claude_dest.join("skills/personal/SKILL.md").is_file());
        assert_eq!(fs::read_link(&bridge).unwrap(), claude_dest.join("skills"));

        let second = install_claude_bundle_set(
            &staged_claude_v1,
            &staged_codex_v1,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            None,
            "1.6.1",
            true,
        )
        .unwrap();
        assert_eq!(second.changed, 0);

        let claude_v2: [(&str, &[u8]); 4] = [
            ("settings.json", b"{}\n"),
            ("agents/codebase-locator.md", b"locator v2\n"),
            ("skills/code_review/SKILL.md", b"review v2\n"),
            ("skills/_thoughts/reference.md", b"reference v2\n"),
        ];
        let codex_v2: [(&str, &[u8]); 1] = [(
            "agents/codebase-locator.toml",
            b"name = \"codebase-locator\"\n",
        )];
        let staged_claude_v2 = tmp.path().join("staged-claude-v2");
        let staged_codex_v2 = tmp.path().join("staged-codex-v2");
        stage_bundle(&staged_claude_v2, "1.6.2", &claude_v2);
        stage_codex_bundle(&staged_codex_v2, "1.6.2", &codex_v2);
        install_claude_bundle_set(
            &staged_claude_v2,
            &staged_codex_v2,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            Some("1.6.2"),
            "1.6.2",
            true,
        )
        .unwrap();
        assert_eq!(
            fs::read_link(claude_dest.join("agents/codebase-locator.md")).unwrap(),
            store.join("1.6.2/claude/agents/codebase-locator.md")
        );
        assert_eq!(
            fs::read_link(codex_dest.join("agents/codebase-locator.toml")).unwrap(),
            store.join("1.6.2/codex/agents/codebase-locator.toml")
        );
        assert!(fs::symlink_metadata(claude_dest.join("agents/retired.md")).is_err());
        assert!(fs::symlink_metadata(codex_dest.join("agents/retired.toml")).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn activation_failure_rolls_back_every_farm_settings_record_and_generation() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dest = tmp.path().join("home/.claude");
        let codex_dest = tmp.path().join("home/.codex");
        let store = tmp.path().join("config/hyprlayer/agents");
        let bridge = tmp.path().join("home/.agents/skills");
        let claude_v1: [(&str, &[u8]); 3] = [
            ("settings.json", b"settings-v1\n"),
            ("agents/codebase-locator.md", b"claude-agent-v1\n"),
            ("skills/code_review/SKILL.md", b"claude-skill-v1\n"),
        ];
        let codex_v1: [(&str, &[u8]); 1] = [(
            "agents/codebase-locator.toml",
            b"name = \"codex-v1\"\n" as &[u8],
        )];
        let staged_claude_v1 = tmp.path().join("staged-claude-v1-rollback");
        let staged_codex_v1 = tmp.path().join("staged-codex-v1-rollback");
        stage_bundle(&staged_claude_v1, "1.6.1", &claude_v1);
        stage_codex_bundle(&staged_codex_v1, "1.6.1", &codex_v1);
        install_claude_bundle_set(
            &staged_claude_v1,
            &staged_codex_v1,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            None,
            "1.6.1",
            true,
        )
        .unwrap();

        let claude_v2: [(&str, &[u8]); 3] = [
            ("settings.json", b"settings-v2\n"),
            ("agents/codebase-locator.md", b"claude-agent-v2\n"),
            ("skills/code_review/SKILL.md", b"claude-skill-v2\n"),
        ];
        let codex_v2: [(&str, &[u8]); 1] = [(
            "agents/codebase-locator.toml",
            b"name = \"codex-v2\"\n" as &[u8],
        )];
        let staged_claude_v2 = tmp.path().join("staged-claude-v2-rollback");
        let staged_codex_v2 = tmp.path().join("staged-codex-v2-rollback");
        stage_bundle(&staged_claude_v2, "1.6.2", &claude_v2);
        stage_codex_bundle(&staged_codex_v2, "1.6.2", &codex_v2);
        let error = install_claude_bundle_set_with_failure(
            &staged_claude_v2,
            &staged_codex_v2,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            Some("1.6.2"),
            "1.6.2",
            true,
            Some(ActivationBoundary::Manifest),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("injected activation failure"));

        assert_eq!(
            fs::read_link(claude_dest.join("skills/code_review")).unwrap(),
            store.join("1.6.1/claude/skills/code_review")
        );
        assert_eq!(
            fs::read_link(claude_dest.join("agents/codebase-locator.md")).unwrap(),
            store.join("1.6.1/claude/agents/codebase-locator.md")
        );
        assert_eq!(
            fs::read_link(codex_dest.join("agents/codebase-locator.toml")).unwrap(),
            store.join("1.6.1/codex/agents/codebase-locator.toml")
        );
        assert_eq!(
            fs::read_to_string(claude_dest.join("settings.json")).unwrap(),
            "settings-v1\n"
        );
        assert_eq!(
            read_installed_manifest(&claude_dest).unwrap().version,
            "1.6.1"
        );
        assert!(paths_resolve_same(&bridge, &claude_dest.join("skills")));
        assert!(!store.join("1.6.2").exists());
        assert_eq!(
            fs::read_to_string(store.join("1.6.1/claude/skills/code_review/SKILL.md")).unwrap(),
            "claude-skill-v1\n"
        );
    }

    #[test]
    #[cfg(unix)]
    fn same_version_generation_replacement_is_restored_when_activation_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dest = tmp.path().join("home/.claude");
        let codex_dest = tmp.path().join("home/.codex");
        let store = tmp.path().join("config/hyprlayer/agents");
        let bridge = tmp.path().join("home/.agents/skills");
        let claude_old: [(&str, &[u8]); 2] = [
            ("agents/codebase-locator.md", b"old-agent\n"),
            ("skills/code_review/SKILL.md", b"old-skill\n"),
        ];
        let codex_old: [(&str, &[u8]); 1] =
            [("agents/codebase-locator.toml", b"old-codex\n" as &[u8])];
        let staged_claude_old = tmp.path().join("same-claude-old");
        let staged_codex_old = tmp.path().join("same-codex-old");
        stage_bundle(&staged_claude_old, "1.6.1", &claude_old);
        stage_codex_bundle(&staged_codex_old, "1.6.1", &codex_old);
        install_claude_bundle_set(
            &staged_claude_old,
            &staged_codex_old,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            None,
            "1.6.1",
            true,
        )
        .unwrap();

        let claude_new: [(&str, &[u8]); 2] = [
            ("agents/codebase-locator.md", b"new-agent\n"),
            ("skills/code_review/SKILL.md", b"new-skill\n"),
        ];
        let codex_new: [(&str, &[u8]); 1] =
            [("agents/codebase-locator.toml", b"new-codex\n" as &[u8])];
        let staged_claude_new = tmp.path().join("same-claude-new");
        let staged_codex_new = tmp.path().join("same-codex-new");
        stage_bundle(&staged_claude_new, "1.6.1", &claude_new);
        stage_codex_bundle(&staged_codex_new, "1.6.1", &codex_new);
        install_claude_bundle_set_with_failure(
            &staged_claude_new,
            &staged_codex_new,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            None,
            "1.6.1",
            true,
            Some(ActivationBoundary::Generation),
        )
        .unwrap_err();

        assert_eq!(
            fs::read_to_string(store.join("1.6.1/claude/skills/code_review/SKILL.md")).unwrap(),
            "old-skill\n"
        );
        assert_eq!(
            fs::read_to_string(claude_dest.join("skills/code_review/SKILL.md")).unwrap(),
            "old-skill\n"
        );
        assert_eq!(
            fs::read_to_string(codex_dest.join("agents/codebase-locator.toml")).unwrap(),
            "old-codex\n"
        );
    }

    #[test]
    #[cfg(unix)]
    fn migration_converts_known_copies_and_preserves_personal_collisions() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dest = tmp.path().join("home/.claude");
        let codex_dest = tmp.path().join("home/.codex");
        let store = tmp.path().join("config/hyprlayer/agents");
        let bridge = tmp.path().join("home/.agents/skills");

        fs::create_dir_all(&claude_dest).unwrap();
        fs::write(claude_dest.join("settings.json"), "{\"mine\":true}\n").unwrap();

        let old: [(&str, &[u8]); 2] = [
            ("agents/codebase-locator.md", b"old locator\n"),
            ("skills/code_review/SKILL.md", b"old review\n"),
        ];
        for (path, bytes) in old {
            let path = claude_dest.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        write_installed_manifest(&claude_dest, &manifest_for("1.6.0", &old)).unwrap();
        touch(&claude_dest.join("skills/personal/SKILL.md"));
        fs::create_dir_all(claude_dest.join("skills/empty-bundled")).unwrap();
        fs::create_dir_all(claude_dest.join("skills/blocked")).unwrap();
        fs::write(
            claude_dest.join("skills/blocked/SKILL.md"),
            "my blocked skill\n",
        )
        .unwrap();

        let current: [(&str, &[u8]); 5] = [
            ("settings.json", b"{}\n"),
            ("agents/codebase-locator.md", b"new locator\n"),
            ("skills/code_review/SKILL.md", b"new review\n"),
            ("skills/empty-bundled/SKILL.md", b"bundle skill\n"),
            ("skills/blocked/SKILL.md", b"bundled blocked\n"),
        ];
        let codex: [(&str, &[u8]); 1] = [(
            "agents/codebase-locator.toml",
            b"name = \"codebase-locator\"\n",
        )];
        let staged_claude = tmp.path().join("staged-claude");
        let staged_codex = tmp.path().join("staged-codex");
        stage_bundle(&staged_claude, "1.6.1", &current);
        stage_codex_bundle(&staged_codex, "1.6.1", &codex);
        let report = install_claude_bundle_set(
            &staged_claude,
            &staged_codex,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            None,
            "1.6.1",
            true,
        )
        .unwrap();

        assert!(
            fs::symlink_metadata(claude_dest.join("skills/code_review"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(claude_dest.join("agents/codebase-locator.md"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(claude_dest.join("skills/personal/SKILL.md").is_file());
        assert!(claude_dest.join("skills/empty-bundled").is_dir());
        assert!(
            !fs::symlink_metadata(claude_dest.join("skills/empty-bundled"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(claude_dest.join("skills/blocked/SKILL.md")).unwrap(),
            "my blocked skill\n"
        );
        assert_eq!(
            fs::read_to_string(claude_dest.join("settings.json")).unwrap(),
            "{\"mine\":true}\n"
        );
        assert!(report.preserved.contains(&"settings.json".to_string()));
        assert!(
            report
                .preserved
                .contains(&"skills/empty-bundled".to_string())
        );
        assert!(report.preserved.contains(&"skills/blocked".to_string()));
    }

    #[test]
    #[cfg(unix)]
    fn frozen_legacy_copy_migrates_without_an_installed_record() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dest = tmp.path().join("home/.claude");
        let codex_dest = tmp.path().join("home/.codex");
        let store = tmp.path().join("config/hyprlayer/agents");
        let bridge = tmp.path().join("home/.agents/skills");
        let relative = "agents/codebase-locator.md";
        let frozen_source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("claude")
            .join(relative);
        let legacy_copy = claude_dest.join(relative);
        fs::create_dir_all(legacy_copy.parent().unwrap()).unwrap();
        fs::copy(&frozen_source, &legacy_copy).unwrap();
        let frozen_entry = AgentTool::Claude
            .frozen_manifest()
            .into_iter()
            .find(|entry| entry.path == relative)
            .unwrap();
        assert_eq!(sha256_of(&legacy_copy), frozen_entry.sha256);
        assert!(!claude_dest.join(INSTALLED_MANIFEST_FILE).exists());

        let claude: [(&str, &[u8]); 2] = [
            ("agents/codebase-locator.md", b"current locator\n"),
            ("skills/code_review/SKILL.md", b"current review\n"),
        ];
        let codex: [(&str, &[u8]); 1] = [(
            "agents/codebase-locator.toml",
            b"name = \"codebase-locator\"\n",
        )];
        let staged_claude = tmp.path().join("staged-claude");
        let staged_codex = tmp.path().join("staged-codex");
        stage_bundle(&staged_claude, "1.6.1", &claude);
        stage_codex_bundle(&staged_codex, "1.6.1", &codex);
        install_claude_bundle_set(
            &staged_claude,
            &staged_codex,
            &claude_dest,
            &codex_dest,
            &store,
            &bridge,
            None,
            "1.6.1",
            true,
        )
        .unwrap();

        assert!(
            fs::symlink_metadata(&legacy_copy)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(&legacy_copy).unwrap(),
            "current locator\n"
        );
    }

    #[test]
    #[cfg(unix)]
    fn root_config_sync_defers_manifest_commit_until_farms_succeed() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("store/claude");
        let dest = tmp.path().join("claude");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("settings.json"), "new\n").unwrap();
        let previous =
            manifest_for_harness("claude", "1.6.0", &[("settings.json", b"old\n" as &[u8])]);
        let current =
            manifest_for_harness("claude", "1.6.1", &[("settings.json", b"new\n" as &[u8])]);
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("settings.json"), "old\n").unwrap();
        write_installed_manifest(&dest, &previous).unwrap();

        sync_claude_root_files(&source, &dest, &current, Some(&previous)).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "new\n"
        );
        assert_eq!(
            read_installed_manifest(&dest).unwrap().version,
            "1.6.0",
            "the caller commits the new record only after link reconciliation"
        );
    }

    #[test]
    #[cfg(unix)]
    fn uninstall_reconciliation_removes_only_links_into_the_store() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let store_sibling = tmp.path().join("store-personal");
        let external = tmp.path().join("external.toml");
        let dest = tmp.path().join("codex/agents");
        touch(&store.join("1/codex/agents/owned.toml"));
        touch(&store_sibling.join("sibling.toml"));
        touch(&external);
        fs::create_dir_all(&dest).unwrap();
        symlink(
            store.join("1/codex/agents/owned.toml"),
            dest.join("owned.toml"),
        )
        .unwrap();
        symlink(&external, dest.join("external.toml")).unwrap();
        symlink(
            store_sibling.join("sibling.toml"),
            dest.join("store-sibling.toml"),
        )
        .unwrap();
        touch(&dest.join("personal.toml"));

        let report = reconcile_link_farm(None, &dest, "agents", &store, &BTreeMap::new()).unwrap();
        assert_eq!(report.removed, vec!["agents/owned.toml"]);
        assert!(fs::symlink_metadata(dest.join("owned.toml")).is_err());
        assert!(fs::symlink_metadata(dest.join("external.toml")).is_ok());
        assert!(fs::symlink_metadata(dest.join("store-sibling.toml")).is_ok());
        assert!(dest.join("personal.toml").is_file());
    }

    #[test]
    #[cfg(unix)]
    fn link_farm_preserves_a_user_owned_root_and_non_utf8_personal_entries() {
        use std::os::unix::ffi::OsStringExt;
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        let source = store.join("1/codex/agents");
        touch(&source.join("bundle.toml"));
        let external = tmp.path().join("external-agents");
        fs::create_dir_all(&external).unwrap();
        let farm = tmp.path().join("codex/agents");
        fs::create_dir_all(farm.parent().unwrap()).unwrap();
        symlink(&external, &farm).unwrap();

        let preserved =
            reconcile_link_farm(Some(&source), &farm, "agents", &store, &BTreeMap::new()).unwrap();
        assert_eq!(preserved.preserved, vec!["agents"]);
        assert!(!external.join("bundle.toml").exists());

        fs::remove_file(&farm).unwrap();
        fs::create_dir_all(&farm).unwrap();
        let non_utf8 = std::ffi::OsString::from_vec(vec![0xff, b'.', b't']);
        touch(&farm.join(non_utf8));
        reconcile_link_farm(Some(&source), &farm, "agents", &store, &BTreeMap::new()).unwrap();
        assert!(farm.join("bundle.toml").exists());
        assert_eq!(fs::read_dir(&farm).unwrap().count(), 2);
    }

    #[test]
    #[cfg(unix)]
    fn legacy_cleanup_never_follows_a_parent_link_into_the_store() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("claude");
        let store_skill = tmp.path().join("store/1/claude/skills/example");
        let target = store_skill.join("SKILL.md");
        touch(&target);
        let target_backup = append_suffix(&target, BACKUP_SUFFIX);
        touch(&target_backup);
        fs::create_dir_all(dest.join("skills")).unwrap();
        symlink(&store_skill, dest.join("skills/example")).unwrap();
        let entries = vec![ManifestEntry {
            path: "skills/example/SKILL.md".to_string(),
            sha256: sha256_of(&target),
        }];

        assert!(remove_legacy_copies(&dest, &entries, None).is_empty());
        assert!(
            target.is_file(),
            "cleanup followed a parent symlink into the store"
        );
        assert_eq!(clean_backups(&dest, &entries), 0);
        assert!(
            target_backup.is_file(),
            "backup cleanup followed a parent symlink into the store"
        );
    }

    #[test]
    #[cfg(unix)]
    fn old_pin_without_codex_is_rejected_without_mutation() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let claude_dest = tmp.path().join("home/.claude");
        let codex_dest = tmp.path().join("home/.codex");
        let store = tmp.path().join("config/hyprlayer/agents");
        let bridge = tmp.path().join("home/.agents/skills");
        let old_codex_target = store.join("newer/codex/agents/old.toml");
        touch(&old_codex_target);
        fs::create_dir_all(codex_dest.join("agents")).unwrap();
        symlink(&old_codex_target, codex_dest.join("agents/old.toml")).unwrap();
        touch(&codex_dest.join("agents/personal.toml"));

        let claude: [(&str, &[u8]); 3] = [
            ("settings.json", b"{}\n"),
            ("agents/codebase-locator.md", b"old locator\n"),
            ("skills/code_review/SKILL.md", b"old review\n"),
        ];
        let staged = tmp.path().join("staged-old-claude");
        stage_bundle(&staged, "1.5.9", &claude);
        let missing_codex = tmp.path().join("missing-codex");
        assert!(
            install_claude_bundle_set(
                &staged,
                &missing_codex,
                &claude_dest,
                &codex_dest,
                &store,
                &bridge,
                Some("1.5.9"),
                "1.5.9",
                true,
            )
            .is_err()
        );

        assert!(!claude_dest.join("skills/code_review/SKILL.md").exists());
        assert!(fs::symlink_metadata(codex_dest.join("agents/old.toml")).is_ok());
        assert!(codex_dest.join("agents/personal.toml").is_file());
        assert!(old_codex_target.is_file());
        assert!(!store.join("1.5.9").exists());
        assert!(fs::symlink_metadata(&bridge).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn invalid_claude_bundle_with_missing_codex_mutates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dest = tmp.path().join("home/.claude");
        let codex_dest = tmp.path().join("home/.codex");
        let store = tmp.path().join("config/hyprlayer/agents");
        let bridge = tmp.path().join("home/.agents/skills");
        touch(&claude_dest.join("sentinel"));
        touch(&codex_dest.join("sentinel"));

        let files: [(&str, &[u8]); 2] = [
            ("agents/codebase-locator.md", b"locator\n"),
            ("skills/code_review/SKILL.md", b"review\n"),
        ];
        let staged = tmp.path().join("torn");
        stage_bundle(&staged, "1.5.9", &files);
        fs::write(staged.join("skills/code_review/SKILL.md"), "corrupt\n").unwrap();
        assert!(
            install_claude_bundle_set(
                &staged,
                &tmp.path().join("missing-codex"),
                &claude_dest,
                &codex_dest,
                &store,
                &bridge,
                Some("1.5.9"),
                "1.5.9",
                true,
            )
            .is_err()
        );
        assert!(claude_dest.join("sentinel").is_file());
        assert!(codex_dest.join("sentinel").is_file());
        assert!(!store.exists());
        assert!(fs::symlink_metadata(&bridge).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn invalid_codex_companion_preflight_mutates_neither_store_nor_farms() {
        let tmp = tempfile::tempdir().unwrap();
        let claude_dest = tmp.path().join("home/.claude");
        let codex_dest = tmp.path().join("home/.codex");
        let store = tmp.path().join("config/hyprlayer/agents");
        let bridge = tmp.path().join("home/.agents/skills");
        touch(&claude_dest.join("sentinel"));
        touch(&codex_dest.join("sentinel"));

        let claude: [(&str, &[u8]); 2] = [
            ("agents/codebase-locator.md", b"locator\n"),
            ("skills/code_review/SKILL.md", b"review\n"),
        ];
        let codex: [(&str, &[u8]); 1] = [(
            "agents/codebase-locator.toml",
            b"name = \"codebase-locator\"\n",
        )];
        let staged_claude = tmp.path().join("claude");
        let staged_codex = tmp.path().join("codex");
        stage_bundle(&staged_claude, "1.6.1", &claude);
        stage_codex_bundle(&staged_codex, "1.6.1", &codex);
        fs::write(
            staged_codex.join("agents/codebase-locator.toml"),
            "corrupt\n",
        )
        .unwrap();

        assert!(
            install_claude_bundle_set(
                &staged_claude,
                &staged_codex,
                &claude_dest,
                &codex_dest,
                &store,
                &bridge,
                None,
                "1.6.1",
                true,
            )
            .is_err()
        );
        assert!(claude_dest.join("sentinel").is_file());
        assert!(codex_dest.join("sentinel").is_file());
        assert!(!store.exists());
        assert!(fs::symlink_metadata(&bridge).is_err());
    }

    /// The two files `is_installed_at` sentinels on, so a fixture bundle is
    /// installable by the legacy gate as well as the manifest one.
    const SENTINELS: [(&str, &[u8]); 2] = [
        ("agents/codebase-locator.md", b"locator v1\n"),
        ("skills/code_review/SKILL.md", b"code_review v1\n"),
    ];

    /// Every file under `dir`, as sorted manifest-form paths.
    fn tree(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = walk_files(dir)
            .unwrap()
            .iter()
            .map(|path| manifest::relative_key(path.strip_prefix(dir).unwrap()).unwrap())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn asset_name_matches_the_builder_output() {
        assert_eq!(
            asset_name("claude", "1.6.0"),
            "hyprlayer-assets-claude-1.6.0.tar.gz"
        );
        assert_eq!(
            asset_name(CODEX_HARNESS, "1.6.0-rc.1"),
            "hyprlayer-assets-codex-1.6.0-rc.1.tar.gz"
        );
    }

    #[test]
    fn live_fallback_paths_do_not_use_the_frozen_root_tree() {
        assert_eq!(AgentTool::Claude.harness_slug(), "claude");
        assert_eq!(AgentTool::Claude.live_repo_dir(), "assets/claude");
        assert_eq!(CODEX_REPO_DIR, "assets/codex");
    }

    /// The resolution truth table: a pin wins, and an unpinned config falls
    /// back to the binary's own version — which is what makes a binary
    /// upgrade move the skills and a pin survive one.
    #[test]
    fn assets_version_resolution_prefers_the_pin_over_the_binary() {
        assert_eq!(resolve_assets_version(None), env!("CARGO_PKG_VERSION"));
        assert_eq!(resolve_assets_version(Some("1.5.9")), "1.5.9");
        assert_eq!(resolve_assets_version(Some("2.0.0")), "2.0.0");
        // A pin is honoured verbatim, backwards or forwards; whether the
        // binary can *consume* the resulting bundle is
        // `verify_pin_is_supported`'s call, not this function's.
        assert_eq!(
            resolve_assets_version(Some("1.6.0-rc.1")),
            "1.6.0-rc.1",
            "a prerelease pin resolves to that exact tag"
        );
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

        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();
        assert_eq!(report.changed, 2);
        assert!(dest.join("skills/code_review/SKILL.md").is_file());
        assert!(dest.join("agents/codebase-locator.md").is_file());
        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "user data"
        );
        // The bundle's self-description is recorded as the install record,
        // not dropped into the user's harness dir as a stray file.
        assert!(!dest.join(MANIFEST_FILE_NAME).exists());
        assert!(dest.join(INSTALLED_MANIFEST_FILE).is_file());
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
            .install_staged(&staged, &dest, None)
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

    // ---------------------------------------------------------------
    // Manifest-driven install: completeness, user-file protection, and
    // orphan removal.
    // ---------------------------------------------------------------

    /// The manifest gate catches what the two sentinels cannot: both
    /// sentinel files are present here, and the bundle is still refused
    /// because a third file the manifest claims never arrived.
    #[test]
    fn manifest_completeness_rejects_a_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("settings.json"), "user data").unwrap();

        let mut files = SENTINELS.to_vec();
        files.push(("skills/create_plan/SKILL.md", b"create_plan v1\n"));
        stage_bundle(&staged, "1.6.0", &files);
        fs::remove_file(staged.join("skills/create_plan/SKILL.md")).unwrap();

        assert!(
            AgentTool::Claude.is_installed_at(&staged),
            "the sentinels are present — only the manifest can catch this"
        );
        let err = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("incomplete"), "{text}");
        assert!(text.contains("skills/create_plan/SKILL.md"), "{text}");
        assert_eq!(
            tree(&dest),
            vec!["settings.json"],
            "a rejected bundle must leave dest byte-identical"
        );
    }

    #[test]
    fn manifest_completeness_rejects_a_mis_hashed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("settings.json"), "user data").unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        // Present, right size class, wrong bytes — a torn or tampered
        // download that every file-existence check would wave through.
        fs::write(staged.join("agents/codebase-locator.md"), "locator XX\n").unwrap();

        let err = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("agents/codebase-locator.md"), "{text}");
        assert!(text.contains("digest"), "{text}");
        assert_eq!(tree(&dest), vec!["settings.json"]);
    }

    /// A `manifest.json` that will not parse is a corrupt bundle, not a
    /// legacy one: falling back to the sentinel gate here would install a
    /// bundle we cannot describe and then record nothing about it.
    #[test]
    fn an_unparseable_bundle_manifest_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        fs::write(staged.join(MANIFEST_FILE_NAME), "{ truncated").unwrap();

        let err = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap_err();
        assert!(format!("{err:#}").contains("manifest"), "{err:#}");
        assert!(tree(&dest).is_empty());
    }

    /// Rewrite a staged bundle's declared CLI floor, so a bundle cut for a
    /// release that does not exist yet can be handed to this binary.
    fn set_staged_min_cli_version(staged: &Path, min_cli_version: &str) {
        let path = staged.join(MANIFEST_FILE_NAME);
        let mut manifest = BundleManifest::parse(&fs::read_to_string(&path).unwrap()).unwrap();
        manifest.min_cli_version = min_cli_version.to_string();
        fs::write(&path, serde_json::to_string_pretty(&manifest).unwrap()).unwrap();
    }

    /// Pinning forward — to a bundle cut for a CLI this binary predates —
    /// is the one way to get skills that reference commands we do not have.
    /// It is refused outright rather than falling back to the legacy tree:
    /// installing something other than what was pinned is worse than
    /// installing nothing.
    #[test]
    fn a_pin_that_needs_a_newer_cli_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("settings.json"), "user data").unwrap();

        stage_bundle(&staged, "9.9.0", &SENTINELS);
        set_staged_min_cli_version(&staged, "9.9.0");

        let err = AgentTool::Claude
            .install_staged(&staged, &dest, Some("9.9.0"))
            .unwrap_err();
        let text = format!("{err:#}");
        assert!(text.contains("9.9.0"), "{text}");
        assert!(
            text.contains("Upgrade hyprlayer"),
            "the message must say what to do about it: {text}"
        );
        assert!(
            text.contains("agentsPinnedVersion"),
            "the message must name the pin that caused this: {text}"
        );
        assert_eq!(
            tree(&dest),
            vec!["settings.json"],
            "a refused pin must leave dest byte-identical"
        );
    }

    /// The guard is scoped to pins on purpose. An unpinned install resolves
    /// to the binary's own version, so its bundle's floor is satisfied by
    /// construction — and gating it here would make a dev build refuse the
    /// very bundles cut from its own tree.
    #[test]
    fn an_unpinned_install_is_not_gated_on_the_cli_floor() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");

        stage_bundle(&staged, "9.9.0", &SENTINELS);
        set_staged_min_cli_version(&staged, "9.9.0");

        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();
        assert_eq!(report.changed, SENTINELS.len());
    }

    #[test]
    fn a_pin_this_binary_can_run_installs() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");

        stage_bundle(&staged, "1.0.0", &SENTINELS);
        set_staged_min_cli_version(&staged, "1.0.0");

        let report = AgentTool::Claude
            .install_staged(&staged, &dest, Some("1.0.0"))
            .unwrap();
        assert_eq!(report.changed, SENTINELS.len());
        assert_eq!(
            read_installed_manifest(&dest).unwrap().version,
            "1.0.0",
            "the pinned bundle is what gets recorded"
        );
    }

    /// The legacy `master`-tree fallback carries no manifest, so it keeps
    /// the sentinel gate — and leaves no install record, which is what
    /// makes the *next* install treat it as pre-manifest.
    #[test]
    fn a_bundle_without_a_manifest_still_uses_the_sentinel_gate() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");

        let torn = tmp.path().join("torn");
        touch(&torn.join("agents/codebase-locator.md"));
        let err = AgentTool::Claude
            .install_staged(&torn, &dest, None)
            .unwrap_err();
        assert!(err.to_string().contains("incomplete"), "{err}");

        let complete = tmp.path().join("complete");
        touch(&complete.join("agents/codebase-locator.md"));
        touch(&complete.join("skills/code_review/SKILL.md"));
        let report = AgentTool::Claude
            .install_staged(&complete, &dest, None)
            .unwrap();
        assert_eq!(report.changed, 2);
        assert!(
            !dest.join(INSTALLED_MANIFEST_FILE).exists(),
            "a legacy install has no manifest to record"
        );
    }

    #[test]
    fn install_records_the_manifest_it_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        let recorded = read_installed_manifest(&dest).expect("install must leave a record");
        assert_eq!(recorded.version, "1.6.0");
        assert_eq!(
            tree(&dest),
            vec![
                ".hyprlayer-manifest.json",
                "agents/codebase-locator.md",
                "skills/code_review/SKILL.md",
            ],
            "the bundle's own manifest.json must not land in dest"
        );
    }

    #[test]
    #[cfg(unix)]
    fn installed_manifest_atomic_replace_never_follows_a_leaf_symlink() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");
        let outside = tmp.path().join("outside.json");
        fs::create_dir_all(&dest).unwrap();
        fs::write(&outside, "personal\n").unwrap();
        symlink(&outside, dest.join(INSTALLED_MANIFEST_FILE)).unwrap();

        write_installed_manifest(&dest, &manifest_for("1.6.1", &SENTINELS)).unwrap();
        assert_eq!(fs::read_to_string(&outside).unwrap(), "personal\n");
        let installed = dest.join(INSTALLED_MANIFEST_FILE);
        assert!(installed.is_file());
        assert!(
            !fs::symlink_metadata(installed)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    /// The `~/.claude/settings.json` case from the plan: we shipped it, the
    /// user edited it, the next bundle ships a different one. Their edit
    /// wins, and every file they did *not* touch still updates.
    #[test]
    fn install_preserves_a_user_modified_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");

        let mut v1 = SENTINELS.to_vec();
        v1.push(("settings.json", b"{\"shipped\": 1}\n"));
        let staged_v1 = tmp.path().join("v1");
        stage_bundle(&staged_v1, "1.6.0", &v1);
        AgentTool::Claude
            .install_staged(&staged_v1, &dest, None)
            .unwrap();

        fs::write(dest.join("settings.json"), "{\"mine\": true}\n").unwrap();

        let v2: [(&str, &[u8]); 3] = [
            ("agents/codebase-locator.md", b"locator v2\n"),
            ("skills/code_review/SKILL.md", b"code_review v1\n"),
            ("settings.json", b"{\"shipped\": 2}\n"),
        ];
        let staged_v2 = tmp.path().join("v2");
        stage_bundle(&staged_v2, "1.6.1", &v2);
        let report = AgentTool::Claude
            .install_staged(&staged_v2, &dest, None)
            .unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "{\"mine\": true}\n",
            "a file the user edited must survive an install"
        );
        assert_eq!(report.preserved, vec!["settings.json"]);
        assert_eq!(
            fs::read_to_string(dest.join("agents/codebase-locator.md")).unwrap(),
            "locator v2\n",
            "files the user did not touch must still update"
        );
        assert_eq!(report.changed, 1);
    }

    /// A file at a path the bundle ships that we never installed is the
    /// user's own work — a hand-written skill of the same name — and is
    /// left alone rather than overwritten.
    #[test]
    fn install_preserves_a_dest_file_we_never_owned() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");

        let staged_v1 = tmp.path().join("v1");
        stage_bundle(&staged_v1, "1.6.0", &SENTINELS);
        AgentTool::Claude
            .install_staged(&staged_v1, &dest, None)
            .unwrap();

        fs::create_dir_all(dest.join("skills/create_plan")).unwrap();
        fs::write(dest.join("skills/create_plan/SKILL.md"), "hand-written\n").unwrap();

        let mut v2 = SENTINELS.to_vec();
        v2.push(("skills/create_plan/SKILL.md", b"ours\n"));
        let staged_v2 = tmp.path().join("v2");
        stage_bundle(&staged_v2, "1.6.1", &v2);
        let report = AgentTool::Claude
            .install_staged(&staged_v2, &dest, None)
            .unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("skills/create_plan/SKILL.md")).unwrap(),
            "hand-written\n"
        );
        assert_eq!(report.preserved, vec!["skills/create_plan/SKILL.md"]);
        assert_eq!(report.changed, 0);
    }

    /// The migration case. Upgrading from a pre-1.6.0 install there is no
    /// record of what we own, so the historical overwrite behaviour has to
    /// stand — skipping instead would freeze every existing user on the
    /// bundle they already have.
    #[test]
    fn install_without_a_prior_manifest_overwrites_as_before() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(dest.join("agents")).unwrap();
        fs::write(dest.join("agents/codebase-locator.md"), "locator 1.5.9\n").unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("agents/codebase-locator.md")).unwrap(),
            "locator v1\n"
        );
        assert!(report.preserved.is_empty());
        assert_eq!(report.changed, 2);
    }

    /// The other half of the migration case: content is replaced in place,
    /// with nothing dropped beside it. 1.6.0 copied each replaced file to
    /// `<name>.hyprlayer-backup`, which put a second file inside every skill
    /// directory the harness scans.
    #[test]
    fn first_manifest_install_replaces_content_without_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(dest.join("agents")).unwrap();
        fs::write(dest.join("agents/codebase-locator.md"), "my own edit\n").unwrap();
        fs::create_dir_all(dest.join("skills/code_review")).unwrap();
        fs::write(dest.join("skills/code_review/SKILL.md"), "my own skill\n").unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("agents/codebase-locator.md")).unwrap(),
            "locator v1\n",
            "the bundle's version lands, so the user is not frozen"
        );
        assert_eq!(
            fs::read_to_string(dest.join("skills/code_review/SKILL.md")).unwrap(),
            "code_review v1\n"
        );
        assert_eq!(
            tree(&dest),
            vec![
                ".hyprlayer-manifest.json",
                "agents/codebase-locator.md",
                "skills/code_review/SKILL.md",
            ],
            "no backup copies anywhere in the tree"
        );
        assert!(report.preserved.is_empty());
        assert_eq!(report.cleaned_backups, 0);
    }

    /// The one file at the harness root is not ours to replace: with no
    /// manifest to prove otherwise, a `settings.json` already on disk holds
    /// the user's permissions, hooks and env, and our copy is only a
    /// starting point for a fresh install.
    #[test]
    fn first_manifest_install_keeps_existing_harness_config() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("settings.json"), "{\"mine\": true}\n").unwrap();

        let mut files = SENTINELS.to_vec();
        files.push(("settings.json", b"{\"ours\": true}\n"));
        stage_bundle(&staged, "1.6.0", &files);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "{\"mine\": true}\n",
            "the user's own settings survive the migration install"
        );
        assert!(
            !dest.join("settings.json.hyprlayer-backup").exists(),
            "preserving it in place means there is nothing to copy aside"
        );
        assert_eq!(report.preserved, vec!["settings.json"]);
        assert_eq!(report.changed, 2, "content still installs");
    }

    /// A fresh install still gets our starter config — preserving one only
    /// applies to a file that is already there.
    #[test]
    fn install_writes_harness_config_when_none_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");

        let mut files = SENTINELS.to_vec();
        files.push(("settings.json", b"{\"ours\": true}\n"));
        stage_bundle(&staged, "1.6.0", &files);
        AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "{\"ours\": true}\n"
        );
    }

    /// The machines 1.6.0 already ran on keep their backups until something
    /// clears them, so every install sweeps the paths it owns — and only
    /// those. A file wearing the suffix beside a skill of the user's own was
    /// not written by any install of ours, whatever put it there.
    #[test]
    fn install_clears_leftover_backups_beside_files_we_own() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(dest.join("agents")).unwrap();
        fs::create_dir_all(dest.join("skills/code_review")).unwrap();
        fs::create_dir_all(dest.join("skills/mine")).unwrap();
        // What 1.6.0 left, beside two files we ship — plus one beside a
        // skill of the user's own, which no install of ours can have
        // written, since we have never shipped that path.
        fs::write(
            dest.join("agents/codebase-locator.md.hyprlayer-backup"),
            "old locator\n",
        )
        .unwrap();
        fs::write(
            dest.join("skills/code_review/SKILL.md.hyprlayer-backup"),
            "old code_review\n",
        )
        .unwrap();
        fs::write(dest.join("skills/mine/SKILL.md"), "hand-written\n").unwrap();
        fs::write(
            dest.join("skills/mine/SKILL.md.hyprlayer-backup"),
            "older hand-written\n",
        )
        .unwrap();
        // The user's own settings copy, taken by that same install, is the
        // one thing the sweep must not touch.
        fs::write(
            dest.join("settings.json.hyprlayer-backup"),
            "{\"mine\": true}\n",
        )
        .unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert_eq!(report.cleaned_backups, 2);
        assert_eq!(
            tree(&dest),
            vec![
                ".hyprlayer-manifest.json",
                "agents/codebase-locator.md",
                "settings.json.hyprlayer-backup",
                "skills/code_review/SKILL.md",
                "skills/mine/SKILL.md",
                "skills/mine/SKILL.md.hyprlayer-backup",
            ],
            "backups beside files we ship go; the harness-config copy, the \
             user's own skill and the stray beside it all stay"
        );
    }

    /// An up-to-date install writes nothing, and still has to sweep: the
    /// daily refresh is what heals most affected machines.
    #[test]
    fn a_no_op_install_still_clears_leftover_backups() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();
        fs::write(
            dest.join("skills/code_review/SKILL.md.hyprlayer-backup"),
            "leftover\n",
        )
        .unwrap();

        let staged2 = tmp.path().join("staged2");
        stage_bundle(&staged2, "1.6.0", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged2, &dest, None)
            .unwrap();

        assert_eq!(report.changed, 0);
        assert_eq!(report.cleaned_backups, 1);
        assert!(
            !dest
                .join("skills/code_review/SKILL.md.hyprlayer-backup")
                .exists()
        );
    }

    /// A directory left holding nothing but a backup — its real file
    /// dropped by this bundle — goes away with it, rather than lingering as
    /// an empty skill directory.
    #[test]
    fn clearing_the_last_backup_prunes_the_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");

        let mut v1 = SENTINELS.to_vec();
        v1.push(("skills/gone/SKILL.md", b"gone v1\n"));
        let staged_v1 = tmp.path().join("v1");
        stage_bundle(&staged_v1, "1.6.0", &v1);
        AgentTool::Claude
            .install_staged(&staged_v1, &dest, None)
            .unwrap();
        fs::write(
            dest.join("skills/gone/SKILL.md.hyprlayer-backup"),
            "older gone\n",
        )
        .unwrap();

        // v2 drops the skill: orphan removal takes `SKILL.md` but cannot
        // prune past the backup still sitting beside it, so the sweep has to
        // be what finishes the directory off.
        let staged_v2 = tmp.path().join("v2");
        stage_bundle(&staged_v2, "1.6.1", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged_v2, &dest, None)
            .unwrap();

        assert_eq!(report.removed, vec!["skills/gone/SKILL.md"]);
        assert_eq!(report.cleaned_backups, 1);
        assert!(!dest.join("skills/gone").exists());
        assert!(dest.join("skills/code_review").is_dir());
    }

    /// The sweep must not reach outside `dest`. Walking the content
    /// directories for the suffix did: `is_dir()` follows symlinks, so a
    /// skills directory pointed at a dotfiles checkout — an ordinary thing
    /// to do — put every matching file in that checkout in range. Driving
    /// the sweep off the manifest instead means an unowned path is never
    /// looked at, symlink or not.
    #[test]
    #[cfg(unix)]
    fn the_sweep_never_follows_a_symlink_out_of_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");

        let outside = tmp.path().join("dotfiles");
        fs::create_dir_all(&outside).unwrap();
        let theirs = outside.join("SKILL.md.hyprlayer-backup");
        fs::write(&theirs, "not ours\n").unwrap();

        fs::create_dir_all(dest.join("skills")).unwrap();
        std::os::unix::fs::symlink(&outside, dest.join("skills/linked")).unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert_eq!(report.cleaned_backups, 0);
        assert!(
            theirs.is_file(),
            "a file outside dest is never the sweep's to delete"
        );
    }

    /// A symlink that happens to wear the suffix is not a copy we took, so
    /// unlinking it would break whatever points through it.
    #[test]
    #[cfg(unix)]
    fn the_sweep_leaves_a_symlink_wearing_the_suffix_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        let target = tmp.path().join("target.md");
        fs::write(&target, "theirs\n").unwrap();

        fs::create_dir_all(dest.join("skills/code_review")).unwrap();
        std::os::unix::fs::symlink(
            &target,
            dest.join("skills/code_review/SKILL.md.hyprlayer-backup"),
        )
        .unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert_eq!(report.cleaned_backups, 0);
        assert!(target.is_file(), "the symlink's target survives");
        assert!(
            fs::symlink_metadata(dest.join("skills/code_review/SKILL.md.hyprlayer-backup")).is_ok(),
            "and so does the symlink itself"
        );
    }

    /// The sweep runs after the bundle and the manifest are already on
    /// disk, so it must never be what fails an install: the caller would
    /// report a failure for an install that had in fact landed, and skip
    /// its own bookkeeping with it.
    #[test]
    #[cfg(unix)]
    fn a_cleanup_it_cannot_finish_still_leaves_the_install_successful() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        let dir = dest.join("skills/code_review");
        fs::create_dir_all(&dir).unwrap();
        // Already byte-identical to the bundle's copy, so the sync itself
        // needs no write into this directory and the only thing left to
        // fail is the cleanup.
        fs::write(dir.join("SKILL.md"), "code_review v1\n").unwrap();
        fs::write(dir.join("SKILL.md.hyprlayer-backup"), "old\n").unwrap();
        // No unlink permission in the directory holding it.
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        let outcome = AgentTool::Claude.install_staged(&staged, &dest, None);

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            outcome.is_ok(),
            "cleanup trouble is a warning, not a failed install: {:?}",
            outcome.err()
        );
        assert!(
            dest.join(INSTALLED_MANIFEST_FILE).is_file(),
            "and the record this install wrote still stands"
        );
    }

    #[test]
    fn settings_is_the_only_mutable_root_file() {
        assert!(is_harness_config(Path::new("settings.json")));
        assert!(!is_harness_config(Path::new("README.md")));
        assert!(!is_harness_config(
            &Path::new("skills").join("commit").join("SKILL.md")
        ));
        assert!(!is_harness_config(&Path::new("agents").join("herald.md")));
    }

    #[test]
    fn windows_file_link_privilege_error_gives_actionable_guidance() {
        let error = std::io::Error::from_raw_os_error(1314);
        let message = windows_file_symlink_error(&error, Path::new(r"C:\Users\me\.codex\agents"));
        assert!(message.contains("Developer Mode"));
        assert!(message.contains("elevated terminal"));
        assert!(message.contains("No agent links were changed"));
        assert!(message.contains("1314"));
    }

    /// The destructive path, with every case that must *not* be deleted
    /// sitting next to the one that must:
    ///
    /// - `skills/gone/SKILL.md` — ours, unmodified, dropped → removed;
    /// - `skills/edited/SKILL.md` — ours, dropped, but the user changed it
    ///   → kept;
    /// - `notes/mine.md` — never ours → kept;
    /// - the sentinels — still in the new bundle → kept.
    #[test]
    fn orphan_removal_deletes_only_dropped_unmodified_owned_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");

        let mut v1 = SENTINELS.to_vec();
        v1.push(("skills/gone/SKILL.md", b"gone v1\n"));
        v1.push(("skills/edited/SKILL.md", b"edited v1\n"));
        let staged_v1 = tmp.path().join("v1");
        stage_bundle(&staged_v1, "1.6.0", &v1);
        AgentTool::Claude
            .install_staged(&staged_v1, &dest, None)
            .unwrap();

        fs::write(dest.join("skills/edited/SKILL.md"), "my notes\n").unwrap();
        fs::create_dir_all(dest.join("notes")).unwrap();
        fs::write(dest.join("notes/mine.md"), "personal\n").unwrap();

        let staged_v2 = tmp.path().join("v2");
        stage_bundle(&staged_v2, "1.6.1", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged_v2, &dest, None)
            .unwrap();

        assert_eq!(report.removed, vec!["skills/gone/SKILL.md"]);
        assert_eq!(
            tree(&dest),
            vec![
                ".hyprlayer-manifest.json",
                "agents/codebase-locator.md",
                "notes/mine.md",
                "skills/code_review/SKILL.md",
                "skills/edited/SKILL.md",
            ]
        );
        assert_eq!(
            fs::read_to_string(dest.join("skills/edited/SKILL.md")).unwrap(),
            "my notes\n",
            "an edited file we shipped is the user's, not an orphan"
        );
        assert!(
            !dest.join("skills/gone").exists(),
            "the directory an orphan emptied should be pruned"
        );
    }

    /// The digest guard on the frozen list: a retired workflow's path is
    /// not enough on its own. These bytes are not the ones we shipped, so
    /// whoever wrote them, it was not us, and the file stays.
    #[test]
    fn a_leftover_that_is_not_byte_identical_to_ours_is_never_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(dest.join("skills/create_plan_nt")).unwrap();
        fs::write(dest.join("skills/create_plan_nt/SKILL.md"), "old skill\n").unwrap();

        stage_bundle(&staged, "1.6.0", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert!(report.removed.is_empty());
        assert!(dest.join("skills/create_plan_nt/SKILL.md").is_file());
    }

    /// The frozen root tree, which is what every pre-1.6.0 install put in
    /// `dest`. Real bytes, so the embedded digests are exercised rather
    /// than a fixture's.
    fn frozen_tree(tool: AgentTool) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(tool.harness_slug())
    }

    /// Copy `paths` out of the frozen tree into `dest`, reproducing what a
    /// pre-1.6.0 install left on disk.
    fn install_frozen_files(tool: AgentTool, dest: &Path, paths: &[&str]) {
        for path in paths {
            let to = dest.join(path);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::copy(frozen_tree(tool).join(path), to).unwrap();
        }
    }

    /// The migration case that the recorded-manifest machinery cannot
    /// reach: `ci_commit` and the seven other workflows retired before
    /// 1.6.0 are in no manifest, because the installs that wrote them wrote
    /// no manifest. They are still sitting in `~/.claude/skills`, where the
    /// harness finds them next to the skills that replaced them, and the
    /// frozen tree is what proves they are ours to delete.
    #[test]
    fn install_removes_workflows_retired_before_1_6_0() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");

        install_frozen_files(
            AgentTool::Claude,
            &dest,
            &[
                "skills/ci_commit/SKILL.md",
                "skills/create_plan_nt/SKILL.md",
                "skills/create_plan/SKILL.md",
                "agents/codebase-locator.md",
            ],
        );
        // One retired workflow the user has since edited, and one skill of
        // their own: neither is ours to remove.
        fs::create_dir_all(dest.join("skills/ci_describe_pr")).unwrap();
        fs::write(dest.join("skills/ci_describe_pr/SKILL.md"), "my take\n").unwrap();
        fs::create_dir_all(dest.join("skills/mine")).unwrap();
        fs::write(dest.join("skills/mine/SKILL.md"), "hand-written\n").unwrap();

        // `create_plan` is in the frozen tree *and* in this bundle, so it
        // is replaced rather than removed — the shape the retired ones
        // would have if they were merely renamed.
        let mut files = SENTINELS.to_vec();
        files.push(("skills/create_plan/SKILL.md", b"create_plan v1\n"));
        stage_bundle(&staged, "1.6.0", &files);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert_eq!(
            report.removed,
            vec![
                "skills/ci_commit/SKILL.md",
                "skills/create_plan_nt/SKILL.md"
            ]
        );
        assert_eq!(
            fs::read_to_string(dest.join("skills/create_plan/SKILL.md")).unwrap(),
            "create_plan v1\n",
            "a skill still in the bundle is brought up to date, not deleted"
        );
        assert_eq!(
            tree(&dest),
            vec![
                ".hyprlayer-manifest.json",
                "agents/codebase-locator.md",
                "skills/ci_describe_pr/SKILL.md",
                "skills/code_review/SKILL.md",
                "skills/create_plan/SKILL.md",
                "skills/mine/SKILL.md",
            ],
            "the two pristine retired workflows go; the edited one, the \
             user's own skill, and a skill this bundle still ships stay"
        );
        assert!(
            !dest.join("skills/ci_commit").exists(),
            "the directory a removal emptied is pruned, not left behind"
        );
    }

    /// The machines that already took 1.6.0 have a record now, and it lists
    /// none of the retired workflows — so consulting the frozen list only
    /// when a record is missing would leave exactly the machines that hit
    /// this uncleaned. It is consulted on every install.
    #[test]
    fn retired_workflows_go_even_when_an_install_record_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");

        let staged_v1 = tmp.path().join("v1");
        stage_bundle(&staged_v1, "1.6.0", &SENTINELS);
        AgentTool::Claude
            .install_staged(&staged_v1, &dest, None)
            .unwrap();
        assert!(read_installed_manifest(&dest).is_some());

        install_frozen_files(AgentTool::Claude, &dest, &["skills/ci_commit/SKILL.md"]);

        let staged_v2 = tmp.path().join("v2");
        stage_bundle(&staged_v2, "1.6.1", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged_v2, &dest, None)
            .unwrap();

        assert_eq!(report.removed, vec!["skills/ci_commit/SKILL.md"]);
        assert!(!dest.join("skills/ci_commit").exists());
    }

    /// A file the frozen tree lists and this bundle still ships is not an
    /// orphan, however it got there — the sweep must not delete something
    /// the same install just wrote.
    #[test]
    fn frozen_cleanup_never_touches_a_path_the_bundle_still_ships() {
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let dest = tmp.path().join("dest");

        install_frozen_files(AgentTool::Claude, &dest, &["settings.json"]);

        let mut files = SENTINELS.to_vec();
        files.push(("settings.json", b"{\"ours\": true}\n"));
        stage_bundle(&staged, "1.6.0", &files);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert!(report.removed.is_empty());
        assert!(dest.join("settings.json").is_file());
    }

    /// The whole repair, on the tree a real machine has.
    ///
    /// Reconstructs what a 1.6.0 install left in `~/.claude`: the bundle's
    /// own files, the workflows retired before 1.6.0 that nothing could
    /// prove were ours, and a `.hyprlayer-backup` beside every file that
    /// install replaced. One more install has to land exactly on the
    /// bundle — no leftovers, nothing of the harness's own lost — for all
    /// three harnesses.
    #[test]
    fn a_1_6_0_tree_heals_completely_on_the_next_install() {
        for tool in AgentTool::ALL {
            let tmp = tempfile::tempdir().unwrap();
            let dest = tmp.path().join("dest");
            let assets = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .join(tool.harness_slug());
            let frozen = frozen_tree(*tool);

            // The bundle, packed from the live tree the release is cut from.
            let bytes: Vec<(String, Vec<u8>)> = tree(&assets)
                .into_iter()
                .map(|path| {
                    let data = fs::read(assets.join(&path)).unwrap();
                    (path, data)
                })
                .collect();
            let files: Vec<(&str, &[u8])> = bytes
                .iter()
                .map(|(path, data)| (path.as_str(), data.as_slice()))
                .collect();
            let staged = tmp.path().join("staged");
            stage_bundle(&staged, "1.6.1", &files);

            // 1.6.0's install, as it left `dest`.
            let staged_v1 = tmp.path().join("v1");
            stage_bundle(&staged_v1, "1.6.0", &files);
            tool.install_staged(&staged_v1, &dest, None).unwrap();

            // What it could not remove: the retired workflows, still
            // carrying the bytes the pre-1.6.0 install wrote.
            let retired: Vec<String> = tree(&frozen)
                .into_iter()
                .filter(|path| !bytes.iter().any(|(shipped, _)| shipped == path))
                .collect();
            assert!(!retired.is_empty(), "{tool} should have retired files");
            let retired_refs: Vec<&str> = retired.iter().map(String::as_str).collect();
            install_frozen_files(*tool, &dest, &retired_refs);

            // ...and what it wrote: a copy beside every file it replaced.
            let replaced: Vec<String> = bytes
                .iter()
                .filter(|(path, data)| fs::read(frozen.join(path)).is_ok_and(|old| old != *data))
                .map(|(path, _)| path.clone())
                .collect();
            assert!(!replaced.is_empty(), "{tool} should have replaced files");
            for path in &replaced {
                fs::write(
                    dest.join(format!("{path}{BACKUP_SUFFIX}")),
                    "the pre-1.6.0 bytes\n",
                )
                .unwrap();
            }

            let report = tool.install_staged(&staged, &dest, None).unwrap();

            assert_eq!(report.changed, 0, "{tool} content is already current");
            assert_eq!(report.removed, retired, "{tool} retired workflows");
            assert_eq!(
                report.cleaned_backups,
                replaced.len(),
                "{tool} leftover backups"
            );

            let mut expected = vec![INSTALLED_MANIFEST_FILE.to_string()];
            expected.extend(tree(&assets));
            expected.sort();
            assert_eq!(
                tree(&dest),
                expected,
                "{tool} should land on exactly the bundle plus its record"
            );
        }
    }

    /// The embedded lists are the only evidence a migration install has for
    /// what a pre-1.6.0 install wrote, so they have to describe the frozen
    /// trees exactly: every file, and the bytes actually on disk. Rerun
    /// `scripts/build-frozen-manifests.sh` if this fails — though per
    /// `assets/FROZEN.md` a frozen tree changing at all wants explaining.
    #[test]
    fn frozen_manifests_match_the_frozen_trees() {
        for tool in AgentTool::ALL {
            let listed: Vec<String> = tool
                .frozen_manifest()
                .iter()
                .map(|entry| entry.path.clone())
                .collect();
            assert!(
                !listed.is_empty(),
                "{tool}'s built-in file list is empty or did not parse"
            );

            let dir = frozen_tree(*tool);
            let mut sorted = listed.clone();
            sorted.sort();
            assert_eq!(listed, sorted, "{tool}'s list should be path-sorted");
            assert_eq!(
                listed,
                tree(&dir),
                "{tool}'s list does not match {}",
                dir.display()
            );

            for entry in tool.frozen_manifest() {
                let bytes = fs::read(dir.join(&entry.path)).unwrap();
                assert!(
                    integrity::bytes_match_sha256(&bytes, &entry.sha256),
                    "{tool}'s recorded digest for {} is stale",
                    entry.path
                );
            }
        }
    }

    /// The install record is a file in a directory the user can write to,
    /// so an entry naming a path outside `dest` must resolve to nothing at
    /// all rather than to a deletion.
    #[test]
    fn orphan_removal_ignores_manifest_paths_outside_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");
        let outsider = tmp.path().join("outside.md");
        fs::write(&outsider, "not ours\n").unwrap();
        fs::create_dir_all(&dest).unwrap();

        let previous = BundleManifest {
            version: "1.6.0".to_string(),
            harness: "claude".to_string(),
            min_cli_version: "1.6.0".to_string(),
            files: vec![
                manifest::ManifestEntry {
                    path: "../outside.md".to_string(),
                    sha256: sha256_of_bytes(b"not ours\n"),
                },
                manifest::ManifestEntry {
                    path: "/etc/hosts".to_string(),
                    sha256: sha256_of_bytes(b"not ours\n"),
                },
            ],
        };
        fs::write(
            dest.join(INSTALLED_MANIFEST_FILE),
            serde_json::to_string_pretty(&previous).unwrap(),
        )
        .unwrap();

        let staged = tmp.path().join("staged");
        stage_bundle(&staged, "1.6.1", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert!(report.removed.is_empty());
        assert!(outsider.is_file(), "a path outside dest is never deleted");
    }

    /// An install record we cannot parse tells us nothing about what we
    /// own, so it must delete nothing — not everything.
    #[test]
    fn an_unparseable_install_record_deletes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");
        fs::create_dir_all(dest.join("skills/gone")).unwrap();
        fs::write(dest.join("skills/gone/SKILL.md"), "gone v1\n").unwrap();
        fs::write(dest.join(INSTALLED_MANIFEST_FILE), "{ truncated").unwrap();

        let staged = tmp.path().join("staged");
        stage_bundle(&staged, "1.6.1", &SENTINELS);
        let report = AgentTool::Claude
            .install_staged(&staged, &dest, None)
            .unwrap();

        assert!(report.removed.is_empty());
        assert!(dest.join("skills/gone/SKILL.md").is_file());
        assert!(
            read_installed_manifest(&dest).is_some(),
            "the unusable record is replaced by this install's"
        );
    }

    /// `prune_empty_dirs` walks upward, so it must stop at `dest` and must
    /// never take a directory that still holds something.
    #[test]
    fn prune_empty_dirs_stops_at_dest_and_at_the_first_non_empty_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("dest");
        fs::create_dir_all(dest.join("skills/a/b")).unwrap();
        prune_empty_dirs(&dest, &dest.join("skills/a/b"));
        assert!(dest.is_dir(), "dest itself is never removed");
        assert!(!dest.join("skills").exists());

        fs::create_dir_all(dest.join("skills/keep/inner")).unwrap();
        fs::write(dest.join("skills/keep/other.md"), "x").unwrap();
        prune_empty_dirs(&dest, &dest.join("skills/keep/inner"));
        assert!(!dest.join("skills/keep/inner").exists());
        assert!(
            dest.join("skills/keep/other.md").is_file(),
            "a parent holding anything else must survive"
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
    fn claude_is_installed_requires_skills() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_claude_is_installed");
        fs::remove_dir_all(&temp_root).ok();

        let case_full = temp_root.join("full");
        touch(&case_full.join("skills/code_review/SKILL.md"));
        touch(&case_full.join("agents/codebase-locator.md"));
        assert!(AgentTool::Claude.is_installed_at(&case_full));

        // Existing install with the right top-level dirs but no sentinels —
        // automatic provisioning must install the new bundle.
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

    /// `has_existing_install` must accept any layout that *was* a valid
    /// install at some point — sentinel files may have moved/renamed
    /// between bundles, but the structural directories haven't. A pre-
    /// `code_review` install is exactly the case the auto-reinstall path
    /// needs to refresh.
    #[test]
    fn has_existing_install_accepts_dirs_without_current_sentinels() {
        let temp_root = std::env::temp_dir().join("hyprlayer_test_has_existing_install");
        fs::remove_dir_all(&temp_root).ok();

        let tool = AgentTool::Claude;
        let (dir_a, dir_b) = ("skills", "agents");
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

        fs::remove_dir_all(&temp_root).ok();
    }

    #[test]
    fn bundle_set_status_json_has_the_paired_platform_contract() {
        let value = bundle_set_status_json(&crate::config::HyprlayerConfig::default());
        assert_eq!(value["agentTool"], "Claude + Codex");
        assert!(value["installed"].is_boolean());
        assert!(value["location"].is_string());
        let platforms = value["platforms"].as_array().unwrap();
        assert_eq!(platforms.len(), 2);
        assert_eq!(platforms[0]["id"], "claude");
        assert_eq!(platforms[0]["name"], "Claude Code");
        assert!(platforms[0]["installed"].is_boolean());
        assert!(platforms[0]["location"].is_string());
        assert_eq!(platforms[1]["id"], "codex");
        assert_eq!(platforms[1]["name"], "Codex");
        assert!(platforms[1]["installed"].is_boolean());
        assert!(platforms[1]["location"].is_string());
    }
}
