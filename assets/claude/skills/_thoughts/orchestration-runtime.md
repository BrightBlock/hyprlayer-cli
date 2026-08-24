# Orchestration runtime

Skills that wire their sub-agents as data carry an `orchestration:` block.
`hyprlayer orchestrate` validates and schedules one. It never runs one:
execution is yours.

## Preflight

Run `hyprlayer orchestrate check <skill-file> --target claude` first and do not
execute a block it rejects; each finding names the file, line, and fix. It reads
files and runs nothing, so it is safe on every call. If the command is
unavailable, that is not a failing block: say so and go on.

Keep `--target claude`. Bare `check` validates against every harness installed
here, which is a skill author's question. Yours is narrower: can the harness you
are running in execute this block? A block can be sound for you and unsound for
one you are not in.

`hyprlayer orchestrate compile <skill-file> --areas N --human` shows the wave
schedule, what each guard skipped and why, and the choices it left to you. It
probes `exit0(...)`, PATH, and the backend unless you pass `--no-probe`.

Neither proves the block is right. A guard that passes its own examples can
still ask the wrong question.

## Block layout

`orchestration:` is this runtime's vocabulary. Everything else is skill data,
interpreted by that skill's own prose, and keys you do not recognize are its
business rather than yours. Three recur: `loads:` names protocol files to read
before anything, resolved against `~/.claude/skills/_thoughts/`; `constraints:`
names files binding you and every agent you spawn; `on-empty-invocation:` is
text to emit verbatim when the skill is invoked with no argument.

## Execution

`steps:` is a set; `requires` alone orders it. Spawn every ready agent step in
one message so they run concurrently, re-evaluate as each returns, and stop when
nothing is ready. A step never starts before its `requires` return: that is the
barrier. A skipped step consumes no wave and satisfies whatever required it.
Say when you skip one.

`id:` names a step; `requires:` and `retry:` refer to it. `agent: <type>` spawns
one sub-agent of that type. `fanout: <type>` with `over: <list>` spawns one per
element of a list you derive at runtime, all in the same message. `inline: true`
you do yourself, never delegated. `judgment:` you decide, stating the call and
your reason. `given:` entries each carry a `src:` from this session; anything you
have not verified is an `ask:`. `reject:` is a precondition on the step's inputs,
so fix the input rather than spawning and hoping. `retry: {step, max}` re-runs a
step at most N times across the whole run, counted. `on: {token: action}` matches
a literal token in returned text; a branch needing interpretation is a
`judgment:`. `because:` is rationale, not executable. `owns:` never goes to a
sub-agent.

## The `when:` grammar

Generated from the parser by `hyprlayer orchestrate grammar`. Only
`available()`, `exit0()`, and `backend` self-resolve; bind anything else with
`--fact` or it evaluates unknown and the step skips.

<!-- generated: hyprlayer orchestrate grammar --markdown -->
| Form | True when | Example |
|---|---|---|
| `<path> == <value>` / `!=` | Literal comparison against known state | `backend == git` |
| `exists(<thing>)` | The named thing is present | `exists(plan.phases)` |
| `matches(<field>, <pattern>)` | Regex match against an input | `matches(request, "ENG-\d+")` |
| `flag(<name>)` | The invocation carried that flag | `flag(--claude)` |
| `available(<binary>)` | Resolvable on PATH | `available(codex)` |
| `count(<thing>) <op> <n>` | Numeric comparison | `count(findings) > 0` |
| `exit0(<command>)` | The command exits 0 | `exit0(git diff --quiet)` |
| `<expr> and/or/not <expr>` | Boolean composition | `flag(--codex) and not available(codex)` |

Precedence: `not` binds tightest, then `and`, then `or`; `and`/`or` are left-associative, so `a and b or c` parses as `(a and b) or c`. Parentheses are permitted and override this.
<!-- /generated -->

If you cannot say what you would check, it is a `judgment:`, not a `when:`.

## Invariants

- A sub-agent's report is input, not truth. Spawn a follow-up if it looks wrong.
- Hand over what the agent cannot discover: repo root, diff range, their phase.
- Constraints the skill sets bind every agent you spawn. Say them in the prompt.
- You own persistence: artifacts, `status` promotion, commits, PR edits.
- If prose contradicts the block's structure, the block wins; if it adds
  judgment the block cannot express, follow both.
