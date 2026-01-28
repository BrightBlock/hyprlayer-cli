# HyprLayer

A standalone Rust CLI for managing developer thoughts and notes, compatible with the thoughts-analyzer and thoughts-locator agents.

## Features

- **Initialize** thoughts for current repository with symlink structure
- **Sync** thoughts to centralized git repository
- **Status** display of thoughts repository configuration
- **Uninit** remove thoughts setup from current repository
- **Config** view or edit thoughts configuration
- **Profiles** manage multiple thoughts repositories

## Installation

### Option 1: Install from release (recommended)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/BrightBlock/hyprlayer-cli/v1.0.2/install.sh | sh
```

This will:
- Download the latest binary for your OS and architecture
- Install it to `~/.hyprlayer/bin/`
- Install Claude Code agents and commands to `~/.claude/`
- Add it to your PATH automatically
- Detect your shell (bash, zsh, fish) and provide setup instructions

### Option 2: Install with cargo

```bash
cargo install --git https://github.com/BrightBlock/hyprlayer-cli.git
```

### Option 3: Install from source

```bash
git clone https://github.com/BrightBlock/hyprlayer-cli.git
cd hyprlayer-cli
cargo install --path .
```

## Usage

### Initialize Thoughts

```bash
# Interactive setup
hyprlayer init

# Use specific directory
hyprlayer init --directory my-project

# Use a profile
hyprlayer init --profile work

# Force reconfiguration
hyprlayer init --force
```

### Sync Thoughts

```bash
# Sync with default message
hyprlayer sync

# Sync with custom message
hyprlayer sync --message "Updated documentation"
```

### Show Status

```bash
hyprlayer status
```

### Remove Thoughts

```bash
hyprlayer uninit

# Force removal
hyprlayer uninit --force
```

### Configuration

```bash
# View configuration
hyprlayer config

# Edit configuration
hyprlayer config --edit

# Output as JSON
hyprlayer config --json

# Use custom config file
hyprlayer config --config-file /path/to/config.json
```

### Profile Management

```bash
# Create a new profile
hyprlayer profile-create work

# List all profiles
hyprlayer profile-list

# Show profile details
hyprlayer profile-show work

# Delete a profile
hyprlayer profile-delete work

# Create profile with specific settings
hyprlayer profile-create work --repo ~/thoughts-work --repos-dir repos --global-dir global
```

## Directory Structure

```
~/thoughts/
├── repos/              # Repository-specific thoughts
│   └── [repo-name]/
│       ├── [user]/     # Personal notes
│       └── shared/       # Team notes
└── global/             # Cross-repository thoughts
    ├── [user]/         # Personal cross-repo notes
    └── shared/         # Team cross-repo notes
```

Per-repository structure:
```
[your-project]/
└── thoughts/
    ├── [user]/         → ~/thoughts/repos/[repo]/[user]/
    ├── shared/         → ~/thoughts/repos/[repo]/shared/
    ├── global/         → ~/thoughts/global/
    └── searchable/     → Hard links for search (read-only)
```

## Configuration

Configuration is stored in `~/.config/hyprlayer/config.json`:

```json
{
  "thoughts": {
    "thoughtsRepo": "~/thoughts",
    "reposDir": "repos",
    "globalDir": "global",
    "user": "username",
    "repoMappings": {
      "/path/to/repo": "repo-name"
    },
    "profiles": {
      "work": {
        "thoughtsRepo": "~/thoughts-work",
        "reposDir": "repos",
        "globalDir": "global"
      }
    }
  }
}
```

## Claude Code Integration

The installer sets up a full suite of Claude Code agents and slash commands in `~/.claude/`. These work together to provide a structured AI-assisted development workflow built around the thoughts directory.

### Workflow Overview

The typical development cycle looks like this:

1. **Research** (`/research_codebase`) -- Explore and document how existing code works before making changes
2. **Plan** (`/create_plan`) -- Build a detailed, phased implementation plan with success criteria
3. **Implement** (`/implement_plan`) -- Execute the plan phase-by-phase with verification at each step
4. **Validate** (`/validate_plan`) -- Verify the implementation matches the plan's success criteria
5. **Ship** (`/describe_pr`, `/commit`) -- Generate PR descriptions and commits

For ongoing work, use `/iterate_plan` to refine plans based on feedback, `/debug` to investigate issues, and `/research_codebase` to document unfamiliar areas of code.

### Commands

| Command | Description |
|---|---|
| `/create_plan` | Create a detailed implementation plan through interactive research |
| `/create_plan_nt` | Same as above, without thoughts directory integration |
| `/create_plan_generic` | Same as above, generic variant |
| `/iterate_plan` | Refine an existing plan based on new information or feedback |
| `/iterate_plan_nt` | Same as above, without thoughts directory |
| `/implement_plan` | Execute a plan from `thoughts/shared/plans/`, phase by phase |
| `/validate_plan` | Check implementation against plan success criteria |
| `/research_codebase` | Document how existing code works (no suggestions, just facts) |
| `/research_codebase_nt` | Same as above, without thoughts directory |
| `/research_codebase_generic` | Same as above, generic variant |
| `/describe_pr` | Generate a PR description from branch changes |
| `/describe_pr_nt` | Same as above, without thoughts directory |
| `/ci_describe_pr` | PR description for CI contexts |
| `/commit` | Create a git commit with user approval |
| `/ci_commit` | Create a git commit without user approval (CI contexts) |
| `/debug` | Investigate issues via logs, database state, and git history |
| `/founder_mode` | Retroactively create a JIRA ticket and PR for already-implemented work |
| `/create_handoff` | Write a handoff document to transfer context to another session |
| `/resume_handoff` | Pick up work from a handoff document |
| `/local_review` | Set up a worktree to review a colleague's branch |

The `_nt` variants skip thoughts directory integration for repos that don't use it.

### Agents

These are specialized sub-agents spawned automatically by the commands above. You don't invoke them directly.

| Agent | Purpose |
|---|---|
| **codebase-locator** | Finds where files and components live in the repo |
| **codebase-analyzer** | Traces data flow and explains how code works |
| **codebase-pattern-finder** | Finds existing patterns and examples to model after |
| **thoughts-locator** | Discovers relevant documents in the thoughts directory |
| **thoughts-analyzer** | Extracts key insights from thoughts documents |
| **jira-ticket-reader** | Fetches full JIRA ticket details via MCP server |
| **jira-searcher** | Searches JIRA for related tickets and history via MCP server |
| **web-search-researcher** | Researches external documentation and resources |

### JIRA Integration

The `jira-ticket-reader` and `jira-searcher` agents require a JIRA MCP server to be configured. When available, commands like `/create_plan` and `/research_codebase` will automatically use them to pull ticket details and search for related issues.

## Development

```bash
cargo build
cargo test
cargo run -- init
```

### Build Release Binaries

```bash
cargo build --release
```

The release binaries will be created in `target/release/` with architecture-specific names:
- `hyprlayer-x86_64-unknown-linux-gnu` (Linux x86_64)
- `hyprlayer-aarch64-unknown-linux-gnu` (Linux ARM64)
- `hyprlayer-x86_64-apple-darwin` (macOS x86_64)
