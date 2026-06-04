# Backend procedure: git (default)

Hyprlayer's default backend: a separate "thoughts repo" on disk, with `<code_repo>/thoughts/` symlinks pointing into a per-repo subtree. Sync is real — `git add -A`, commit, pull --rebase, push (per `src/backends/git.rs`). Checks are cheap (~1–3s) — no network unless step 6 is requested.

Inputs from the dispatcher: `GitConfig { thoughtsRepo, reposDir, globalDir }`, the resolved `user`, the resolved `mapped_name` (may be null if the current repo isn't mapped).

## 1. Repo presence

- Expand `~` in `GitConfig.thoughtsRepo` (per `expand_path` in `src/config.rs`). The resolved path must exist and be a git repo: `git -C <path> rev-parse --git-dir` returns 0.
- ❌ if the path doesn't exist or isn't a repo. Remediation: `hyprlayer thoughts init` will create it (including `git init` + initial commit if missing — per `initialize_git_if_needed`).

## 2. Repo mapped

- The current repo must be mapped: `mapped_name` from the dispatcher must be non-null. `GitBackend::init` and `sync` both refuse to run without it ("repo is not mapped" / "Thoughts not initialized for this repository").
- ❌ if unmapped. Remediation: `hyprlayer thoughts init` from inside this repo.

## 3. Symlink integrity

- `<code_repo>/thoughts/` must exist and contain three symlinks (per `setup_symlinks_into` in `src/backends/common.rs`):
  - `thoughts/<user>` → `<thoughtsRepo>/<reposDir>/<mapped_name>/<user>`
  - `thoughts/shared` → `<thoughtsRepo>/<reposDir>/<mapped_name>/shared`
  - `thoughts/global` → `<thoughtsRepo>/<globalDir>`
- For each: `readlink -f` must resolve into the expected target. ❌ if missing, dangling, or pointing elsewhere.
- Remediation: re-run `hyprlayer thoughts init` (it tears down and re-creates the symlinks).
- Note: `thoughts/searchable/` is a hardlink mirror created by `sync`, not `init`. Its absence is fine on a freshly-initialized repo; do not flag it.

## 4. Directory layout on the thoughts side

- These four directories must exist under `<thoughtsRepo>` (per `setup_directory_structure_at`):
  - `<reposDir>/<mapped_name>/<user>/`
  - `<reposDir>/<mapped_name>/shared/`
  - `<globalDir>/<user>/`
  - `<globalDir>/shared/`
- ❌ on any missing. Remediation: re-run `hyprlayer thoughts init`.

## 5. Working tree state

- Run `git -C <thoughtsRepo> status --porcelain`.
- ⚠ warn (not fail) if there are uncommitted changes — `sync` will fold them into the next commit, which is the expected behavior, but worth surfacing in case the user wants to inspect first.
- ❌ if mid-rebase/mid-merge: `<thoughtsRepo>/.git/MERGE_HEAD` or `<thoughtsRepo>/.git/rebase-*` present. `sync` will fail until resolved.

## 6. Remote configured (optional)

- Run `git -C <thoughtsRepo> remote get-url origin`. The `sync` implementation skips push/pull entirely when there's no remote (per `git.rs` line ~74) — so absence is a warning, not a failure.
- ⚠ if no remote: local-only thoughts. Push/pull will silently no-op. Remediation: `git -C <thoughtsRepo> remote add origin <url>` if cross-device sync is wanted.
- On full-report mode only: `git -C <thoughtsRepo> ls-remote --heads origin` with a 5s timeout — surfaces network/auth issues that would make `sync`'s `pull --rebase` step warn at runtime. ⚠ on failure.

## 7. GitHub CLI (`gh`)

- `gh` underpins pull-request and commit-push flows, and is also how hyprlayer reads org-managed telemetry config (`HYPRLAYER_TELEMETRY_KEY` / `HYPRLAYER_ORG_ID` repo variables — see `src/telemetry/org_config.rs`). When it's missing or unauthenticated, telemetry **silently** falls back to anonymous mode.
- Installed? `command -v gh` returns 0. ⚠ (not ❌) if absent — git-backend reads/writes still work locally; remediation: install https://cli.github.com.
- Authenticated? `gh auth status` returns 0 (it prints to **stderr**, so capture `2>&1`; a 5s timeout is plenty). ⚠ if installed but unauthed; remediation: `gh auth login`.
- Diagnostic tie-in: if telemetry seems stuck on anonymous despite an org key being set, point the user at `hyprlayer telemetry status --verbose` (or `hyprlayer telemetry config --refresh --verbose`), which traces the exact `gh`/`git` resolution step that failed.
- This is a ⚠ for the git backend, never a ❌: hyprlayer's local thoughts operations don't require `gh`.

## 8. Schema

- Schema (`THOUGHT_SCHEMA` in `src/backends/schema.rs`) is enforced at write time in markdown frontmatter, not at rest. A doctor cannot cheaply verify "every file has valid frontmatter."
- ⏭ Schema check skipped: enforced on write by hyprlayer, not verifiable as a setup property.
