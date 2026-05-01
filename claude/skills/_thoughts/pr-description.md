# PR description dispatch

This is the shared protocol for the `describe_pr` and `ci_describe_pr` skills. Both produce a PR description body that is passed to `gh pr edit --body-file` and (where the backend supports it) also persisted as a typed thoughts artifact.

The skill that loaded this file already knows the PR `{number}` and `{pr_title}`.

> Path convention from `_thoughts/storage-backend.md` applies here too: `thoughts/shared/...` paths are literal on `git`/`obsidian`; on `notion`/`anytype`, substitute the matching identifier returned by `hyprlayer storage info`.

Run `hyprlayer storage info --json` and read the `backend` field. Use the table below to resolve three values:

- **template** — where the prompt sections live
- **record** — where the description is filed for posterity
- **scratch file** — the path passed to `gh pr edit --body-file`

| Backend | Template | Record | Scratch file for `gh pr edit` |
|---|---|---|---|
| `git` | `thoughts/shared/pr_description.md` | `thoughts/shared/prs/{number}_description.md` (commit + `hyprlayer thoughts sync`) | same path as the record |
| `obsidian` | `thoughts/shared/pr_description.md` (symlinked into the vault) | `thoughts/shared/prs/{number}_description.md` (no sync step) | same path as the record |
| `notion` | Workspace page titled `PR Description Template` (locate via `mcp__claude_ai_Notion__notion-search`, read via `mcp__claude_ai_Notion__notion-fetch`) | Row in the data source under `settings.databaseId`, with `type=pr` and the required-metadata properties from `_thoughts/required-metadata.md`. Title: `PR #{number}: {pr_title}`. Create with `mcp__claude_ai_Notion__notion-create-pages`; on update use `mcp__claude_ai_Notion__notion-update-page`. | `/tmp/hyprlayer_pr_{number}_description.md` (transient; delete after `gh pr edit`) |
| `anytype` | Object named `PR Description Template` in `settings.spaceId` (locate via `mcp__anytype__API-list-objects`, read via `mcp__anytype__API-get-object`) | Anytype object with `type_key=hyprlayer_thought` and `type` property set to `pr`. Title: `PR #{number}: {pr_title}`. Follow the create / update protocol in `_thoughts/storage-backend.md`. | `/tmp/hyprlayer_pr_{number}_description.md` (transient; delete after `gh pr edit`) |

If the `hyprlayer` binary is unavailable or the project is not mapped, fall back to the `git` row.

If the template cannot be located on `notion`/`anytype`, tell the user to create a workspace page/object named exactly `PR Description Template` and stop. Do not silently fall back to a hardcoded template — `describe_pr_nt` is the skill for that case.

## Required metadata for the record

For `notion`/`anytype`, populate every required field from `_thoughts/required-metadata.md` as a typed property:

- `type` is `pr`.
- `status` is `draft` on first save and `active` once `gh pr edit` succeeds.
- `title` follows the `PR #{number}: {pr_title}` convention (overrides the generic title-format rule for this artifact).

Do not duplicate metadata as a body header block — it rides as typed properties only.

## Workflow

1. **Locate the template** at the location named for the active backend; read it.
2. **Locate any prior record** for this PR number:
   - `git`/`obsidian`: read the file at the record path if it exists.
   - `notion`: query the data source for a row with `type=pr` and `title` starting with `PR #{number}:`.
   - `anytype`: list objects filtered by `type_key=hyprlayer_thought` and `type=pr` with matching title prefix.
   If found, treat it as the prior version and update rather than creating a duplicate.
3. **Fill out the template** using the PR diff, commit history, and verification steps. Tick automatable checklist items as you run them.
4. **Persist the description**:
   - Always write the body to the scratch file (it is the input to `gh pr edit`).
   - On `git`: the scratch file IS the record; run `hyprlayer thoughts sync`.
   - On `obsidian`: the scratch file IS the record; skip the sync.
   - On `notion`: also create or update the database row.
   - On `anytype`: also create or update the object.
5. **Update the PR** with `gh pr edit {number} --body-file <scratch-file>`.
6. **Promote and clean up** on `notion`/`anytype`: bump the record's `status` from `draft` to `active`, then delete the `/tmp` scratch file.
