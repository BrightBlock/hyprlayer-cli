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
# Initialize thoughts in a project
cd ~/Projects/my-project
hyprlayer thoughts init
```

Claude Code and Codex assets are provisioned automatically on the first
hyprlayer run; there is no platform-selection step.

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

## Supported AI platforms

- **Claude Code** -- Anthropic's Claude Code CLI
- **Codex** -- OpenAI's Codex CLI

Hyprlayer always provisions both platform integrations. OpenCode remains a
compatible app harness rather than a third installation target: Claude-family
models use the Claude integration, and every other model uses the Codex
integration.

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

## Agent bundles

The skills, agents, and hooks hyprlayer installs for Claude and Codex ship as
two matching assets attached to each GitHub release:
`hyprlayer-assets-{claude,codex}-<version>.tar.gz`.

From **1.6.0** the bundle is pinned to a release version rather than tracking
`master`. Hyprlayer provisions the matching Claude/Codex pair automatically;
`hyprlayer ai reinstall` repairs it on demand. Both assets are downloaded and
their published SHA256 digests verified before either live integration is
changed. A release carrying only one half is not installable. Once the
installed version and symlinks match the wanted generation, startup performs
no network I/O at all.

On every supported OS, bundle bytes live in
`~/.config/hyprlayer/agents/<version>/{claude,codex}`. Claude and Codex agent
and skill locations are per-entry link farms into that central generation,
matching Omarchy's mixed-directory convention: Hyprlayer creates the native
directory and links each named entry into it without replacing unrelated
skills or agents. Claude skills go to `~/.claude/skills`; Codex-compatible
skills go to both `~/.agents/skills` and `~/.codex/skills`, as Omarchy does;
custom agents go to each harness's `agents` directory. Windows uses directory
junctions where a directory symlink is unavailable, while file entries remain
symlinks. If Windows does not permit file symlinks, setup stops with Developer
Mode/elevation guidance instead of silently creating divergent copies.

### Choosing a version

```bash
hyprlayer ai versions                      # releases carrying both supported bundles
hyprlayer ai versions --json --limit 20
hyprlayer ai reinstall --version 1.6.0     # pin to a version and install it
hyprlayer ai reinstall --unpin             # back to the binary's own bundle
hyprlayer ai reinstall --force             # bypass the local store and download again
```

A pin survives binary upgrades, so a bundle that regresses can be held back
until it is fixed. `hyprlayer ai status` reports `assetsVersion`, `pinnedVersion`
and `binaryVersion` so the skew is visible. A pin is refused if its bundle needs
a newer CLI than the one running.

`ai versions` is the only new API call, it runs on demand rather than at
startup, and its result is cached for an hour — repeated calls do not spend the
unauthenticated rate-limit budget.

### Your edits are kept

Installs never replace a personal file or an external link that occupies a
managed name. They leave the collision in place, report the incomplete pair,
and wait for you to resolve it. On the first store-mode install, exact
digest-matched copies from older Hyprlayer releases are migrated to links;
anything modified is treated as personal and kept. The desktop presents
store-backed agents as read-only System entries, so customizations are made by
creating a separate personal agent rather than editing shared release bytes.

`~/.claude/settings.json` remains mutable configuration rather than an agent
asset: an existing file is preserved, and only a missing file receives the
starter settings.

Skills retired before 1.6.0 are cleaned up too. `ci_commit`, `create_plan_nt`
and the six others dropped at that release are still on disk on every machine
that ran an older hyprlayer — pre-1.6.0 installs kept no record, so nothing
could prove they were ours to delete, and the harness kept finding them next to
the skills that replaced them. The binary now carries a digest list of exactly
what those installs wrote, so an install removes them, and removes only files
that still match it byte for byte. Edit one and it is yours; it stays.

1.6.0 handled that first install differently: it copied each file it replaced to
`<name>.hyprlayer-backup`, which left one inside nearly every skill directory —
extra files sitting where the harness scans for skills. Installs from 1.6.1 on
clear them out, on the daily refresh as well as an explicit reinstall. Only
copies sitting beside a file hyprlayer ships are removed — that is exactly the
set that install could have written, so anything of yours wearing the same
suffix stays. `settings.json.hyprlayer-backup` stays too: it may be the only
copy left of the settings that install replaced, so merge it back and delete it
yourself.

### Frozen legacy trees

The root `claude/` directory is **frozen** and exists only to serve Claude
installs made by CLIs older than 1.6.0, whose download path is compiled in and
cannot be retargeted. It is not a live installation source. Current Claude
and Codex work happens in `assets/`, which is where release bundles are cut
from. See
[assets/FROZEN.md](assets/FROZEN.md).

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
