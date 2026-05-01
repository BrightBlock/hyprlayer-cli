---
name: code-review
description: Adversarial code review of the current branch. Uses OpenAI Codex CLI when available for a true cross-model second opinion; falls back to a Claude adversarial subagent when codex is absent. Read-only; user-only — never auto-invoke.
allowed-tools: Bash, Read, Agent
disable-model-invocation: true
---

# Code Review (Adversarial)

Drive an adversarial review of the current branch's diff. Two tiers, picked automatically:

- **Tier 1 — codex.** If `codex` CLI is on PATH and authenticated, run codex in challenge mode for a true cross-model second opinion.
- **Tier 2 — Claude subagent.** Spawn the `adversarial-reviewer` agent with a fresh context window. Same model family as the caller, but a clean window plus a canonical adversarial persona produce findings the main thread misses.

Tell the user which tier ran in the output header.

## Arguments

- `/code-review` — auto-detect tier, review the diff against the base branch
- `/code-review <focus>` — narrow the review (e.g., `security`, `concurrency`, `error handling`)
- `--codex` — require codex; fail loudly if missing instead of falling through
- `--claude` — force the subagent path even when codex is available
- `--xhigh` — codex only: use `model_reasoning_effort=xhigh` (slow, ~23x more tokens)
- `-m <model>` — codex only: pass a specific model through (e.g., `-m gpt-5.1-codex-max`)

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

## Step 2: Pick a tier

```bash
HAS_CODEX=no
if command -v codex >/dev/null 2>&1; then
  if [ -n "$CODEX_API_KEY" ] || [ -n "$OPENAI_API_KEY" ] || [ -f "${CODEX_HOME:-$HOME/.codex}/auth.json" ]; then
    HAS_CODEX=yes
  fi
fi
```

Resolve tier:

| Flags | `HAS_CODEX=yes` | `HAS_CODEX=no` |
|---|---|---|
| (none) | Tier 1 (codex) | Tier 2 (subagent) |
| `--codex` | Tier 1 (codex) | Stop with install instructions |
| `--claude` | Tier 2 (subagent) | Tier 2 (subagent) |

If `--codex` was passed and codex is missing or unauthenticated, stop and tell the user:

> codex required but not available. Install: `npm install -g @openai/codex`. Authenticate: `codex login` (or set `$CODEX_API_KEY` / `$OPENAI_API_KEY`).

State the chosen tier in your first response line so the user knows what's running.

## Step 3 (Tier 1): Run codex

### Filesystem boundary

Every prompt sent to codex MUST be prefixed with this exact instruction. Without it, codex wastes tool calls reading skill definitions:

> IMPORTANT: Do NOT read or execute any files under `~/.claude/`, `.claude/skills/`, `claude/skills/`, or `claude/agents/`. These are skill definitions for a different AI system. Stay focused on repository code only.

### Construct the adversarial prompt

Default (no focus argument):

> Review the changes on this branch against `<base>`. Run `git diff origin/<base>...HEAD` to see the diff. Your job is to find ways this code will fail in production. Think like an attacker and a chaos engineer. Look for edge cases, race conditions, security holes, resource leaks, failure modes, and silent data corruption. Be adversarial. Be thorough. No compliments — only the problems.

With focus argument (e.g., `/code-review security`):

> Review the changes on this branch against `<base>`. Run `git diff origin/<base>...HEAD` to see the diff. Focus specifically on `<focus>`. Be adversarial. No compliments — only the problems.

Final prompt = filesystem boundary + blank line + adversarial prompt.

### Run codex with JSONL streaming

Reasoning effort: `high` by default, `xhigh` if `--xhigh` was passed.

```bash
EFFORT=high  # or xhigh
TMPERR=$(mktemp /tmp/codex-err-XXXXXX.txt)
TIMEOUT_BIN=$(command -v gtimeout || command -v timeout || echo "")

CMD=(codex exec "$PROMPT" -C "$_REPO_ROOT" -s read-only -c "model_reasoning_effort=\"$EFFORT\"" --enable web_search_cached --json)
[ -n "$MODEL" ] && CMD+=(-m "$MODEL")

if [ -n "$TIMEOUT_BIN" ]; then
  "$TIMEOUT_BIN" 600 "${CMD[@]}" < /dev/null 2>"$TMPERR" | hyprlayer codex stream
else
  "${CMD[@]}" < /dev/null 2>"$TMPERR" | hyprlayer codex stream
fi
EXIT=${PIPESTATUS[0]}
```

Use a Bash `timeout` of 660000ms on the outer call so the inner `timeout 600` fires first when codex hangs.

> **Note:** `hyprlayer codex stream` reads codex's `--json` output on stdin and prints `[codex thinking]`, `[codex ran]`, agent messages, and token totals. This keeps the skill dependency-free across macOS, Linux, and Windows.

### Hang and auth detection

```bash
if [ "$EXIT" = "124" ]; then
  echo "codex stalled past 10 minutes. Re-run, or check ~/.codex/log/."
fi
if grep -qiE "auth|login|unauthorized" "$TMPERR" 2>/dev/null; then
  echo "[codex auth error] $(head -1 "$TMPERR")"
fi
rm -f "$TMPERR"
```

### Skill-file rabbit hole check

After codex returns, scan its output for `SKILL.md`, `.claude/skills`, `claude/skills`, or `claude/agents`. If any appear:

> warning: codex appears to have read skill definition files instead of repo code. Re-run, or tighten the focus argument.

## Step 3 (Tier 2): Spawn adversarial-reviewer subagent

Build a self-contained prompt and call the Agent tool with `subagent_type: adversarial-reviewer`. The agent runs in a fresh context window with read-only tools.

Prompt template:

```
Review the diff on the current branch against origin/<base>.

Diff range: origin/<base>...HEAD
Repo root: <_REPO_ROOT>
<if focus was provided:> Focus: <focus>

Run `git diff origin/<base>...HEAD` yourself to see the full diff. Read the
surrounding context of any changed file before flagging an issue. Report
ordered by severity per your standard format. If you find nothing
significant, say so in one line.
```

Pass that string as the `prompt` argument. The agent's output is the body of the review — present it verbatim in Step 4.

## Step 4: Present output

First line tells the user which tier ran:

```
tier: codex (cross-model)
```

or

```
tier: claude subagent (single-model, fresh context)
```

Then the review body, verbatim from whichever tier ran. No banners, no separator characters, no decoration.

After the review body, append at most one of:

- `note: claude disagrees with <tier output> on <X> because <Y>.` — only when you genuinely disagree with a specific finding. Don't fabricate disagreement.
- nothing.

If `/review` (Claude's own review) ran earlier in this conversation **and** Tier 1 ran, append a cross-model block:

```
cross-model:
  both found: <overlapping findings>
  only codex: <findings unique to codex>
  only claude: <findings unique to claude /review>
```

Skip the cross-model block when Tier 2 ran (same model family — the comparison isn't meaningful).

## Rules

- **Read-only.** Codex runs with `-s read-only`; the subagent has no `Edit`/`Write`. This skill never modifies files.
- **Verbatim output.** Don't summarize, truncate, or soften findings before presenting. Synthesis comes after, not instead of.
- **No double-reviewing.** Don't re-run Claude's own `/review` from inside this skill.
- **One pass per invocation.** Don't loop. The user can re-run with a tighter focus.

## Error Handling

| Symptom | Action |
|---|---|
| Not in a git repo | Stop |
| No diff against base | Stop, "nothing to review" |
| `--codex` and codex missing | Stop, print install + auth instructions |
| Codex inner `timeout 600` fires (exit 124) | Print stall message |
| Codex stderr contains auth keywords | Print auth error |
| Codex empty stdout, no auth keywords | Print "codex returned no output — check `~/.codex/log/`" |
| Subagent returns empty | Print "subagent returned no findings — re-run with a tighter focus" |
