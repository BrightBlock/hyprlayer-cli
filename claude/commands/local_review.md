---
description: Set up worktree for reviewing colleague's branch
---

# Local Review

You are tasked with setting up a local review environment for a colleague's branch. This involves creating a worktree, setting up dependencies, and launching a new Claude Code session.

## Process

When invoked with a parameter like `gh_username:branchName`:

1. **Parse the input**:
   - Extract GitHub username and branch name from the format `username:branchname`
   - If no parameter provided, ask for it in the format: `gh_username:branchName`

2. **Extract ticket information**:
   - Look for ticket numbers in the branch name (e.g., `eng-1696`, `ENG-1696`)
   - Use this to create a short worktree directory name
   - If no ticket found, use a sanitized version of the branch name

3. **Determine repo info**:
   - `$REPO_NAME` = basename of current directory (e.g., `hyprlayer`, `my-sample-project`)

4. **Set up the remote and worktree**:
   - Check if the remote already exists using `git remote -v`
   - If not, add it: `git remote add USERNAME git@github.com:USERNAME/$REPO_NAME`
   - Fetch from the remote: `git fetch USERNAME`
   - Create worktree: `git worktree add -b BRANCHNAME ~/wt/$REPO_NAME/SHORT_NAME USERNAME/BRANCHNAME`

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
- If remote fetch fails, check if the username/repo exists
- If setup fails, provide the error but continue with the launch

## Example Usage

```
/local_review colleague:colleague/eng-1696-add-feature
```

This will:
- Add 'colleague' as a remote
- Create worktree at `~/wt/$REPO_NAME/eng-1696`
- Set up the environment
