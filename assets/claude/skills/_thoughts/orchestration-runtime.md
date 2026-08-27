# Orchestration runtime

Skills that wire their sub-agents as data carry an `orchestration:` block.
`hyprlayer orchestrate` validates and schedules one. It never runs one:
execution is yours.

## Preflight

Run `hyprlayer orchestrate check <skill-file> --target <base>` first and do not
execute a block it rejects; each finding names the file, line, and fix. Select
the native execution base, not a wrapper harness: native Claude and a Claude
model in OpenCode use `--target claude`; native Codex and every other or unknown
OpenCode model use `--target codex`. It reads files and runs nothing, so it is
safe on every call. If the command is unavailable, that is not a failing block:
say so and go on.

Keep `--target` set to that model-selected base. Bare `check` validates against
both installed base platforms, which is a skill author's question. Yours is
narrower: can the model family executing this block resolve its agents? A block
can be sound for one base and unsound for the other.

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

## Fan-out returns

The selected model family determines the return transport, not whether another
harness wraps it:

- Native Claude and a Claude model running through OpenCode use Claude
  behavior: return sub-agent reports in-band and use those reports as the
  fan-out step's result.
- Native Codex and **every non-Claude model** running through OpenCode use Codex
  behavior. An unknown OpenCode model is non-Claude; do not guess that it can
  carry all branch reports in-band.

Codex behavior does not reliably carry every branch through the parent's
message. For every fan-out step using Codex behavior, use this file-mediated
return protocol:

1. Before spawning, create a **fresh** scratch directory as a direct child of
   `<cwd>`. Choose a safe opaque run token and use
   `<cwd>/.hyprlayer-fanout-<run-token>/` as the base. Create it with
   no-clobber semantics (`mkdir`, never `mkdir -p`): if that candidate already
   exists, try a unique `-01`, `-02`, ... suffix until one create succeeds.
   Never enter, reuse, or delete an existing candidate. Record the exact path
   created successfully; it is the only scratch directory this run owns.
2. In the parent, enumerate the fan-out elements and assign safe opaque IDs
   such as zero-padded counters (`0001`, `0002`, ...). Keep the ID-to-element
   mapping in parent state. Never use a raw fan-out value, label, filesystem
   path, or other user-controlled text as a filename or path component. Record
   the exact expected `<created-dir>/<opaque-id>.md` paths before spawning.
3. Start each sub-agent prompt, before the persona or task, with this contract:
   its **only deliverable is `<dir>/<id>.md`**. It must write that file first,
   verify it exists with `ls`, reply with only the path, and never put findings
   in the reply.
4. After the join, run `ls <dir>` and compare it with the expected paths.
   Re-spawn each missing item once with the same contract. This is a Codex
   transport retry, independent of any step-level `retry:` in the block.
5. Never relay the files' contents through the parent message. The synthesis
   step reads every result file directly.
6. After synthesis has read the files, re-check the recorded directory before
   cleanup. It must still be the exact path returned by this run's successful
   create, be a real directory rather than a symlink, have a canonical parent
   equal to canonical `<cwd>`, have a basename beginning
   `.hyprlayer-fanout-`, and contain only this run's expected result files.
   Treat the successful no-clobber create plus these checks as the ownership
   proof; a matching name by itself proves nothing.
7. Only after every containment and ownership check passes, delete that one
   exact recorded directory. Never reconstruct its path from a step ID, use a
   glob/prefix delete, or remove any other `.hyprlayer-fanout-*` entry. If a
   check fails, leave the directory in place and report its path and the failed
   check. The scratch data is transient session state and must never be
   committed, but uncertain ownership always wins over cleanup.

`track-with: task-list` means the harness-native tracker: Claude's native
task-list tool, Codex's native plan tool, or OpenCode's todo list. Keep that
tracker current for the lifetime of the step.

## Agent resolution

On native Codex, resolve `agent:` and `fanout:` names as custom agents first.
Hyprlayer installs their generated definitions at `~/.codex/agents/`; spawn the
named custom agent directly.

If custom agents are unavailable because the feature is gated off or the bundle
is not installed, treat `<name>` as a persona. Read
`./.claude/agents/<name>.md` first, fall back to
`~/.claude/agents/<name>.md`, and carry the file body in the sub-agent prompt.
For a fan-out using Codex behavior, put that body after the write-first
deliverable contract.

OpenCode receives Hyprlayer's shared skills through `~/.claude/skills`, but it
does not discover either Claude's or Codex's custom-agent registry. Therefore
an OpenCode session **always** reads a persona file and carries its instructions
into a generic sub-agent prompt, regardless of its selected model:

- With a Claude model, read `./.claude/agents/<name>.md`, falling back to
  `~/.claude/agents/<name>.md`, and carry the Markdown body.
- With every other model, read `./.codex/agents/<name>.toml`, falling back to
  `~/.codex/agents/<name>.toml`, and carry `developer_instructions`.

The same model-family choice controls return transport: a Claude model uses
Claude behavior, while every other model uses Codex behavior and the
file-mediated fan-out return protocol. For a Codex-behavior fan-out, put the
persona instructions after the write-first deliverable contract.

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
