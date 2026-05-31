use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::get_default_thoughts_repo;
use crate::git_ops::GitRepo;

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

/// Names that don't count as "real content" when judging whether a git repo is
/// an empty/fresh thoughts repo: VCS/OS cruft plus the metadata files a freshly
/// created repo carries — GitHub's "Add a README", a license, .gitignore. A repo
/// cloned with just these is the canonical "I made my thoughts repo" state and
/// must classify as CreateNew, not Reject. Matched case-insensitively so
/// case-preserving filesystems (macOS/Windows) don't leak a `readme.md` or
/// `.ds_store` through as content.
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
        // We already confirmed this is a git repo but cannot enumerate it (e.g.
        // an unreadable working tree). Don't assume empty — fall through to
        // Reject rather than scaffold into a populated repo we couldn't inspect.
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
    // Reuse get_default_thoughts_repo() for ~/thoughts so the probed default and
    // the prompt fallback stay one source of truth.
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
        // An existing, empty git repo (e.g. `git init` or a fresh clone) is the
        // legitimate create case — scaffold the thoughts tree into it.
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
        // GitHub's "Initialize this repository with a README" (and a license)
        // is the most common way users create the repo the git backend now
        // requires — it must still classify as empty/CreateNew, not Reject.
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
        // On case-preserving filesystems (macOS/Windows) a `readme.md` or
        // `.ds_store` must not leak through as content and flip CreateNew→Reject.
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
}
