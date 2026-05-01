---
name: describe_pr
description: Generate comprehensive PR descriptions following the repository's pr_description.md template. Use when the user asks to describe an existing PR (including running its automated verification commands and updating the PR via gh pr edit).
allowed-tools: Bash, Read, Write, mcp__claude_ai_Notion__*, mcp__anytype__*
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

8. **Persist the description** per the dispatch's "persist the description" step (scratch file always; record per backend; sync on `git`; create/update database row or object on `notion`/`anytype`). Show the user the generated description.

9. **Update the PR:**
   - `gh pr edit {number} --body-file <scratch-file>`
   - Confirm the update was successful.
   - Promote and clean up per the dispatch (bump `status` from `draft` to `active`, delete the `/tmp` scratch file on `notion`/`anytype`).
   - If any verification steps remain unchecked, remind the user to complete them before merging.

## Important notes:
- This command works across different repositories — always read the local template.
- Be thorough but concise — descriptions should be scannable.
- Focus on the "why" as much as the "what".
- Include any breaking changes or migration notes prominently.
- If the PR touches multiple components, organize the description accordingly.
- Always attempt to run verification commands when possible.
- Clearly communicate which verification steps need manual testing.
