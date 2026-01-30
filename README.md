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

Requires the [GitHub CLI](https://cli.github.com/) (`gh`) to be installed and authenticated.

```bash
export GITHUB_TOKEN=$(gh auth token) && \
  curl --proto '=https' --tlsv1.2 -sSf \
  -H "Authorization: token $GITHUB_TOKEN" \
  https://raw.githubusercontent.com/BrightBlock/hyprlayer-cli/master/install.sh | sh
```

This will:
- Download the latest binary for your OS and architecture
- Install it to `~/.hyprlayer/bin/`
- Install Claude Code agents and commands to `~/.claude/`
- Add it to your PATH automatically
- Detect your shell (bash, zsh, fish) and provide setup instructions

### Windows Installation

Requires the [GitHub CLI](https://cli.github.com/) (`gh`) to be installed and authenticated.

**Prerequisites:** Install the [Visual C++ Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe) if not already installed.

```powershell
$env:GITHUB_TOKEN = (gh auth token); `
  Invoke-Expression ((Invoke-WebRequest `
  -Uri "https://raw.githubusercontent.com/BrightBlock/hyprlayer-cli/master/install.ps1" `
  -Headers @{Authorization = "token $env:GITHUB_TOKEN"}).Content)
```

If you get an execution policy error, run PowerShell as Administrator and execute:

```powershell
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser
```

Or run the script directly with bypass:

```powershell
powershell -ExecutionPolicy Bypass -Command { ... }
```

After installation, add to your PATH:

```powershell
[Environment]::SetEnvironmentVariable('PATH', $env:PATH + ';C:\Users\<username>\.hyprlayer\bin', 'User')
```

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

## Getting Started

After installing the CLI, set up a thoughts profile to connect to your team's shared thoughts repository:

1. **Clone the thoughts repository** into your home directory:

```bash
gh repo clone <org>/<thoughts-repo> ~/hyprlayer-thoughts
```

2. **Create a profile** pointing to the cloned directory:

```bash
hyprlayer thoughts profile create <PROFILE_NAME>
```

When prompted, specify the `~/hyprlayer-thoughts` directory as the thoughts repository path.

3. **Initialize thoughts in a project.** Navigate to a project you want to use thoughts with, then run:

```bash
cd ~/Projects/my-project
hyprlayer thoughts init --profile <PROFILE_NAME>
```

It will prompt you to create a new folder for your project inside the thoughts repository. Once complete, a `thoughts/` symlink structure is set up in your project directory.

You can repeat step 3 for each project you work on.

## Usage

### Initialize Thoughts

```bash
# Interactive setup
hyprlayer thoughts init

# Use specific directory
hyprlayer thoughts init --directory my-project

# Use a profile
hyprlayer thoughts init --profile work

# Force reconfiguration
hyprlayer thoughts init --force
```

### Sync Thoughts

```bash
# Sync with default message
hyprlayer thoughts sync

# Sync with custom message
hyprlayer thoughts sync --message "Updated documentation"
```

### Show Status

```bash
hyprlayer thoughts status
```

### Remove Thoughts

```bash
hyprlayer thoughts uninit

# Force removal
hyprlayer thoughts uninit --force
```

### Configuration

```bash
# View configuration
hyprlayer thoughts config

# Edit configuration
hyprlayer thoughts config --edit

# Output as JSON
hyprlayer thoughts config --json

# Use custom config file
hyprlayer thoughts config --config-file /path/to/config.json
```

### Profile Management

```bash
# Create a new profile
hyprlayer thoughts profile create work

# List all profiles
hyprlayer thoughts profile list

# Show profile details
hyprlayer thoughts profile show work

# Delete a profile
hyprlayer thoughts profile delete work

# Create profile with specific settings
hyprlayer thoughts profile create work --repo ~/thoughts-work --repos-dir repos --global-dir global
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
- `hyprlayer-aarch64-apple-darwin` (macOS Apple Silicon)
- `hyprlayer-x86_64-pc-windows-msvc.exe` (Windows x86_64)
