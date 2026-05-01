---
description: Adversarial review of the current branch's diff via a fresh-context subagent. With Abacus or GitHub Copilot provider, runs cross-model on GPT-5-Codex.
model: {{SONNET_MODEL}}
subtask: false
---

# Code Review (Adversarial)

Drive an adversarial review of the current branch's diff. Spawns the `adversarial-reviewer` subagent with a fresh context window. The subagent's model is configured at install time per your OpenCode provider:

- **Abacus** → `gpt-5-codex` (true cross-model second opinion)
- **GitHub Copilot** → `gpt-5-codex` (true cross-model second opinion)
- **Anthropic** → `claude-opus-4-5` (same family, fresh context — Anthropic's API is Claude-only)

## Arguments

- `/code_review` — review the diff against the auto-detected base branch
- `/code_review <focus>` — narrow the review (e.g., `security`, `concurrency`, `error handling`)
- `--model <id>` — request a specific model for this run instead of the configured default (e.g., `--model abacus/o3-pro`). Note: OpenCode does not natively support per-spawn model overrides, so this flag is honored on a best-effort basis — the requested model is mentioned in the subagent prompt and shown in the output header, but the actual model that runs is whatever is set in `~/.config/opencode/agents/adversarial-reviewer.md`. To truly switch models for a single run, use OpenCode's session-level override (`opencode -m <id>`) instead.

Strip flags out of the prompt text before constructing the focus argument.

## Step 0: Minimum hyprlayer version

```bash
HYPR_VER=$(hyprlayer --version 2>/dev/null | awk '{print $2}' | cut -d'(' -f1 | tr -d ' ')
if [ -z "$HYPR_VER" ]; then
  echo "hyprlayer not found. Install: brew install brightblock/tap/hyprlayer"
  exit 1
fi
if ! awk -v v="$HYPR_VER" 'BEGIN { split(v,a,"."); if (a[1]<1 || (a[1]==1 && a[2]<5) || (a[1]==1 && a[2]==5 && a[3]<2)) exit 1 }'; then
  echo "hyprlayer >= 1.5.2 required (have $HYPR_VER). Upgrade: brew upgrade hyprlayer"
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

## Step 2: Spawn the adversarial-reviewer subagent

Build the prompt and dispatch the subagent with `@adversarial-reviewer`:

```
@adversarial-reviewer Review the diff on the current branch against origin/<base>.

Diff range: origin/<base>...HEAD
Repo root: <_REPO_ROOT>
<if focus was provided:> Focus: <focus>
<if --model was provided:> User requested model: <model> (best-effort; actual model is configured in agent frontmatter)

Run `git diff origin/<base>...HEAD` yourself to see the full diff. Read the
surrounding context of any changed file before flagging an issue. Report
ordered by severity per your standard format. If you find nothing
significant, say so in one line.
```

The subagent's output is the body of the review — present it verbatim in Step 3.

## Step 3: Present output

First line tells the user what model the subagent ran on:

```
tier: opencode subagent (<model>, fresh context)
```

Where `<model>` is the model ID from `~/.config/opencode/agents/adversarial-reviewer.md` (the install-time default), or the `--model <id>` value if it was passed. Then the review body verbatim. No banners, no separator characters, no decoration.

If you genuinely disagree with a specific finding, append at most one:

> note: orchestrator disagrees with subagent on <X> because <Y>.

Don't fabricate disagreement.

## Rules

- **Read-only.** Subagent has no `write` / `edit` / `patch` tools. This command never modifies files.
- **Verbatim output.** Don't summarize, truncate, or soften findings before presenting. Synthesis comes after, not instead of.
- **One pass per invocation.** Don't loop. Re-run with a tighter focus to drill deeper.

## Error Handling

| Symptom | Action |
|---|---|
| Not in a git repo | Stop |
| No diff against base | Stop, "nothing to review" |
| Subagent returns empty | Print "subagent returned no findings — re-run with a tighter focus" |
| Subagent file missing `model:` (placeholder not substituted) | Print "adversarial-reviewer agent not properly installed. Run `hyprlayer ai reinstall`." |
