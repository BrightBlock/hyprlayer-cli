# PR description dispatch

This is the shared protocol for the `describe_pr` skills. They produce a PR description body that is passed to `gh pr edit --body-file` and (where the backend supports it) also persisted as a typed thoughts artifact.

The skill that loaded this file already knows the PR `{number}` and `{pr_title}`.

> Path convention from `_thoughts/storage-backend.md` applies here too: `thoughts/shared/...` paths are literal on `git`/`obsidian`; on `notion`/`anytype`, substitute the matching identifier returned by `hyprlayer storage info`.

Run `hyprlayer storage info --json` and read the `backend` field. Use the table below to resolve three values:

- **template** — where the prompt sections live
- **record** — where the description is filed for posterity
- **scratch file** — the path passed to `gh pr edit --body-file`

| Backend | Template | Record | Scratch file for `gh pr edit` |
|---|---|---|---|
| `git` | `thoughts/shared/pr_description.md` | `thoughts/shared/prs/{number}_description.md` (commit + `hyprlayer thoughts sync`) | `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md` (transient, body only — no frontmatter; delete after `gh pr edit`) |
| `obsidian` | `thoughts/shared/pr_description.md` (symlinked into the vault) | `thoughts/shared/prs/{number}_description.md` (no sync step) | `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md` (transient, body only — no frontmatter; delete after `gh pr edit`) |
| `notion` | Workspace page titled `PR Description Template` (locate via `mcp__claude_ai_Notion__notion-search`, read via `mcp__claude_ai_Notion__notion-fetch`) | Row in the data source under `settings.databaseId`, with `type=pr` and the required-metadata properties from `_thoughts/required-metadata.md`. Title: `PR #{number}: {pr_title}`. Create with `mcp__claude_ai_Notion__notion-create-pages`; on update use `mcp__claude_ai_Notion__notion-update-page`. | `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md` (transient; delete after `gh pr edit`) |
| `anytype` | Object named `PR Description Template` in `settings.spaceId` (locate via `mcp__anytype__API-list-objects`, read via `mcp__anytype__API-get-object`) | Anytype object with `type_key=hyprlayer_thought` and `type` property set to `pr`. Title: `PR #{number}: {pr_title}`. Follow the create / update protocol in `_thoughts/storage-backend.md`. | `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md` (transient; delete after `gh pr edit`) |

If the `hyprlayer` binary is unavailable or the project is not mapped, fall back to the `git` row.

If the template cannot be located on `notion`/`anytype`, tell the user to create a workspace page/object named exactly `PR Description Template` and stop. Do not silently fall back to a hardcoded template — `describe_pr_nt` is the skill for that case.

## Required metadata for the record

Populate every required field from `_thoughts/required-metadata.md` — as YAML frontmatter on `git`/`obsidian`, as typed properties on `notion`/`anytype`:

- `type` is `pr`.
- `title` follows the `PR #{number}: {pr_title}` convention (overrides the generic title-format rule for this artifact).
- `status` follows the lifecycle below **on every backend**, not just `notion`/`anytype`.

On `notion`/`anytype`, do not duplicate metadata as a body header block — it rides as typed properties only. `git`/`obsidian` keep using YAML frontmatter as usual.

## Status lifecycle

The record's `status` tracks the PR's actual state, not just whether it's been saved once. `gh pr view --json ...state` (already fetched in workflow step 4) is the source of truth:

| PR state | Record `status` |
|---|---|
| Record just created, `gh pr edit` not yet confirmed | `draft` |
| `state: OPEN`, `gh pr edit` succeeded | `active` |
| `state: MERGED` | `merged` |
| `state: CLOSED` (closed without merging) | `closed` |

This lookup happens **every time this skill runs**, not just the first time — whether that run is right after opening the PR, a re-check while CI is running, or a final pass after merge. Re-read `state` and reconcile `status` to match the table even if the description body itself doesn't need to change; this is what keeps records from getting stuck on `draft`.

For `notion`/`anytype`, only set `status` to `merged`/`closed` if that value is present in `schema.options`; if it isn't, leave `status` at `active` and add a one-line body note instead of inventing an unsupported option.

## Workflow

1. **Locate the template** at the location named for the active backend; read it.
2. **Locate any prior record** for this PR number:
   - `git`/`obsidian`: read the file at the record path if it exists.
   - `notion`: query the data source for a row with `type=pr` and `title` starting with `PR #{number}:`.
   - `anytype`: list objects filtered by `type_key=hyprlayer_thought` and `type=pr` with matching title prefix.
   If found, treat it as the prior version and update rather than creating a duplicate.
3. **Fill out the template** using the PR diff, commit history, and verification steps. Tick automatable checklist items as you run them.
4. **Persist the description**:
   - Always write the body — template sections only, **never** YAML frontmatter or any metadata block — to the scratch file. It is the input to `gh pr edit`, and GitHub renders frontmatter as literal text, so the scratch file must stay metadata-free on every backend.
   - On `git`: also write the record at `thoughts/shared/prs/{number}_description.md` as YAML frontmatter (per `_thoughts/required-metadata.md`) followed by the same body; commit it and run `hyprlayer thoughts sync`. The record carries the frontmatter; the scratch file does not.
   - On `obsidian`: same as `git` — write the frontmatter+body record at `thoughts/shared/prs/{number}_description.md`; skip the sync.
   - On `notion`: also create or update the database row (metadata rides as typed properties).
   - On `anytype`: also create or update the object (metadata rides as typed properties).
5. **Update the PR** with `gh pr edit {number} --body-file <scratch-file>` — **skip this** if `state` is already `MERGED` or `CLOSED` (rewriting a merged/closed PR's body is unusual; just reconcile `status` in step 6). Otherwise confirm the command exited 0 before proceeding; transient failures like TLS timeouts are common, so retry up to twice with a short delay if it errors. If `gh pr edit` ultimately fails, halt — do not run step 6.
6. **Promote and clean up**, **only after step 5 succeeded (or was skipped because the PR is already merged/closed)**:
   - On **every** backend: delete the transient scratch file at `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md`. It was only ever the input to `gh pr edit`, never the record, so nothing of value is lost.
   - On **every** backend: set the record's `status` to match the live PR `state` per the Status lifecycle table above (`draft` → `active` on a successful edit of an open PR; `merged`/`closed` when the PR has already resolved). For `git`/`obsidian` this means editing the YAML frontmatter of the record file directly, then — for `git` — running `hyprlayer thoughts sync` again so the promoted status is pushed.
   Doing the status update without step 5 having succeeded (for an open PR) leaves a `status: active` record advertising a synced PR while the body is still the placeholder.
