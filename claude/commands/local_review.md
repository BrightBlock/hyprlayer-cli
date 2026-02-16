---
description: Set up worktree for reviewing colleague's branch
---

# Local Review

You are tasked with setting up a local review environment for a branch. This involves creating a worktree, setting up dependencies, and launching a new Claude Code session.

## Process

When invoked with a branch name or PR number:

1. **Parse the input**:
   - If a PR number is provided (e.g., `123`), resolve it to a branch name: `gh pr view 123 --json headRefName -q .headRefName`
   - If a branch name is provided, use it directly
   - If no parameter provided, ask for a branch name or PR number

2. **Extract ticket information**:
   - Look for ticket numbers in the branch name (e.g., `eng-1696`, `ENG-1696`)
   - Use this to create a short worktree directory name
   - If no ticket found, use a sanitized version of the branch name

3. **Determine repo info**:
   - `$REPO_NAME` = basename of current directory (e.g., `hyprlayer`, `my-sample-project`)

4. **Set up the worktree**:
   - Fetch latest from origin: `git fetch origin`
   - Create worktree: `git worktree add -b review/BRANCHNAME ~/wt/$REPO_NAME/SHORT_NAME origin/BRANCHNAME`

5. **Configure the worktree**:
   - Copy Claude local settings if they exist: `mkdir -p WORKTREE/.claude && cp .claude/settings.local.json WORKTREE/.claude/`
   - Detect and run the appropriate setup command:
     - If `Makefile` exists with a `setup` target: `make -C WORKTREE setup`
     - Else if `package.json` exists: `cd WORKTREE && npm install`
     - Else if `pyproject.toml` exists: `cd WORKTREE && pip install -e .`
     - Else if `go.mod` exists: `cd WORKTREE && go mod download`
     - Else if a `*.sln` or `*.csproj` file exists: `cd WORKTREE && dotnet restore`
     - Otherwise, skip dependency setup
   - Initialize thoughts: `cd WORKTREE && hyprlayer thoughts init --directory $REPO_NAME`

## Error Handling

- If worktree already exists, inform the user they need to remove it first
- If the branch doesn't exist on origin, check for typos and suggest similar branch names
- If setup fails, provide the error but continue with the launch

## Example Usage

```
/local_review eng-1696-add-feature
/local_review 42
```

This will:
- Fetch from origin and create worktree at `~/wt/$REPO_NAME/eng-1696`
- Set up the environment
