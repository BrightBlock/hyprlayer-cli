# Orchestration

Some skills carry their sub-agent wiring as data — an `orchestration:` block — instead of prose. `hyprlayer orchestrate` validates and schedules that block. It is a **validator and a planner, not an execution engine**: it never spawns an agent or runs a step.

```bash
hyprlayer orchestrate check   <skill.md>...             # six mechanical checks, exit 0/1
hyprlayer orchestrate compile <skill.md> --areas N       # the wave schedule, as JSON on stdout
hyprlayer orchestrate grammar                            # the `when:` guard grammar
```

`check` never executes anything — no `exit0` probing, no PATH lookups, no config reads — which is what makes it safe to run from a hook or an editor on every keystroke. `compile` may probe live state (`exit0(...)` commands, `available()` PATH lookups, the effective storage backend) to resolve a step's `when:` guard; pass `--no-probe` to pin every fact explicitly instead and skip execution entirely.

Both leaves validate against a **target platform's agent namespace** —
`claude` or `codex` — because the same skill file is read by both. OpenCode
maps its selected model to one of those two namespaces rather than carrying a
separate agent registry. `check` defaults to both supported platforms;
`compile` takes exactly one target, since a compiled plan is executed by a
single model family.

Worked example, from a checkout of this repo, against a real shipped skill's block:

```bash
hyprlayer orchestrate check assets/claude/skills/research_codebase/SKILL.md --target claude --agents-dir assets/claude/agents
hyprlayer orchestrate compile assets/claude/skills/research_codebase/SKILL.md --target claude --agents-dir assets/claude/agents \
  --areas 4 --request "map the PTY stack" > plan.json
```

`check` exits 0 with four `warn:` lines, all of the "examples cannot be
evaluated statically" kind — guards that need a live probe. `compile` reports
15 steps → 9 waves → 7 spawns.

Those are the numbers for a checkout whose commits are all pushed. `compile`
resolves guards against the repo as it stands, so the schedule moves with it:
with unpushed commits, `permalinks` (`exit0(git merge-base --is-ancestor HEAD
@{u})`) is skipped and you get 9 waves → 8. Pin the probes to compare runs
across different repo states:

```bash
hyprlayer orchestrate compile assets/claude/skills/research_codebase/SKILL.md --target claude --agents-dir assets/claude/agents \
  --areas 4 --request "map the PTY stack" \
  --fact 'backend=git' --fact 'exit0(git merge-base --is-ancestor HEAD @{u})=true' --fact 'exit0(test -d thoughts)=true'
```

`plan.json` is a diffable artifact: the same block, repo state, request text, and fanout count produce byte-identical output across process restarts, keyed by a `planHash` that changes whenever the target, the schedule, or any resolved fact changes.

