# GitHub permalinks step

After writing an artifact that references local file paths, upgrade those references to GitHub permalinks when the commit is on a pushed branch. Skip this step if the branch is local-only.

```bash
git branch --show-current
git status
```

If on `main`/`master` or the current commit is already pushed:

```bash
gh repo view --json owner,name
```

Build permalinks of the form `https://github.com/{owner}/{repo}/blob/{commit}/{file}#L{line}`. Replace the local file references in the artifact body with these permalinks.

Skip silently if `gh` is not available or the repo has no GitHub remote.
