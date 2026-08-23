# Hyprlayer

AI-assisted spec-driven development.

**[Documentation](https://brightblock.ai/hyprlayer/)**

Hyprlayer provides a structured workflow where AI agents research your codebase, build implementation plans, execute them phase-by-phase, and validate the results -- all grounded in shared team knowledge through a persistent thoughts directory.

## Quick Start

### Install

**macOS / Linux (Homebrew)**
```bash
brew tap brightblock/tap && brew install hyprlayer
```

**Windows (Scoop)**
```powershell
scoop bucket add brightblock https://github.com/BrightBlock/scoop-bucket
scoop install hyprlayer
```

**Windows (WinGet)**
```powershell
winget install BrightBlock.Hyprlayer
```

**Arch Linux (AUR)**
```bash
yay -S hyprlayer-bin
```

### Setup

```bash
# Configure your AI tool
hyprlayer ai configure

# Initialize thoughts in a project
cd ~/Projects/my-project
hyprlayer thoughts init
```

See the [Getting Started guide](https://brightblock.ai/hyprlayer/getting-started/installation/) for full setup instructions.

## Storage Backends

Hyprlayer stores thoughts (plans, research, handoffs, notes) in one of four backends. Pick one at `init` time:

```bash
hyprlayer thoughts init --backend git         # default: a separate git repo, synced via `hyprlayer thoughts sync`
hyprlayer thoughts init --backend obsidian    # local Obsidian vault, no sync (requires --vault-path)
hyprlayer thoughts init --backend notion      # Notion database, via your agent tool's Notion connector
hyprlayer thoughts init --backend anytype     # Anytype object, via the Anytype MCP server
```

For `notion`, the AI agent uses your agent tool's Notion connector (e.g. the Claude.ai connector from `/mcp`) -- hyprlayer never registers a Notion MCP server or manages a Notion token. For `anytype`, hyprlayer registers the MCP server automatically (requires the Anytype desktop app running and an `ANYTYPE_API_KEY`). In both cases the target database (Notion) or object type (Anytype) is **created lazily on the first write-oriented slash command** (e.g. the first `/create_plan` call); re-running after deleting the database/type out-of-band auto-heals.

### Unified metadata schema

Every thought carries the same 10 standardized properties regardless of backend. In `git`/`obsidian` these ride as YAML frontmatter; in `notion` they are first-class database properties; in `anytype` they are type properties.

| Field | Type | Required | Notes |
|---|---|---|---|
| `title` | text | yes | Human-readable title |
| `type` | `plan` \| `research` \| `handoff` \| `note` \| `pr` | yes | Primary category |
| `date` | date (YYYY-MM-DD) | yes | Creation date |
| `status` | `draft` \| `active` \| `implemented` \| `superseded` \| `archived` | yes | Lifecycle state |
| `ticket` | text | no | Optional external reference, e.g. `ENG-1234` |
| `project` | text | yes | Which code repo this relates to |
| `scope` | `user` \| `shared` \| `global` | yes | Matches the thoughts directory split |
| `tags` | multi-select | no | Freeform topic labels |
| `author` | text | yes | Thoughts user |
| `related` | relation | no | Cross-references: page/object IDs or file paths |

Run `hyprlayer storage info --json` from inside a project to see the resolved backend, settings, and the schema contract that slash commands populate.

## Workflow

1. **Research** (`/research_codebase`) -- Explore and document how existing code works
2. **Plan** (`/create_plan`) -- Build a phased implementation plan with success criteria
3. **Implement** (`/implement_plan`) -- Execute the plan phase-by-phase with verification
4. **Validate** (`/validate_plan`) -- Verify the implementation against the plan
5. **Commit** (`/commit`) -- Create atomic commits for changes
6. **Review** (`/code_review`) -- Adversarial diff review (codex CLI when available, Claude subagent otherwise)
7. **Ship** (`/describe_pr`) -- Generate a PR description

## Supported AI Tools

- **Claude Code** -- Anthropic's Claude Code CLI
- **OpenCode** -- OpenCode CLI (GitHub Copilot, Anthropic, or Abacus providers).

## Commands

| Command | Description |
|---|---|
| [`/research_codebase`](https://brightblock.ai/hyprlayer/commands/research-codebase/) | Document how existing code works |
| [`/create_plan`](https://brightblock.ai/hyprlayer/commands/create-plan/) | Create an implementation plan through interactive research |
| [`/iterate_plan`](https://brightblock.ai/hyprlayer/commands/iterate-plan/) | Refine an existing plan based on feedback |
| [`/implement_plan`](https://brightblock.ai/hyprlayer/commands/implement-plan/) | Execute a plan phase-by-phase |
| [`/validate_plan`](https://brightblock.ai/hyprlayer/commands/validate-plan/) | Verify implementation against plan success criteria |
| [`/commit`](https://brightblock.ai/hyprlayer/commands/commit/) | Create a git commit with user approval |
| [`/describe_pr`](https://brightblock.ai/hyprlayer/commands/describe-pr/) | Generate a PR description from branch changes |
| [`/create_handoff`](https://brightblock.ai/hyprlayer/commands/create-handoff/) | Write a handoff document for another session |
| [`/resume_handoff`](https://brightblock.ai/hyprlayer/commands/resume-handoff/) | Pick up work from a handoff document |
| [`/local_review`](https://brightblock.ai/hyprlayer/commands/local-review/) | Set up a worktree to review a branch |
| [`/code_review`](https://brightblock.ai/hyprlayer/commands/code-review/) | Adversarial review of the current branch's diff (codex CLI when available, Claude subagent otherwise) |
| [`/founder_mode`](https://brightblock.ai/hyprlayer/commands/founder-mode/) | Retroactively create a JIRA ticket and PR |
| [`/cost_estimate`](https://brightblock.ai/hyprlayer/commands/cost-estimate/) | Estimate development costs |

The `_nt` and `_generic` command variants have been folded into their parents. A repo
with no thoughts backend, or a request asking for the codebase on its own terms, now
skips the steps that read or write prior context — see the `Prior context` judgment in
`research_codebase`, `create_plan`, and `iterate_plan`.

## Orchestration

Some skills carry their sub-agent wiring as data — an `orchestration:` block — instead of prose. `hyprlayer orchestrate` validates and schedules that block. It is a **validator and a planner, not an execution engine**: it never spawns an agent or runs a step.

```bash
hyprlayer orchestrate check   <skill.md>...             # six mechanical checks, exit 0/1
hyprlayer orchestrate compile <skill.md> --areas N       # the wave schedule, as JSON on stdout
hyprlayer orchestrate grammar                            # the `when:` guard grammar
```

`check` never executes anything — no `exit0` probing, no PATH lookups, no config reads — which is what makes it safe to run from a hook or an editor on every keystroke. `compile` may probe live state (`exit0(...)` commands, `available()` PATH lookups, the effective storage backend) to resolve a step's `when:` guard; pass `--no-probe` to pin every fact explicitly instead and skip execution entirely.

Both leaves validate against a **target harness's agent namespace** — `claude`, `opencode`, or `codex` — because the same skill file is often read by more than one. This matters because **OpenCode reads `~/.claude/skills/` directly**: a skill authored for Claude Code is loaded and executed by OpenCode with no action from the author, against a smaller agent registry. `check` defaults to every harness installed on the machine so an author sees the portability gap before it becomes a spawn-time failure; `compile` takes exactly one target, since a compiled plan is executed by a single harness.

Worked example, from a checkout of this repo, against a real shipped skill's block:

```bash
hyprlayer orchestrate check claude/skills/research_codebase/SKILL.md --target claude --agents-dir claude/agents
hyprlayer orchestrate compile claude/skills/research_codebase/SKILL.md --target claude --agents-dir claude/agents \
  --areas 4 --request "map the PTY stack" > plan.json
```

`check` exits 0 with four `warn:` lines, all of the "examples cannot be
evaluated statically" kind — guards that need a live probe. `compile` reports
15 steps → 9 waves → 7 spawns.

`plan.json` is a diffable artifact: the same block, repo state, request text, and fanout count produce byte-identical output across process restarts, keyed by a `planHash` that changes whenever the target, the schedule, or any resolved fact changes.

## Configuration

### Auto-update

Set `"autoUpdate": true` in `~/.config/hyprlayer/config.json` to make the existing 24-hour startup check perform updates silently instead of just printing a notification. On success, hyprlayer prints `hyprlayer updated to X.Y.Z, please re-run your command.` and exits 0. On failure, it falls back to the notification so the user's command is never blocked.

Auto-update only fires for the Windows `install.ps1` path, which is provided for emergency hotfixes. Every other install method — Homebrew, Winget, Scoop, AUR, `cargo install --git` — keeps the notification UX, so a background startup check never shells out to `brew upgrade` / `winget upgrade` / `pacman` (which may prompt for sudo or take minutes). On those install methods, run `hyprlayer self-update` explicitly when you want the upgrade; it dispatches to the same package manager you installed with.

The default is `false`. Run `hyprlayer self-update` manually any time to update on demand regardless of this flag.

## Telemetry

Hyprlayer records anonymized usage metrics by default so we can prioritize features and catch regressions. No personally identifiable information is collected.

### Opt out

```bash
hyprlayer telemetry off
```

Re-enable later with `hyprlayer telemetry on`.

## Development

```bash
cargo build
cargo test
```

## Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) to get
started. Note that Hyprlayer requires every contributor to sign a
[Contributor License Agreement](CLA.md) before their first change can be merged —
an automated bot walks you through it on your first pull request.

## Acknowledgements

Inspired by [HumanLayer](https://humanlayer.dev).
