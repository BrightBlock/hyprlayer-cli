---
description: Create git commits with user approval and no Claude attribution
model: {{SONNET_MODEL}}
subtask: false
---

# Commit Changes

You are tasked with creating git commits for the changes made during this session.

## Process:

1. **Ensure you're on a feature branch:**
   - Run `git branch --show-current` to check the current branch
   - If on `main` or `master`, you MUST create a feature branch before committing
   - Use gitflow naming conventions based on the nature of the changes:
     - `feature/<description>` — New functionality
     - `fix/<description>` — Bug fixes
     - `chore/<description>` — Maintenance, docs, config changes
     - `refactor/<description>` — Code restructuring without behavior change
   - Use kebab-case for the description (e.g., `feature/add-user-auth`, `fix/null-pointer-on-login`)
   - If a JIRA ticket is associated, include it (e.g., `feature/ENG-1234-add-user-auth`)
   - Create the branch with `git checkout -b <branch-name>`
   - If already on a feature branch, continue on it

2. **Think about what changed:**
   - Review the conversation history and understand what was accomplished
   - Run `git status` to see current changes
   - Run `git diff` to understand the modifications
   - Consider whether changes should be one commit or multiple logical commits

3. **Plan your commit(s):**
   - Identify which files belong together
   - Draft clear, descriptive commit messages
   - Use imperative mood in commit messages
   - Focus on why the changes were made, not just what

4. **Present your plan to the user:**
   - List the files you plan to add for each commit
   - Show the commit message(s) you'll use
   - Ask: "I plan to create [N] commit(s) with these changes. Shall I proceed?"

5. **Execute upon confirmation:**
   - Use `git add` with specific files (never use `-A` or `.`)
   - Create commits with your planned messages
   - Show the result with `git log --oneline -n [number]`

## Important:
- **NEVER add co-author information or Claude attribution**
- Commits should be authored solely by the user
- Do not include any "Generated with Claude" messages
- Do not add "Co-Authored-By" lines
- Write commit messages as if the user wrote them

## Remember:
- You have the full context of what was done in this session
- Group related changes together
- Keep commits focused and atomic when possible
- The user trusts your judgment - they asked you to commit
