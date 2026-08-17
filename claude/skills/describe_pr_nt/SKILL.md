---
name: describe_pr_nt
description: Generate comprehensive PR descriptions using a built-in template, no-thoughts variant (writes to the OS temp directory instead of thoughts/shared/prs). Use when the user asks to describe a PR in a repo without a thoughts/shared/pr_description.md template.
allowed-tools: Bash, Read
---

# Generate PR Description

You are tasked with generating a comprehensive pull request description following the repository's standard template.

## Steps to follow:

1. **Read the PR description template:**

    - Use the following PR description template:

        ```md
        ## What problem(s) was I solving?

        ## What user-facing changes did I ship?

        ## How I implemented it

        ## How to verify it

        ### Manual Testing

        ## Description for the changelog
        ```

    - Read the template carefully to understand all sections and requirements

2. **Identify the PR to describe:**
   - Check if the current branch has an associated PR: `gh pr view --json url,number,title,state 2>/dev/null`
   - If no PR exists for the current branch, or if on main/master, list open PRs: `gh pr list --limit 10 --json number,title,headRefName,author`
   - Ask the user which PR they want to describe

3. **Check for existing description:**
   - Check if `${TMPDIR:-${TEMP:-/tmp}}/{repo_name}/prs/{number}_description.md` already exists
   - If it exists, read it and inform the user you'll be updating it
   - Consider what has changed since the last description was written

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

8. **Save and sync the description:**
   - Write the completed description to `${TMPDIR:-${TEMP:-/tmp}}/{repo_name}/prs/{number}_description.md`
   - Show the user the generated description

9. **Update the PR:**
   - Update the PR description directly: `gh pr edit {number} --body-file "${TMPDIR:-${TEMP:-/tmp}}/{repo_name}/prs/{number}_description.md"`
   - Confirm the update was successful
   - If any verification steps remain unchecked, remind the user to complete them before merging

## Important notes:
- This command works across different repositories - always read the local template
- Be thorough but concise - descriptions should be scannable
- Focus on the "why" as much as the "what"
- Include any breaking changes or migration notes prominently
- If the PR touches multiple components, organize the description accordingly
- Ask before running verification commands; never auto-run them
- Clearly communicate which verification steps need manual testing
- **Never** `git add thoughts` or any path under it, and never include those paths in a commit. The `thoughts/` directory may contain symlinks to a separate repo. If a commit is needed in the project repo, stage explicit file paths only; never `git add .` or `git add -A`.
