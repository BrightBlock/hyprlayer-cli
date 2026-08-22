---
name: code_review
description: Adversarial code review of the current branch. Uses OpenAI Codex CLI when available for a true cross-model second opinion; falls back to a Claude adversarial subagent when codex is absent. Read-only; user-only — never auto-invoke.
allowed-tools: Bash, Read, Agent
disable-model-invocation: true
---

# Code Review (Adversarial)

Drive an adversarial review of the current branch's diff against its base, through a
cross-model reviewer when one is available and a fresh-context Claude reviewer when it
is not.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - subagent-guide             # the marshal's contract, used by `triage`

arguments:
  "/code_review":        auto-detect the tier, review the diff against the base branch
  "/code_review <focus>": narrow the review — e.g. security, concurrency, error handling
  "--codex":  require codex; abort loudly instead of falling through to tier 2
  "--claude": force tier 2 even when codex is available
  "--xhigh":  codex only — model_reasoning_effort=xhigh (slow, roughly 23x more tokens)
  "-m <model>": codex only — pass a specific model through, e.g. -m gpt-5.1-codex-max
  strip-flags-before: focus-argument

flag-facts: >
  No fact source binds `flag(...)`, so no guard in this block reads one: the flags are
  yours to apply, and every step whose behaviour depends on one carries the question as
  a `judgment:` instead. What the guards do carry is the half that self-resolves —
  `available(codex)` and the `exit0(...)` auth probe, which together are HAS_CODEX.

rules:                         # bind you and every agent you spawn — say them in the prompt
  read-only: >
    Codex runs with `-s read-only`; `adversarial-reviewer` and `marshal` have no
    Edit or Write. This skill never modifies files.
  verbatim-output: >
    Never summarize, truncate, or soften a finding before presenting it.
    Synthesis comes after the body, never instead of it.
  no-double-reviewing: >
    Never re-run Claude's own `/review` from inside this skill.
  one-pass-per-invocation: >
    Do not loop. There is no `retry:` in this block on purpose — a re-run with a
    tighter focus is the user's call to make, not yours, including after a
    rabbit-hole warning.
  exactly-one-tier: >
    `codex-review` and `subagent-review` can schedule in the same wave because only
    you know the flags. Run one. Say which, and say you skipped the other.

orchestration:
  owns: [tier-selection, prompt-construction, verbatim-presentation, disagreement, cross-model-comparison]

  steps:
    - id: version-gate
      inline: true
      run:
        posix:   bash "$HOME/.claude/skills/code_review/scripts/check-hyprlayer-version.sh" 1.5.2
        windows: powershell -NoProfile -ExecutionPolicy Bypass -File "$HOME\.claude\skills\code_review\scripts\check-hyprlayer-version.ps1" 1.5.2
      on-nonzero: stop
      minimum-version: 1.5.2
      because: >
        Either variant prints an install or upgrade hint naming the right package
        manager, and exits non-zero when `hyprlayer` is missing or older than the
        required version. Both are cwd-independent, so this runs before repo-root.
        Take the platform from your system context — Windows users typically run
        Claude Code in PowerShell, not git-bash — and stop on a non-zero exit
        rather than continuing without the version.

    - id: repo-root
      requires: [version-gate]
      inline: true
      produces: repo-root
      run: git rev-parse --show-toplevel
      reject: not exit0(git rev-parse --show-toplevel)
      because: >
        cd to the root once here, and every later step and every agent gets an
        absolute frame of reference. Not being in a git repo is a full stop, not
        a fallback to the working directory.

    - id: base-branch
      requires: [repo-root]
      inline: true
      produces: base
      resolve-order:
        - gh pr view --json baseRefName -q .baseRefName
        - gh repo view --json defaultBranchRef -q .defaultBranchRef.name
        - git symbolic-ref refs/remotes/origin/HEAD 2>/dev/null | sed 's|refs/remotes/origin/||'
        - main when origin/main exists, else master
      because: >
        A cascade, not a menu: the first command that answers wins, and its answer
        is the `<base>` every later step refers to. The PR's own base outranks the
        repo default because a stacked branch is not based on the default branch.

    - id: confirm-diff
      requires: [base-branch]
      inline: true
      run:
        - git fetch origin <base> 2>/dev/null
        - git diff origin/<base>...HEAD --stat | tail -1
      reject: exit0(git diff --quiet origin/<base>...HEAD)
      because: >
        An empty diff is a full stop, not an empty review. Say
        "no diff against origin/<base> — nothing to review." and end. The reject
        fires exactly when `--quiet` finds nothing, which is the mechanical form
        of that sentence.

    - id: pick-tier
      requires: [confirm-diff]
      inline: true
      produces: tier
      run: >
        HAS_CODEX=no; if command -v codex >/dev/null 2>&1; then if [ -n "$CODEX_API_KEY" ]
        || [ -n "$OPENAI_API_KEY" ] || [ -f "${CODEX_HOME:-$HOME/.codex}/auth.json" ];
        then HAS_CODEX=yes; fi; fi
      judgment: >
        Which row of the table below does this invocation land on, once you have
        HAS_CODEX from the command above and the flags from the invocation text?
        See "Which tier runs" below.
      table:
        no-flags:   codex-review when HAS_CODEX=yes, else subagent-review
        "--codex":  codex-review when HAS_CODEX=yes, else abort-missing-codex
        "--claude": subagent-review either way
      because: >
        On PATH is not enough — an unauthenticated codex is HAS_CODEX=no and falls
        to tier 2 exactly like a missing one, which is why the guard on tier 1
        carries both halves. HAS_CODEX is the half a guard can carry; the flags are
        not, so the row you land on is the judgment above rather than a `when:` that
        would evaluate unknown and skip every tier step in every run. State the
        chosen tier in your first response line so the user knows what is running.

    - id: abort-missing-codex
      requires: [pick-tier]
      when: not available(codex) or not exit0([ -n "$CODEX_API_KEY" ] || [ -n "$OPENAI_API_KEY" ] || [ -f "${CODEX_HOME:-$HOME/.codex}/auth.json" ])
      when-examples:
        match:
          - "codex not on PATH"
          - "codex on PATH but no auth.json and neither API key set"
        no-match:
          - "codex on PATH and authenticated"
      inline: true
      judgment: >
        Was `--codex` passed on this invocation? If it was, this is the abort: emit
        the text below and end the run. If it was not, this step emits nothing and
        `subagent-review` owns the case. See "Aborting instead of falling through"
        below.
      emit: >
        codex required but not available. Install: `npm install -g @openai/codex`.
        Authenticate: `codex login` (or set `$CODEX_API_KEY` / `$OPENAI_API_KEY`).
      ends-run: >
        When the judgment says `--codex` was passed: emit the text above and stop
        there. No tier, no triage, no present. Every step downstream names this one
        in its `requires:`, so the schedule puts it in a strictly earlier wave than
        the marshal rather than alongside it — you get the chance to stop before
        anything is spawned.
      because: >
        `--codex` means the user wants the cross-model opinion specifically.
        Falling through to tier 2 would hand them a single-model review under the
        name they explicitly refused, so this branch stops the run instead. The
        guard is HAS_CODEX=no, which is exactly the condition under which the abort
        is even possible; whether the user demanded codex is the judgment, because
        `flag(--codex)` never resolves and would drop this step from every run.

    - id: codex-review
      requires: [pick-tier, abort-missing-codex]
      when: available(codex) and exit0([ -n "$CODEX_API_KEY" ] || [ -n "$OPENAI_API_KEY" ] || [ -f "${CODEX_HOME:-$HOME/.codex}/auth.json" ])
      when-examples:
        match:
          - "codex on PATH and authenticated, no flags"
          - "--codex passed, codex on PATH and authenticated"
        no-match:
          - "codex not on PATH"
          - "codex on PATH but unauthenticated"
      inline: true
      produces: review-body
      tier: 1
      reject: not matches(codex-prompt, "Do NOT read or execute")
      prompt:
        boundary: >
          IMPORTANT: Do NOT read or execute any files under `~/.claude/`,
          `.claude/skills/`, `claude/skills/`, or `claude/agents/`. These are skill
          definitions for a different AI system. Stay focused on repository code only.
        default: >
          Review the changes on this branch against `<base>`. Run
          `git diff origin/<base>...HEAD` to see the diff. Your job is to find ways
          this code will fail in production. Think like an attacker and a chaos
          engineer. Look for edge cases, race conditions, security holes, resource
          leaks, failure modes, and silent data corruption. Be adversarial. Be
          thorough. No compliments — only the problems.
        with-focus: >
          Review the changes on this branch against `<base>`. Run
          `git diff origin/<base>...HEAD` to see the diff. Focus specifically on
          `<focus>`. Be adversarial. No compliments — only the problems.
        assemble: boundary, then a blank line, then with-focus if a focus argument survived flag-stripping, else default
      effort:
        default:  high
        "--xhigh": xhigh      # model_reasoning_effort=xhigh — slow, roughly 23x more tokens
      model:
        default:  codex's own default
        "-m":     pass the named model straight through; omit the argument when the flag is absent
      run:
        posix:   _REPO_ROOT="$_REPO_ROOT" bash "$HOME/.claude/skills/code_review/scripts/run-codex.sh" "$PROMPT" "$EFFORT" ${MODEL:-}
        windows: powershell -NoProfile -ExecutionPolicy Bypass -File "$HOME\.claude\skills\code_review\scripts\run-codex.ps1" -Prompt $PROMPT -Effort high-or-xhigh [-Model $MODEL]
      timeout:
        posix:   660000ms on the outer Bash call, so the script's inner `timeout 600` fires first when codex hangs
        windows: about 600000ms on the outer Bash call — PowerShell ships no native `timeout`, so the script lets codex enforce its own request timeout and surfaces a stall as a non-zero exit
      on:
        SKILL.md:       warn-rabbit-hole
        .claude/skills: warn-rabbit-hole
        claude/skills:  warn-rabbit-hole
        claude/agents:  warn-rabbit-hole
      because: >
        The guard is HAS_CODEX alone: false makes tier 1 impossible on every
        row, so skipping is always right. True makes it only the default row —
        `--claude` still forces tier 2, and that half lives in `pick-tier`'s
        judgment because `flag(--claude)` evaluates unknown and would skip
        tier 1 even on the row it owns. Both script variants pipe codex's
        `--json` through `hyprlayer codex stream` and print a one-line hint on
        an auth error. The boundary prefix is not decoration — without it codex
        burns tool calls reading skill definitions, and `on:` checks it did not.

    - id: subagent-review
      requires: [pick-tier, abort-missing-codex]
      agent: adversarial-reviewer
      judgment: >
        Did `pick-tier` land on tier 2 — HAS_CODEX=no with no `--codex`, or
        `--claude` on any row? See "Which tier runs" below.
      produces: review-body
      tier: 2
      given:
        - { value: repo-root,  src: "git rev-parse --show-toplevel, from repo-root" }
        - { value: diff-range, src: "origin/<base>...HEAD, built from base-branch" }
        - { value: focus,      src: "the invocation argument with flags stripped" }
      ask: [findings-ordered-by-severity, one-line-when-nothing-significant]
      because: >
        No `when:` and no `reject:`, both for one reason: `flag(--claude)` never
        resolves, so a guard here would skip tier 2 in exactly the case the user
        named it, and rejecting on `flag(--codex)` would block the row
        `--claude` selects. Either way the run produces no review at all. It
        schedules unconditionally and the judgment decides, including deciding
        not to run when tier 1 did. Same model family as the caller, but a fresh
        context and an adversarial persona surface what the main thread missed.
        The prompt is self-contained: the agent runs
        `git diff origin/<base>...HEAD` itself and reads around each change
        before flagging it. Its output is the review body, verbatim.

    - id: triage
      requires: [pick-tier, abort-missing-codex, codex-review, subagent-review]
      agent: marshal
      judgment: >
        Does this review need a coordinator at all? See "Whether to triage" below.
      given:
        - { value: review-output, src: "the tier's returned text, verbatim and uncut" }
        - { value: diff-range,    src: "origin/<base>...HEAD, built from base-branch" }
        - { value: repo-root,     src: "git rev-parse --show-toplevel, from repo-root" }
      ask: [verified-against-tree, duplicates-merged-across-reviewers, ruling-with-evidence]
      rulings: [fix-now, defer, reject]
      because: >
        Triage never replaces the verbatim body — it is appended after it under a
        `triage:` heading, so the user sees what the reviewer said and then what
        survived checking, in that order. Never let it silently swallow a finding:
        a rejected finding still appears, with its reason. `abort-missing-codex` is
        in the requires because a skipped step satisfies whatever required it: naming
        the abort costs nothing on the runs where it is skipped, and on the run where
        it fires it puts the marshal in a later wave than the abort instead of
        alongside it, so you never spawn a coordinator for a review that never ran.
        `subagent-review` is a real anchor now that it carries no guard.

    - id: present
      requires: [pick-tier, abort-missing-codex, codex-review, subagent-review, triage]
      inline: true
      header:
        tier-1: "tier: codex (cross-model)"
        tier-2: "tier: claude subagent (single-model, fresh context)"
      body: the review-body of whichever tier ran, verbatim
      append: the triage block, if triage ran, under a `triage:` heading
      because: >
        The tier line first so the user knows what produced the body, then the body
        exactly as the tier returned it. No banners, no separator characters, no
        decoration. An empty subagent return is reported as "subagent returned no
        findings — re-run with a tighter focus", never papered over with a
        summary you wrote yourself. There is no tier header without a body: if the
        run reached `abort-missing-codex` and stopped, this step was never reached.

    - id: disagree-note
      requires: [present]
      inline: true
      at-most: 1
      judgment: >
        Do you genuinely disagree with a specific finding, and can you name the
        finding and the reason? See "Disagreeing on the record" below.
      emit: "note: claude disagrees with <tier output> on <X> because <Y>."

    - id: cross-model
      requires: [present, disagree-note]
      when: available(codex) and exit0([ -n "$CODEX_API_KEY" ] || [ -n "$OPENAI_API_KEY" ] || [ -f "${CODEX_HOME:-$HOME/.codex}/auth.json" ])
      when-examples:
        match:
          - "codex on PATH and authenticated — tier 1 was available"
        no-match:
          - "codex not on PATH — tier 2 ran, nothing to compare against"
          - "codex on PATH but unauthenticated — tier 2 ran"
      inline: true
      judgment: >
        Did tier 1 actually run, and did Claude's own `/review` run earlier in this
        conversation? See "Claiming a cross-model comparison" below.
      emit: |
        cross-model:
          both found: <overlapping findings>
          only codex: <findings unique to codex>
          only claude: <findings unique to claude /review>
      because: >
        The guard carries the half a guard can carry, and it is the same guard
        `codex-review` carries, so the two skip together: with codex absent or
        unauthenticated tier 2 ran, the comparison is not meaningful, and this block
        is skipped outright. The `--claude` case is the judgment's first half rather
        than a `not flag(--claude)` clause that would evaluate unknown and drop this
        step from every run. Whether a `/review` ran is a transcript fact no fact
        source binds, so it is the judgment's second half. `disagree-note` is in the
        requires to hold the original's append order — the note comes after the body
        and before the cross-model block — which prose alone would not schedule.

conventions:

  warn-rabbit-hole: >
    warning: codex appears to have read skill definition files instead of repo
    code. Re-run, or tighten the focus argument.

  error-handling:
    not-in-a-git-repo:        stop
    no-diff-against-base:     stop with "nothing to review"
    codex-required-missing:   stop, print the install and auth instructions
    codex-exit-124:           the inner `timeout 600` fired — print the stall message
    codex-stderr-auth-words:  print the auth error
    codex-empty-stdout:       when stderr carries no auth keywords, print "codex returned no output — check `~/.codex/log/`"
    subagent-returns-empty:   print "subagent returned no findings — re-run with a tighter focus"
```

## Judgment

**Which tier runs.** HAS_CODEX is mechanical and the guards carry it; the flags are not,
because nothing binds `flag(...)` and a guard reading one evaluates unknown, which
skips. So `pick-tier`'s table is yours to apply: no flags means tier 1 when
HAS_CODEX=yes and tier 2 otherwise, `--codex` means tier 1 or the abort, `--claude`
means tier 2 on either row. With codex present both tier steps land in the same wave —
that is the compiler saying it cannot see your flags, not an instruction to run two.
Run exactly one, name it in the first response line, and say which you skipped. There
is no row for `--codex --claude`; `--claude` is unconditional tier 2, so take it and
say which flag you honoured. Run both and you bill twice for one diff; run neither and
the invocation produces a header with nothing under it.

**Aborting instead of falling through.** The guard only establishes codex is missing or
unauthenticated — the common case, where tier 2 is right and this step emits nothing.
It becomes an abort only when `--codex` was passed, which lives in the invocation text
and nowhere a `--fact` reaches. Emit the install and auth text and end the run. Fall
through instead and you hand the user a single-model review under the one name they
explicitly refused, with no way to tell from the output.

**Whether to triage.** Three findings need no coordinator; a long review does. Run the
marshal at roughly six or more, when a previous invocation ran the other tier and you
have two reviews to reconcile, or when the user asks what to fix rather than what is
wrong. Not a `when:` because the count exists only after the tier returns, so
`count(findings) >= 6` evaluates unknown and skips exactly the long reviews it targets.
Cheap direction: an agent ranks three findings. Expensive direction: fifteen unverified
findings with no ordering, so they fix the three cheapest and miss the one that pages
them.

**Disagreeing on the record.** At most one `note:` line, and only against a *named*
finding for a *stated* reason. Fabricate one to look engaged and you teach the user to
discount the whole review, including the finding that was right. Suppress a real one
and you have passed along a finding you know is wrong, under a header saying an
adversarial reviewer stands behind it.

**Claiming a cross-model comparison.** The guard establishes codex was available, not
that it ran — with `--claude` there is no codex output to diff. Only you can establish
that tier 1 ran, and that Claude's own `/review` ran earlier, because that lives in the
transcript. If either did not happen, emit nothing. A block assembled from a
half-remembered review is a fabricated comparison, and it is the one part of this
output the user cannot check.
