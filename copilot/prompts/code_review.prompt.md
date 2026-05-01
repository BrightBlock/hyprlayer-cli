---
description: Adversarial review of the current branch's diff. Spawns @adversarial-reviewer on gpt-5-codex for a cross-model second opinion.
agent: agent
---

# Code Review (Adversarial)

Drive an adversarial review of the current branch's diff. Spawns the `@adversarial-reviewer` agent with a fresh context window running on `gpt-5-codex` for a true cross-model second opinion.

## Arguments

- `/code_review` — review the diff against the auto-detected base branch
- `/code_review <focus>` — narrow the review (e.g., `security`, `concurrency`, `error handling`)

Strip flags out of the prompt text before constructing the focus argument.

## Step 0: Minimum hyprlayer version

```bash
hyprlayer_install_hint() {
  if   command -v brew   >/dev/null 2>&1; then echo "brew tap brightblock/tap && brew install hyprlayer"
  elif command -v scoop  >/dev/null 2>&1; then echo "scoop bucket add brightblock https://github.com/BrightBlock/scoop-bucket && scoop install hyprlayer"
  elif command -v winget >/dev/null 2>&1; then echo "winget install BrightBlock.Hyprlayer"
  elif command -v yay    >/dev/null 2>&1; then echo "yay -S hyprlayer-bin"
  elif command -v paru   >/dev/null 2>&1; then echo "paru -S hyprlayer-bin"
  else echo "see https://github.com/BrightBlock/hyprlayer-cli#install"; fi
}
hyprlayer_upgrade_hint() {
  if   command -v brew   >/dev/null 2>&1; then echo "brew upgrade hyprlayer"
  elif command -v scoop  >/dev/null 2>&1; then echo "scoop update hyprlayer"
  elif command -v winget >/dev/null 2>&1; then echo "winget upgrade BrightBlock.Hyprlayer"
  elif command -v yay    >/dev/null 2>&1; then echo "yay -Syu hyprlayer-bin"
  elif command -v paru   >/dev/null 2>&1; then echo "paru -Syu hyprlayer-bin"
  else echo "see https://github.com/BrightBlock/hyprlayer-cli#install"; fi
}

HYPR_VER=$(hyprlayer --version 2>/dev/null | awk '{print $2}' | cut -d'(' -f1 | tr -d ' ')
if [ -z "$HYPR_VER" ]; then
  echo "hyprlayer not found. Install: $(hyprlayer_install_hint)"
  exit 1
fi
if ! awk -v v="$HYPR_VER" 'BEGIN { split(v,a,"."); if (a[1]<1 || (a[1]==1 && a[2]<5) || (a[1]==1 && a[2]==5 && a[3]<2)) exit 1 }'; then
  echo "hyprlayer >= 1.5.2 required (have $HYPR_VER). Upgrade: $(hyprlayer_upgrade_hint)"
  exit 1
fi
```

## Step 1: Detect repo root and base branch

```bash
_REPO_ROOT=$(git rev-parse --show-toplevel) || { echo "not in a git repo"; exit 1; }
cd "$_REPO_ROOT"
```

Detect the base branch in this order:
1. `gh pr view --json baseRefName -q .baseRefName` (if a PR exists for the current branch)
2. `gh repo view --json defaultBranchRef -q .defaultBranchRef.name`
3. `git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's|refs/remotes/origin/||'`
4. `main` if `origin/main` exists, else `master`

Use the result as `<base>` below.

Confirm there is a diff to review:

```bash
git fetch origin <base> 2>/dev/null
git diff origin/<base>...HEAD --stat | tail -1
```

If no changes, stop with: "no diff against origin/<base> — nothing to review."

## Step 2: Spawn the adversarial-reviewer agent

Build the prompt and dispatch with `@adversarial-reviewer`:

```
@adversarial-reviewer Review the diff on the current branch against origin/<base>.

Diff range: origin/<base>...HEAD
Repo root: <_REPO_ROOT>
<if focus was provided:> Focus: <focus>

Run `git diff origin/<base>...HEAD` yourself to see the full diff. Read the
surrounding context of any changed file before flagging an issue. Report
ordered by severity per your standard format. If you find nothing
significant, say so in one line.
```

The agent's output is the body of the review — present it verbatim in Step 3.

## Step 3: Present output

First line tells the user which model the agent ran on:

```
tier: copilot subagent (gpt-5-codex, cross-model)
```

Then the review body verbatim. No banners, no separator characters, no decoration.

If you genuinely disagree with a specific finding, append at most one:

> note: orchestrator disagrees with subagent on <X> because <Y>.

Don't fabricate disagreement.

## Rules

- **Read-only.** The `@adversarial-reviewer` agent has no write or edit tools. This command never modifies files.
- **Verbatim output.** Don't summarize, truncate, or soften findings before presenting. Synthesis comes after, not instead of.
- **One pass per invocation.** Don't loop. Re-run with a tighter focus to drill deeper.

## Error Handling

| Symptom | Action |
|---|---|
| Not in a git repo | Stop |
| No diff against base | Stop, "nothing to review" |
| Subagent returns empty | Print "subagent returned no findings — re-run with a tighter focus" |
| Agent file missing | Print "adversarial-reviewer agent not properly installed. Run `hyprlayer ai reinstall`." |
