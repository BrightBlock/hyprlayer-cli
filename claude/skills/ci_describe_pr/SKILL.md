---
name: ci_describe_pr
description: Generate comprehensive PR descriptions following the repository's pr_description.md template, syncing the result back to the PR. Use when the user asks to describe a PR in CI/non-interactive mode (assumes a PR description template exists in the configured thoughts backend).
model: sonnet
allowed-tools: Bash, Read, Write, Agent, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# Generate PR Description

You are tasked with generating a comprehensive pull request description following the repository's standard template.

## PR description dispatch

Read `~/.claude/skills/_thoughts/pr-description.md` and follow it for the per-backend template, record, and scratch-file locations and the workflow that ties them together. Read `~/.claude/skills/_thoughts/required-metadata.md` for the schema-required fields. For this command: artifact `type` is `pr`; the title is `PR #{number}: {pr_title}` once the PR exists.

In CI mode there is no interactive prompt — fail the run with a clear message if the template cannot be located on `notion`/`anytype` (do not silently fall back to a hardcoded template; `describe_pr_nt` is the skill for that case).

The numbered steps below fold the dispatched read/write into the broader flow (template + PR identification + diff analysis + verification + edit + cleanup).

## Steps to follow:

1. **Read the PR description template** at the location named for the active backend (per the dispatch). Read it carefully to understand all sections and requirements.

2. **Identify the PR to describe:**
   - Check if the current branch has an associated PR: `gh pr view --json url,number,title,state 2>/dev/null`
   - If no PR exists for the current branch, or if on main/master, list open PRs: `gh pr list --limit 10 --json number,title,headRefName,author`
   - Ask the user which PR they want to describe

3. **Check for an existing record** for this PR number, per the dispatch's "locate any prior record" step. If a prior version is found, inform the user you'll update it (not create a new one) and consider what has changed since.

4. **Gather PR information** in two calls (don't fan out into separate `gh pr view` invocations):
   - `gh pr diff {number}` — full diff
   - `gh pr view {number} --json url,number,title,state,baseRefName,commits` — all metadata in one shot
   - If either errors with "no default remote repository", fail the run with that message — CI mode cannot prompt for `gh repo set-default`.

5. **Analyze the changes:**
   - Read through the diff
   - Identify user-facing changes vs internal implementation details
   - Look for breaking changes or migration requirements
   - For non-trivial PRs (>10 files changed or >300 added+deleted lines), think hard about architectural implications. For small diffs, skip the extended reasoning.
   - Only read adjacent files when the diff is genuinely ambiguous about a change.

6. **Handle verification requirements** (CI mode runs them automatically — no interactive prompt available):
   - Look for automatable checklist items in the "How to verify it" section of the template. If there are none, skip this step.
   - For each automatable command (e.g. `make check test`, `npm test`), run it. Mark passing steps `- [x]`, failing steps `- [ ]` with a brief note of what failed.
   - Manual-only steps (UI interactions, external services) remain unchecked.

7. **Generate the description:**
   - **Delegate the write-up to the `herald` agent** (see `~/.claude/skills/_thoughts/subagent-guide.md`). Hand it the PR number, the template (path or body), the repo root, and which verification commands were run with their results. It returns the finished body. If the Agent tool is unavailable in this CI environment, write the description inline — do not fail the run over it.
   - Fill out each section from the template thoroughly:
     - Answer each question/section based on your analysis
     - Be specific about problems solved and changes made
     - Focus on user impact where relevant
     - Include technical details in appropriate sections
     - Write a concise changelog entry
   - Ensure all checklist items are addressed (checked or explained)

8. **Persist the description** per the dispatch's "persist the description" step (scratch file always; record per backend; sync on `git`; create/update database row or object on `notion`/`anytype`). Show the user the generated description.

9. **Update the PR:**
   - `gh pr edit {number} --body-file <scratch-file>` (skip if the PR is already merged/closed — see the dispatch's status lifecycle table)
   - Confirm the update was successful.
   - Promote and clean up per the dispatch (delete the transient scratch file at `${TMPDIR:-${TEMP:-/tmp}}` on every backend; reconcile `status` to the live PR state — `draft`→`active`, or `merged`/`closed` — on **every** backend, not just `notion`/`anytype`).
   - If any verification steps remain unchecked, remind the user to complete them before merging.

## Important notes:
- This command works across different repositories — always read the local template.
- Be thorough but concise — descriptions should be scannable.
- Focus on the "why" as much as the "what".
- Include any breaking changes or migration notes prominently.
- If the PR touches multiple components, organize the description accordingly.
- CI mode runs verification commands automatically (no interactive prompt available).
- Clearly communicate which verification steps need manual testing.
- **Never** `git add thoughts` or any path under it from this repo, and never include those paths in a commit. The `thoughts/` directory contains symlinks to a separate repo managed by `hyprlayer thoughts sync`. If a commit is required for the `git`-backend record, stage explicit file paths only; never `git add .` or `git add -A`.
