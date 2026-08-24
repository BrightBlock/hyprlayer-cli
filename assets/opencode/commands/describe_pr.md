---
description: Generate comprehensive PR descriptions following repository templates
model: {{SONNET_MODEL}}
subtask: false
---

# Generate PR Description

You are tasked with generating a comprehensive pull request description following the repository's standard template.

## Storage backend dispatch

Before step 1, run `hyprlayer storage info --json` and read the `backend` field. Use it to resolve the **template** (where the prompt lives), the **persistent record** (where the description is filed for posterity, including a `status` field), and a **scratch file** (transient, body-only — passed to `gh pr edit --body-file`). The record and the scratch file must stay separate files on every backend: GitHub renders YAML frontmatter as literal text, so the scratch file can never carry it.

| Backend | Template | Persistent record | Scratch file for `gh pr edit` |
|---|---|---|---|
| `git` | `thoughts/shared/pr_description.md` | `thoughts/shared/prs/{number}_description.md`, YAML frontmatter + body (commit + `hyprlayer thoughts sync`) | `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md` (transient, body only — no frontmatter; delete after `gh pr edit`) |
| `obsidian` | `thoughts/shared/pr_description.md` (symlinked into the vault) | `thoughts/shared/prs/{number}_description.md`, YAML frontmatter + body (no sync step) | `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md` (transient, body only — no frontmatter; delete after `gh pr edit`) |
| `notion` | Workspace page titled `PR Description Template` (locate via `mcp__notion__search`, read via `mcp__notion__retrieve-page`) | Row in the data source under `settings.databaseId`, with `type=pr` and the required-metadata properties. Title format: `PR #{number}: {pr_title}`. Create with `mcp__notion__create-page`; on update use `mcp__notion__update-page`. | `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md` (transient; delete after `gh pr edit`) |
| `anytype` | Object named `PR Description Template` in `settings.spaceId` (locate via `mcp__anytype__API-list-objects`, read via `mcp__anytype__API-get-object`) | Anytype object with `type_key=hyprlayer_thought` and `type` property set to `pr`. Title: `PR #{number}: {pr_title}`. Use `mcp__anytype__API-create-object` / `API-update-object`. | `${TMPDIR:-${TEMP:-/tmp}}/hyprlayer_pr_{number}_description.md` (transient; delete after `gh pr edit`) |

If the `hyprlayer` binary is unavailable or the project is not mapped, fall back to the `git` row.

If the template cannot be located on `notion`/`anytype`, tell the user to create a workspace page/object named exactly `PR Description Template` and stop. Do not silently fall back to a hardcoded template — `describe_pr_nt` is the command for that case.

Populate every required field on the record — as YAML frontmatter on `git`/`obsidian`, as typed properties on `notion`/`anytype`. The artifact `type` is `pr`; `title` follows `PR #{number}: {pr_title}`. Do not duplicate metadata as a body header block on `notion`/`anytype` — it rides as typed properties only.

### Status lifecycle

The record's `status` tracks the PR's actual state, not just whether it's been saved once. `gh pr view --json ...state` (already fetched in step 4) is the source of truth:

| PR state | Record `status` |
|---|---|
| Record just created, `gh pr edit` not yet confirmed | `draft` |
| `state: OPEN`, `gh pr edit` succeeded | `active` |
| `state: MERGED` | `merged` |
| `state: CLOSED` (closed without merging) | `closed` |

Reconcile `status` to this table **every time this command runs** — right after opening the PR, a re-check while CI is running, or a final pass after merge — even if the description body itself doesn't need to change; this is what keeps records from getting stuck on `draft`. For `notion`/`anytype`, only set `status` to `merged`/`closed` if that value is present in `schema.options`; otherwise leave `status` at `active` and add a one-line body note instead.

Below, "the template", "the record", and "the scratch file" are placeholders for the values from this table.

## Steps to follow:

1. **Read the PR description template** at the location named for the active backend. Read it carefully to understand all sections and requirements.

2. **Identify the PR to describe:**
   - Check if the current branch has an associated PR: `gh pr view --json url,number,title,state 2>/dev/null`
   - If no PR exists for the current branch, or if on main/master, list open PRs: `gh pr list --limit 10 --json number,title,headRefName,author`
   - Ask the user which PR they want to describe

3. **Check for an existing record:**
   - On `git`/`obsidian`: read `thoughts/shared/prs/{number}_description.md` if it exists.
   - On `notion`: query the data source for a row with `type=pr` and `title` starting with `PR #{number}:`. If found, treat it as the prior version.
   - On `anytype`: list objects in the space filtered by `type_key=hyprlayer_thought` and `type=pr` with matching title prefix. If found, treat it as the prior version.
   - If a prior version is found, inform the user you'll update it (not create a new one) and consider what has changed since.

4. **Gather PR information** in two calls (don't fan out into separate `gh pr view` invocations):
   - `gh pr diff {number}` — full diff
   - `gh pr view {number} --json url,number,title,state,baseRefName,commits` — all metadata in one shot
   - If either errors with "no default remote repository", tell the user to run `gh repo set-default` and pick the right one

5. **Analyze the changes:**
   - Read through the diff
   - Identify user-facing changes vs internal implementation details
   - Look for breaking changes or migration requirements
   - For non-trivial PRs (>10 files changed or >300 added+deleted lines), think hard about architectural implications. For small diffs, skip the extended reasoning — it's wasted time on typo fixes and one-liners.
   - Only read adjacent files when the diff is genuinely ambiguous about a change. Don't pre-emptively load surrounding context.

6. **Handle verification requirements:**
   - Look for automatable checklist items in the "How to verify it" section of the template. If there are none, skip this step.
   - Otherwise, **ask the user**: "Run the automatable verification commands now, or skip and leave them for you to verify?" Wait for an answer before proceeding.
   - **If they say run:** execute each automatable command (e.g. `make check test`, `npm test`). Mark passing steps `- [x]`, failing steps `- [ ]` with a brief note of what failed.
   - **If they say skip:** leave the automatable boxes unchecked and add a one-line note in the "How to verify it" section that verification was deferred to the user.
   - Manual-only steps (UI interactions, external services) remain unchecked regardless.

7. **Generate the description:**
   - Fill out each section from the template thoroughly:
     - Answer each question/section based on your analysis
     - Be specific about problems solved and changes made
     - Focus on user impact where relevant
     - Include technical details in appropriate sections
     - Write a concise changelog entry
   - Ensure all checklist items are addressed (checked or explained)

8. **Persist the description:**
   - Always write the body — template sections only, never YAML frontmatter or any metadata block — to the scratch file (it is the input to `gh pr edit`).
   - On `git`: also write the record at `thoughts/shared/prs/{number}_description.md` as YAML frontmatter (`status: draft` on first save) followed by the same body; commit it and run `hyprlayer thoughts sync`. The record carries the frontmatter; the scratch file does not.
   - On `obsidian`: same as `git` — write the frontmatter+body record; skip the sync.
   - On `notion`: also create or update the database row per the dispatch table. Do not duplicate the schema-required fields inside the body — they ride as typed properties.
   - On `anytype`: also create or update the object per the dispatch table.
   - Show the user the generated description.

9. **Update the PR:**
   - `gh pr edit {number} --body-file <scratch-file>` — skip this if `state` is already `MERGED` or `CLOSED` (rewriting a merged/closed PR's body is unusual; just reconcile `status` below).
   - Confirm the update was successful.
   - Reconcile the record's `status` to the live PR `state` per the Status lifecycle table above, on **every** backend (not just `notion`/`anytype`): for `git`/`obsidian`, edit the record file's frontmatter directly, then — for `git` — run `hyprlayer thoughts sync` again.
   - Delete the transient scratch file (every backend).
   - If any verification steps remain unchecked, remind the user to complete them before merging.

## Important notes:
- This command works across different repositories — always read the local template.
- Be thorough but concise — descriptions should be scannable.
- Focus on the "why" as much as the "what".
- Include any breaking changes or migration notes prominently.
- If the PR touches multiple components, organize the description accordingly.
- Ask before running verification commands; never auto-run them.
- Clearly communicate which verification steps need manual testing.
- **Never** `git add thoughts` or any path under it from this repo, and never include those paths in a commit. The `thoughts/` directory contains symlinks to a separate repo managed by `hyprlayer thoughts sync` — use that command for thoughts, not `git add`/`git commit`. If a commit elsewhere in this flow is required (e.g. the `git`-backend record write), stage explicit file paths only; never `git add .` or `git add -A`.
