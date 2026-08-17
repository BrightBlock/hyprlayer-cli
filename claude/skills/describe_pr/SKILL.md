---
name: describe_pr
description: Generate comprehensive PR descriptions following the repository's pr_description.md template. Use when the user asks to describe an existing PR (including running its automated verification commands and updating the PR via gh pr edit).
model: sonnet
allowed-tools: Bash, Read, Write, Agent, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# Generate PR Description

You are tasked with generating a comprehensive pull request description following the repository's standard template.

## PR description dispatch

Read `~/.claude/skills/_thoughts/pr-description.md` and follow it for the per-backend template, record, and scratch-file locations and the workflow that ties them together. Read `~/.claude/skills/_thoughts/required-metadata.md` for the schema-required fields. For this command: artifact `type` is `pr`; the title is `PR #{number}: {pr_title}` once the PR exists.

The numbered steps below fold the dispatched read/write into the broader interactive flow (template + PR identification + diff analysis + verification + edit + cleanup).

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
   - If either errors with "no default remote repository", tell the user to run `gh repo set-default` and pick the right one

5. **Analyze the changes:**
   - Read through the diff
   - Identify user-facing changes vs internal implementation details
   - Look for breaking changes or migration requirements
   - For non-trivial PRs (>10 files changed or >300 added+deleted lines), think hard about architectural implications. For small diffs, skip the extended reasoning — it's wasted time on typo fixes and one-liners.
   - Only read adjacent files when the diff is genuinely ambiguous about a change. Don't pre-emptively load surrounding context.

6. **Handle verification requirements:**
   - Look for automatable checklist items in the "How to verify it" section of the template. If there are none (or the template has no such section), skip this step.
   - Otherwise, **ask the user**: "Run the automatable verification commands now, or skip and leave them for you to verify?" Wait for an answer before proceeding.
   - **If they say run:** execute each automatable command (e.g. `make check test`, `npm test`). Mark passing steps `- [x]`, failing steps `- [ ]` with a brief note of what failed.
   - **If they say skip:** leave the automatable boxes unchecked and add a one-line note in the "How to verify it" section that verification was deferred to the user.
   - Manual-only steps (UI interactions, external services) remain unchecked regardless.

7. **Generate the description:**
   - **For a PR of any size, delegate the write-up to the `herald` agent** (see `~/.claude/skills/_thoughts/subagent-guide.md`). Hand it the PR number, the template (path or body), the repo root, and — if step 6 ran verification — which commands were run and what they returned. It reads the diff and commits itself and returns the finished body. Keeping a large diff out of this context is most of the value; write inline only for a trivial one-file PR.
   - The herald marks a verification box `- [x]` only for commands you told it passed. Check that its checklist matches what actually happened in step 6 before you use the body.
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
   - If the PR has since merged or closed, mention that this record's `status` now reflects that — or, if the user reports a merge later in the session, offer to re-run this skill to finalize `status: merged`.

## Important notes:
- This command works across different repositories — always read the local template.
- Be thorough but concise — descriptions should be scannable.
- Focus on the "why" as much as the "what".
- Include any breaking changes or migration notes prominently.
- If the PR touches multiple components, organize the description accordingly.
- Ask before running verification commands; never auto-run them.
- Clearly communicate which verification steps need manual testing.
- **Never** `git add thoughts` or any path under it from this repo, and never include those paths in a commit. The `thoughts/` directory contains symlinks to a separate repo managed by `hyprlayer thoughts sync` — use that command for thoughts, not `git add`/`git commit`. If a commit elsewhere in this flow is required (e.g. the `git`-backend record write), stage explicit file paths only; never `git add .` or `git add -A`.
