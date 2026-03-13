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

## Workflow

1. **Research** (`/research_codebase`) -- Explore and document how existing code works
2. **Plan** (`/create_plan`) -- Build a phased implementation plan with success criteria
3. **Implement** (`/implement_plan`) -- Execute the plan phase-by-phase with verification
4. **Validate** (`/validate_plan`) -- Verify the implementation against the plan
5. **Commit** (`/commit`) -- Create atomic commits for changes
6. **Ship** (`/describe_pr`) -- Generate a PR description

## Supported AI Tools

- **Claude Code** -- Anthropic's Claude Code CLI
- **GitHub Copilot** -- GitHub Copilot in VS Code
- **OpenCode** -- OpenCode CLI (GitHub Copilot, Anthropic, or Abacus providers)

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
| [`/founder_mode`](https://brightblock.ai/hyprlayer/commands/founder-mode/) | Retroactively create a JIRA ticket and PR |
| [`/cost_estimate`](https://brightblock.ai/hyprlayer/commands/cost-estimate/) | Estimate development costs |

Most commands have [`_nt` and `_generic` variants](https://brightblock.ai/hyprlayer/reference/variants/).

## Development

```bash
cargo build
cargo test
```

## Acknowledgements

Inspired by [HumanLayer](https://humanlayer.dev).
