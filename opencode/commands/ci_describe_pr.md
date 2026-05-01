---
description: Generate comprehensive PR descriptions following repository templates
model: {{SONNET_MODEL}}
subtask: false
---

# Generate PR Description

You are tasked with generating a comprehensive pull request description following the repository's standard template.

## Storage backend dispatch

Before step 1, run `hyprlayer storage info --json` and read the `backend` field. Use it to resolve the **template** (where the prompt lives), the **persistent record** (where the description is filed for posterity), and a **scratch file** (passed to `gh pr edit --body-file`). On all backends GitHub also receives the description, but it is not the only place it lives.

| Backend | Template | Persistent record | Scratch file for `gh pr edit` |
|---|---|---|---|
| `git` | `thoughts/shared/pr_description.md` | `thoughts/shared/prs/{number}_description.md` (commit + `hyprlayer thoughts sync`) | same path as the record |
| `obsidian` | `thoughts/shared/pr_description.md` (symlinked into the vault) | `thoughts/shared/prs/{number}_description.md` (no sync step) | same path as the record |
| `notion` | Workspace page titled `PR Description Template` (locate via `mcp__notion__search`, read via `mcp__notion__retrieve-page`) | Row in the data source under `settings.databaseId`, with `type=pr` and the required-metadata properties. Title format: `PR #{number}: {pr_title}`. Create with `mcp__notion__create-page`; on update use `mcp__notion__update-page`. | `/tmp/hyprlayer_pr_{number}_description.md` (transient; delete after `gh pr edit`) |
| `anytype` | Object named `PR Description Template` in `settings.spaceId` (locate via `mcp__anytype__API-list-objects`, read via `mcp__anytype__API-get-object`) | Anytype object with `type_key=hyprlayer_thought` and `type` property set to `pr`. Title: `PR #{number}: {pr_title}`. Use `mcp__anytype__API-create-object` / `API-update-object`. | `/tmp/hyprlayer_pr_{number}_description.md` (transient; delete after `gh pr edit`) |

If the `hyprlayer` binary is unavailable or the project is not mapped, fall back to the `git` row.

If the template cannot be located on `notion`/`anytype`, fail the run with a clear message instructing the user to create a workspace page/object named exactly `PR Description Template`. Do not silently fall back to a hardcoded template — `describe_pr_nt` is the command for that case.

For `notion`/`anytype` records, populate the schema-required fields as typed properties. The artifact `type` is `pr`. `status` is `draft` on first save and `active` once `gh pr edit` succeeds. Do not duplicate metadata as a body header block.

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

4. **Gather comprehensive PR information:**
   - Get the full PR diff: `gh pr diff {number}`
   - If you get an error about no default remote repository, instruct the user to run `gh repo set-default` and select the appropriate repository
   - Get commit history: `gh pr view {number} --json commits`
   - Review the base branch: `gh pr view {number} --json baseRefName`
   - Get PR metadata: `gh pr view {number} --json url,title,number,state`

5. **Analyze the changes thoroughly:** (ultrathink about the code changes, their architectural implications, and potential impacts)
   - Read through the entire diff carefully
   - For context, read any files that are referenced but not shown in the diff
   - Understand the purpose and impact of each change
   - Identify user-facing changes vs internal implementation details
   - Look for breaking changes or migration requirements

6. **Handle verification requirements:**
   - Look for any checklist items in the "How to verify it" section of the template
   - For each verification step:
     - If it's a command you can run (like `make check test`, `npm test`, etc.), run it
     - If it passes, mark the checkbox as checked: `- [x]`
     - If it fails, keep it unchecked and note what failed: `- [ ]` with explanation
     - If it requires manual testing (UI interactions, external services), leave unchecked and note for user
   - Document any verification steps you couldn't complete

7. **Generate the description:**
   - Fill out each section from the template thoroughly:
     - Answer each question/section based on your analysis
     - Be specific about problems solved and changes made
     - Focus on user impact where relevant
     - Include technical details in appropriate sections
     - Write a concise changelog entry
   - Ensure all checklist items are addressed (checked or explained)

8. **Persist the description:**
   - Always write the body to the scratch file (it is the input to `gh pr edit`).
   - On `git`: the scratch file IS the record. Run `hyprlayer thoughts sync` afterwards.
   - On `obsidian`: the scratch file IS the record. Skip the sync.
   - On `notion`: also create or update the database row per the dispatch table. Do not duplicate the schema-required fields inside the body — they ride as typed properties.
   - On `anytype`: also create or update the object per the dispatch table.
   - Show the user the generated description.

9. **Update the PR:**
   - `gh pr edit {number} --body-file <scratch-file>`
   - Confirm the update was successful.
   - On `notion`/`anytype`: bump the record's `status` from `draft` to `active`, then delete the `/tmp` scratch file.
   - If any verification steps remain unchecked, remind the user to complete them before merging.

## Important notes:
- This command works across different repositories — always read the local template.
- Be thorough but concise — descriptions should be scannable.
- Focus on the "why" as much as the "what".
- Include any breaking changes or migration notes prominently.
- If the PR touches multiple components, organize the description accordingly.
- Always attempt to run verification commands when possible.
- Clearly communicate which verification steps need manual testing.
