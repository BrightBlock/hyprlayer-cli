use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{ThoughtsConfig, get_default_thoughts_repo};
use crate::git_ops::GitRepo;

/// Canonicalize `p`, falling back to `p` itself when it can't be resolved
/// (e.g. it doesn't exist yet) — an unresolved path is its own canonical form.
fn canonicalize_or_self(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// True if the already-canonicalized `cwd` is at or beneath `root`.
/// Canonicalizing both sides collapses `thoughts/` symlinks back into the
/// repo and neutralizes `..`; a `root` that can't be canonicalized contains
/// nothing.
fn path_at_or_within(root: &Path, cwd_canon: &Path) -> bool {
    match fs::canonicalize(root) {
        Ok(root_canon) => cwd_canon.starts_with(&root_canon),
        Err(_) => false,
    }
}

/// The configured thoughts root `cwd` sits at or beneath, if any —
/// top-level backend plus every profile (git checkouts and Obsidian content
/// roots; Notion/Anytype have no local tree).
fn thoughts_repo_containing(thoughts: &ThoughtsConfig, cwd: &Path) -> Option<PathBuf> {
    let cwd_canon = canonicalize_or_self(cwd);
    std::iter::once(&thoughts.backend)
        .chain(thoughts.profiles.values().map(|p| &p.backend))
        .filter_map(|b| b.filesystem_content_root())
        .find(|root| path_at_or_within(root, &cwd_canon))
}

/// Error out if `cwd` is inside a *configured* thoughts root. No `--force`
/// escape — it is never valid to run from there.
pub fn ensure_cwd_outside_thoughts_repo(
    thoughts: &ThoughtsConfig,
    cwd: &Path,
    command: &str,
) -> Result<()> {
    match thoughts_repo_containing(thoughts, cwd) {
        Some(root) => Err(recursive_run_error(command, &root)),
        None => Ok(()),
    }
}

/// Error out if `cwd` is at or inside `content_root` — the thoughts root
/// this `init` is about to write into. Catches what the configured-roots
/// check can't: a first-ever init, or an interactive run that selects the
/// cwd as the thoughts repo. Call on the resolved root, before any writes.
pub fn ensure_cwd_outside_content_root(
    content_root: &Path,
    cwd: &Path,
    command: &str,
) -> Result<()> {
    if path_at_or_within(content_root, &canonicalize_or_self(cwd)) {
        Err(recursive_run_error(command, content_root))
    } else {
        Ok(())
    }
}

fn recursive_run_error(command: &str, root: &Path) -> anyhow::Error {
    anyhow::anyhow!(
        "Refusing to run `hyprlayer thoughts {command}` from inside your thoughts \
         repository ({}).\n\
         The current directory is the thoughts repo (or a `thoughts/` symlink that \
         points into it). Running `{command}` here would recursively manage the \
         thoughts repo and could add stray folders at its root, bypassing the commit \
         guards.\n\
         cd into the code repository you want to manage and run it from there.",
        root.display(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThoughtsDirState {
    /// An existing, empty git repo — scaffold the thoughts tree into it.
    CreateNew,
    /// An existing git repo already laid out like a thoughts repo (repos/ + global/).
    AdoptHeuristic,
    /// Not a git repository. The git backend requires the thoughts location to
    /// be an existing repo (created or cloned by the user) — reject unless --force.
    NotGitRepo,
    /// An existing git repo that is populated but isn't a thoughts repo
    /// (e.g. the user's code repo) — reject unless --force.
    Reject,
}

/// Names that don't count as content when judging whether a git repo is a
/// fresh/empty thoughts repo: VCS/OS cruft plus the metadata files (README,
/// license, .gitignore) a new or freshly cloned repo carries. Case-insensitive.
const IGNORABLE: &[&str] = &[
    ".git",
    ".gitignore",
    ".gitattributes",
    ".DS_Store",
    "Thumbs.db",
    "README",
    "README.md",
    "README.txt",
    "LICENSE",
    "LICENSE.md",
    "LICENSE.txt",
];

fn is_ignorable(name: &str) -> bool {
    IGNORABLE.iter().any(|ig| name.eq_ignore_ascii_case(ig))
}

fn is_effectively_empty(root: &Path) -> bool {
    match fs::read_dir(root) {
        Ok(entries) => !entries
            .flatten()
            .any(|e| !is_ignorable(&e.file_name().to_string_lossy())),
        // A git repo we can't enumerate isn't "empty" — don't scaffold blind.
        Err(_) => false,
    }
}

/// Classify a candidate git thoughts root. The git backend requires the path to
/// be an existing git repository: the user must create (`git init`) or clone it
/// first, so we never silently turn an arbitrary folder into a repo.
pub fn classify_git_target(root: &Path, repos_dir: &str, global_dir: &str) -> ThoughtsDirState {
    if !GitRepo::is_repo(root) {
        return ThoughtsDirState::NotGitRepo;
    }
    if root.join(repos_dir).is_dir() && root.join(global_dir).is_dir() {
        return ThoughtsDirState::AdoptHeuristic;
    }
    if is_effectively_empty(root) {
        return ThoughtsDirState::CreateNew;
    }
    ThoughtsDirState::Reject
}

/// Validate a git thoughts root before it is scaffolded. Errors unless the path
/// is an existing git repo that is either empty or already a thoughts repo.
/// `--force` bypasses the check (and lets init create + initialize the repo).
pub fn guard_git_thoughts_root(
    root: &Path,
    repos_dir: &str,
    global_dir: &str,
    force: bool,
) -> Result<()> {
    if force {
        return Ok(());
    }
    match classify_git_target(root, repos_dir, global_dir) {
        ThoughtsDirState::CreateNew | ThoughtsDirState::AdoptHeuristic => Ok(()),
        ThoughtsDirState::NotGitRepo => Err(anyhow::anyhow!(
            "{0} is not a git repository. The git backend requires the thoughts \
             location to be an existing git repository — create one with \
             `git init \"{0}\"` (or clone your thoughts repo there), then re-run init.\n\
             Re-run with --force to create and initialize it automatically.",
            root.display(),
        )),
        ThoughtsDirState::Reject => Err(anyhow::anyhow!(
            "{} doesn't look like a hyprlayer thoughts directory: it's a git \
             repository that isn't a thoughts repo (no `{}` and `{}` folders).\n\
             Point init at your thoughts repository, a new/empty repo, or re-run \
             with --force to use this directory anyway.",
            root.display(),
            repos_dir,
            global_dir,
        )),
    }
}

/// Probe well-known home-relative names for an existing thoughts repo, to seed
/// the interactive prompt. Returns the first that is recognizably a thoughts dir.
pub fn detect_existing_thoughts_dir(repos_dir: &str, global_dir: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    // ~/thoughts via get_default_thoughts_repo() so it stays one source of truth.
    let candidates = [
        get_default_thoughts_repo().ok(),
        Some(home.join("hyprlayer-thoughts")),
    ];
    candidates.into_iter().flatten().find(|cand| {
        classify_git_target(cand, repos_dir, global_dir) == ThoughtsDirState::AdoptHeuristic
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn git_init(path: &Path) {
        GitRepo::init(path).expect("git init failed in test");
    }

    // ── classify_git_target ──────────────────────────────────────────────────

    #[test]
    fn classify_absent_path_is_not_git_repo() {
        let tmp = tempdir().unwrap();
        let absent = tmp.path().join("does_not_exist");
        assert_eq!(
            classify_git_target(&absent, "repos", "global"),
            ThoughtsDirState::NotGitRepo
        );
    }

    #[test]
    fn classify_empty_non_git_dir_is_not_git_repo() {
        let tmp = tempdir().unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::NotGitRepo
        );
    }

    #[test]
    fn classify_populated_non_git_dir_is_not_git_repo() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("notes.txt"), b"hello").unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::NotGitRepo
        );
    }

    #[test]
    fn classify_empty_git_repo_is_create_new() {
        // An existing empty git repo (git init / fresh clone) — the create case.
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::CreateNew
        );
    }

    #[test]
    fn classify_git_repo_with_only_ignorable_entries_is_create_new() {
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::write(tmp.path().join(".DS_Store"), b"").unwrap();
        fs::write(tmp.path().join("Thumbs.db"), b"").unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::CreateNew
        );
    }

    #[test]
    fn classify_git_repo_with_readme_and_license_is_create_new() {
        // A GitHub "Add a README" repo is the common create-first flow — still empty.
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::write(tmp.path().join("README.md"), b"# my thoughts").unwrap();
        fs::write(tmp.path().join("LICENSE"), b"MIT").unwrap();
        fs::write(tmp.path().join(".gitignore"), b"*.tmp").unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::CreateNew
        );
    }

    #[test]
    fn classify_ignores_entries_case_insensitively() {
        // Case-preserving FS (macOS/Windows): readme.md/.ds_store must still be ignored.
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::write(tmp.path().join("readme.md"), b"x").unwrap();
        fs::write(tmp.path().join(".ds_store"), b"").unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::CreateNew
        );
    }

    #[test]
    fn classify_git_repo_with_readme_and_real_content_is_reject() {
        // README is ignorable, but a real source file alongside it is content.
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::write(tmp.path().join("README.md"), b"# code").unwrap();
        fs::write(tmp.path().join("main.rs"), b"fn main() {}").unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::Reject
        );
    }

    #[test]
    fn classify_valid_thoughts_repo_is_adopt_heuristic() {
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::create_dir_all(tmp.path().join("repos")).unwrap();
        fs::create_dir_all(tmp.path().join("global")).unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::AdoptHeuristic
        );
    }

    #[test]
    fn classify_valid_thoughts_repo_with_renamed_dirs_is_adopt_heuristic() {
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::create_dir_all(tmp.path().join("notes")).unwrap();
        fs::create_dir_all(tmp.path().join("shared-notes")).unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "notes", "shared-notes"),
            ThoughtsDirState::AdoptHeuristic
        );
    }

    #[test]
    fn classify_git_repo_without_layout_is_reject() {
        // The key dangerous case: pointing init at a populated code repo.
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::write(tmp.path().join("src.rs"), b"fn main() {}").unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::Reject
        );
    }

    #[test]
    fn classify_git_repo_with_only_one_layout_dir_is_reject() {
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        // Has repos/ but NOT global/ — should not be accepted.
        fs::create_dir_all(tmp.path().join("repos")).unwrap();
        assert_eq!(
            classify_git_target(tmp.path(), "repos", "global"),
            ThoughtsDirState::Reject
        );
    }

    // ── guard_git_thoughts_root ──────────────────────────────────────────────

    #[test]
    fn guard_errors_on_not_git_repo_without_force() {
        let tmp = tempdir().unwrap();
        let err = guard_git_thoughts_root(tmp.path(), "repos", "global", false).unwrap_err();
        assert!(
            err.to_string().contains("is not a git repository"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn guard_succeeds_on_not_git_repo_with_force() {
        let tmp = tempdir().unwrap();
        assert!(guard_git_thoughts_root(tmp.path(), "repos", "global", true).is_ok());
    }

    #[test]
    fn guard_errors_on_reject_without_force() {
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::write(tmp.path().join("src.rs"), b"fn main() {}").unwrap();
        let err = guard_git_thoughts_root(tmp.path(), "repos", "global", false).unwrap_err();
        assert!(
            err.to_string()
                .contains("doesn't look like a hyprlayer thoughts directory"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn guard_succeeds_on_reject_with_force() {
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::write(tmp.path().join("src.rs"), b"fn main() {}").unwrap();
        assert!(guard_git_thoughts_root(tmp.path(), "repos", "global", true).is_ok());
    }

    #[test]
    fn guard_succeeds_on_empty_git_repo() {
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        assert!(guard_git_thoughts_root(tmp.path(), "repos", "global", false).is_ok());
        // The guard is a read-only pre-flight check: it must not scaffold.
        assert!(!tmp.path().join("repos").exists());
        assert!(!tmp.path().join("global").exists());
    }

    #[test]
    fn guard_succeeds_on_adopt_heuristic() {
        let tmp = tempdir().unwrap();
        git_init(tmp.path());
        fs::create_dir_all(tmp.path().join("repos")).unwrap();
        fs::create_dir_all(tmp.path().join("global")).unwrap();
        assert!(guard_git_thoughts_root(tmp.path(), "repos", "global", false).is_ok());
    }

    #[test]
    fn guard_not_git_repo_message_includes_path() {
        let tmp = tempdir().unwrap();
        let target = tmp.path().join("nope");
        let err = guard_git_thoughts_root(&target, "repos", "global", false).unwrap_err();
        assert!(
            err.to_string().contains(&target.display().to_string()),
            "error should name the path: {err}"
        );
    }

    // ── ensure_cwd_outside_thoughts_repo ─────────────────────────────────────

    use crate::config::{BackendConfig, GitConfig, NotionConfig, ProfileConfig, ThoughtsConfig};

    fn git_thoughts(repo: &Path) -> ThoughtsConfig {
        ThoughtsConfig {
            user: "alice".to_string(),
            backend: BackendConfig::Git(GitConfig {
                thoughts_repo: repo.to_string_lossy().into_owned(),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn guard_blocks_cwd_at_thoughts_repo_root() {
        let repo = tempdir().unwrap();
        let thoughts = git_thoughts(repo.path());
        let err = ensure_cwd_outside_thoughts_repo(&thoughts, repo.path(), "sync").unwrap_err();
        assert!(
            err.to_string().contains("inside your thoughts repository"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn guard_blocks_cwd_nested_inside_thoughts_repo() {
        let repo = tempdir().unwrap();
        let nested = repo.path().join("repos").join("proj").join("shared");
        fs::create_dir_all(&nested).unwrap();
        let thoughts = git_thoughts(repo.path());
        assert!(ensure_cwd_outside_thoughts_repo(&thoughts, &nested, "init").is_err());
    }

    #[test]
    fn guard_allows_cwd_outside_thoughts_repo() {
        let repo = tempdir().unwrap();
        let elsewhere = tempdir().unwrap();
        let thoughts = git_thoughts(repo.path());
        assert!(ensure_cwd_outside_thoughts_repo(&thoughts, elsewhere.path(), "sync").is_ok());
    }

    /// A `thoughts/` symlink pointing into the thoughts repo must be caught
    /// via canonicalization — standing in it is standing in the repo.
    #[cfg(unix)]
    #[test]
    fn guard_blocks_cwd_via_thoughts_symlink() {
        let repo = tempdir().unwrap();
        let inner = repo.path().join("repos").join("proj").join("shared");
        fs::create_dir_all(&inner).unwrap();

        let code = tempdir().unwrap();
        let link = code.path().join("thoughts");
        std::os::unix::fs::symlink(repo.path().join("repos").join("proj"), &link).unwrap();

        let thoughts = git_thoughts(repo.path());
        // cwd is `<code>/thoughts/shared`, a symlink that resolves into the repo.
        assert!(ensure_cwd_outside_thoughts_repo(&thoughts, &link.join("shared"), "sync").is_err());
    }

    #[test]
    fn guard_checks_profile_repos_too() {
        let profile_repo = tempdir().unwrap();
        let mut thoughts = ThoughtsConfig {
            user: "alice".to_string(),
            backend: BackendConfig::Notion(NotionConfig {
                parent_page_id: "p1".to_string(),
                database_id: None,
            }),
            ..Default::default()
        };
        thoughts.profiles.insert(
            "corp".to_string(),
            ProfileConfig {
                backend: BackendConfig::Git(GitConfig {
                    thoughts_repo: profile_repo.path().to_string_lossy().into_owned(),
                    repos_dir: "repos".to_string(),
                    global_dir: "global".to_string(),
                }),
            },
        );
        // Standing inside the profile's git repo is blocked even though the
        // top-level backend is notion.
        assert!(ensure_cwd_outside_thoughts_repo(&thoughts, profile_repo.path(), "sync").is_err());
        // ...and an unrelated dir is allowed.
        let elsewhere = tempdir().unwrap();
        assert!(ensure_cwd_outside_thoughts_repo(&thoughts, elsewhere.path(), "sync").is_ok());
    }

    #[test]
    fn guard_ignores_non_git_backends() {
        let elsewhere = tempdir().unwrap();
        let thoughts = ThoughtsConfig {
            user: "alice".to_string(),
            backend: BackendConfig::Notion(NotionConfig {
                parent_page_id: "p1".to_string(),
                database_id: None,
            }),
            ..Default::default()
        };
        assert!(ensure_cwd_outside_thoughts_repo(&thoughts, elsewhere.path(), "sync").is_ok());
    }

    /// Obsidian is filesystem-backed too: standing inside its content root
    /// (vault + subpath) must be blocked, not just git repos.
    #[test]
    fn guard_blocks_cwd_inside_obsidian_content_root() {
        use crate::config::ObsidianConfig;
        let vault = tempdir().unwrap();
        let root = vault.path().join("hyprlayer");
        fs::create_dir_all(root.join("repos").join("proj")).unwrap();
        let thoughts = ThoughtsConfig {
            user: "alice".to_string(),
            backend: BackendConfig::Obsidian(ObsidianConfig {
                vault_path: vault.path().to_string_lossy().into_owned(),
                vault_subpath: Some("hyprlayer".to_string()),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
            }),
            ..Default::default()
        };
        // Inside the content root → blocked.
        assert!(ensure_cwd_outside_thoughts_repo(&thoughts, &root.join("repos"), "init").is_err());
        // Elsewhere in the vault but outside the subpath → allowed.
        assert!(ensure_cwd_outside_thoughts_repo(&thoughts, vault.path(), "init").is_ok());
    }

    // ── ensure_cwd_outside_content_root ──────────────────────────────────────

    #[test]
    fn content_root_guard_blocks_cwd_at_root() {
        let root = tempdir().unwrap();
        let err = ensure_cwd_outside_content_root(root.path(), root.path(), "init").unwrap_err();
        assert!(err.to_string().contains("inside your thoughts repository"));
    }

    #[test]
    fn content_root_guard_blocks_cwd_nested_in_root() {
        let root = tempdir().unwrap();
        let nested = root.path().join("repos").join("proj");
        fs::create_dir_all(&nested).unwrap();
        assert!(ensure_cwd_outside_content_root(root.path(), &nested, "init").is_err());
    }

    #[test]
    fn content_root_guard_allows_cwd_outside_root() {
        let root = tempdir().unwrap();
        let elsewhere = tempdir().unwrap();
        assert!(ensure_cwd_outside_content_root(root.path(), elsewhere.path(), "init").is_ok());
    }

    /// A sibling whose name merely shares a prefix with the root
    /// (`/x/thoughts` vs `/x/thoughts-repo`) must NOT be treated as inside —
    /// the canonical check is path-segment aware, not raw string prefix.
    #[test]
    fn content_root_guard_does_not_confuse_prefix_siblings() {
        let base = tempdir().unwrap();
        let root = base.path().join("thoughts");
        let sibling = base.path().join("thoughts-repo");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        assert!(ensure_cwd_outside_content_root(&root, &sibling, "init").is_ok());
    }
}
