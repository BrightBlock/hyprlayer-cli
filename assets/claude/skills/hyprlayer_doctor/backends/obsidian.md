# Backend procedure: obsidian

Obsidian backend: a local-filesystem layout rooted in an Obsidian vault, with the same `thoughts/` symlink pattern as the git backend. Critically, `sync` is a **no-op** (per `src/backends/obsidian.rs`) — Obsidian's own file-sync handles propagation, and hyprlayer doesn't run `git` against the vault. Checks are filesystem-only; total budget: ~1–2s.

Inputs from the dispatcher: `ObsidianConfig { vaultPath, vaultSubpath?, reposDir, globalDir }`, the resolved `user`, the resolved `mapped_name`.

## 1. Vault path resolves

- `ObsidianConfig.vaultPath` must be non-empty — empty would have failed `hyprlayer thoughts init` (`obsidian.rs` returns "Obsidian backend requires vaultPath in settings").
- After `expand_path` (`~` expansion), the path must exist and be a directory. ❌ otherwise — `init` enforces this too.
- Remediation: re-run `hyprlayer thoughts init` with the correct vault path, or restore the vault directory if it was moved/deleted.

## 2. Valid Obsidian vault

- `<vaultPath>/.obsidian/` should exist. `obsidian.rs` prints a yellow warning at init time when it's missing ("open it in Obsidian to make it a vault") and proceeds — so this is ⚠, not ❌. Hyprlayer's read/write will work without it, but Obsidian itself won't treat the directory as a vault.
- Remediation: open the directory in Obsidian once.

## 3. Content root

- The content root is `<vaultPath>` if `vaultSubpath` is empty/absent, otherwise `<vaultPath>/<vaultSubpath>` (per `ObsidianConfig::obsidian_root`).
- This directory must exist. ❌ otherwise. Remediation: re-run `hyprlayer thoughts init` (it does `create_dir_all` on the root).

## 4. Repo mapped

- `mapped_name` from the dispatcher must be non-null — `ObsidianBackend::init` refuses to run without it ("repo is not mapped").
- ❌ if unmapped. Remediation: `hyprlayer thoughts init` from inside this repo.

## 5. Directory layout

- These four directories must exist under the content root (per `setup_directory_structure_at` in `src/backends/common.rs`):
  - `<reposDir>/<mapped_name>/<user>/`
  - `<reposDir>/<mapped_name>/shared/`
  - `<globalDir>/<user>/`
  - `<globalDir>/shared/`
- ❌ on any missing. Remediation: re-run `hyprlayer thoughts init`.

## 6. Symlink integrity

- `<code_repo>/thoughts/` must contain three symlinks pointing into the content root (same pattern as git backend, per `setup_symlinks_into`):
  - `thoughts/<user>` → `<content_root>/<reposDir>/<mapped_name>/<user>`
  - `thoughts/shared` → `<content_root>/<reposDir>/<mapped_name>/shared`
  - `thoughts/global` → `<content_root>/<globalDir>`
- ❌ if missing, dangling, or pointing elsewhere. Remediation: re-run `hyprlayer thoughts init`.

## 7. Vault-as-git collision

- The Obsidian backend deliberately does **not** create a `.git/` directory inside the content root (verified by `init_creates_tree_and_symlinks_no_git_dir` in the test suite).
- ⚠ if `<content_root>/.git/` exists — likely a leftover from a prior `git` backend that the user switched away from. Sync semantics are now "Obsidian-only" (no-op), but `git` activity in the same tree can confuse the user.
- Remediation: decide between backends — either `rm -rf .git/` in the content root, or switch back to the git backend.

## 8. Writability

- Verify the content root is writable: create-then-remove `.hyprlayer_doctor_<unix_ts>` at the root.
- ❌ on EACCES. Remediation: `chmod`/ownership fix; for cloud-synced vaults (iCloud/Dropbox), confirm the vault has finished downloading and isn't in a "pinned to cloud" placeholder state.

## 9. Live-edit advisory

- Detect whether Obsidian.app is currently running. macOS: `osascript -e 'tell application "System Events" to (name of processes) contains "Obsidian"'`. Linux: `pgrep -f obsidian`. Windows: `tasklist | findstr /I obsidian`.
- ⚠ (not ❌) if running with this vault open — concurrent edits from Obsidian's UI and from hyprlayer can produce confusing diffs, but it's not blocking.
- Remediation: close the vault in Obsidian during bulk hyprlayer operations, or accept the risk.

## 10. Schema

- Same as the git backend: `THOUGHT_SCHEMA` is enforced in frontmatter at write time, not verifiable as a setup property.
- ⏭ Schema check skipped: enforced on write by hyprlayer.
