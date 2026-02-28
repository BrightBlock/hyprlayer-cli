# Hyprlayer AI

AI-assisted spec-driven development. Hyprlayer provides a structured workflow where AI agents research your codebase, build implementation plans, execute them phase-by-phase, and validate the results -- all grounded in shared team knowledge through a persistent thoughts directory.

## How It Works

Hyprlayer installs a CLI (`hyprlayer`) and a suite of AI agent commands for Claude Code and GitHub Copilot. The CLI manages a shared thoughts repository that gives AI agents persistent context about your codebase across sessions and team members.

The development workflow:

1. **Research** (`/research_codebase`) -- Explore and document how existing code works
2. **Plan** (`/create_plan`) -- Build a phased implementation plan with success criteria
3. **Implement** (`/implement_plan`) -- Execute the plan phase-by-phase with verification
4. **Commit** (`/commit`) -- Create atomic commits for changes
5. **Validate** (`/validate_plan`) -- Verify the implementation against the plan
6. **Ship** (`/describe_pr`) -- Generate a PR description

Use `/iterate_plan` to refine plans based on feedback and `/research_codebase` to document unfamiliar areas of code.

## Installation

### Homebrew (macOS and Linux)

```bash
brew tap brightblock/tap
brew install hyprlayer
```

### Windows

Install the [Visual C++ Redistributable](https://aka.ms/vs/17/release/vc_redist.x64.exe) if not already present.

```powershell
iex "& { $(irm https://raw.githubusercontent.com/BrightBlock/hyprlayer-cli/master/install.ps1) }"
```

### Cargo

```bash
cargo install --git https://github.com/BrightBlock/hyprlayer-cli.git
```

## Getting Started

1. Clone your team's thoughts repository:

```bash
gh repo clone <org>/<thoughts-repo> ~/thoughts
```

2. Create a profile pointing to it:

```bash
hyprlayer thoughts profile create <PROFILE_NAME>
```

3. Initialize thoughts in any project:

```bash
cd ~/Projects/my-project
hyprlayer thoughts init --profile <PROFILE_NAME>
```

This creates a `thoughts/` symlink structure in your project that connects to the shared repository. Repeat for each project.

## Commands

| Command | Description |
|---|---|
| `/create_plan` | Create an implementation plan through interactive research |
| `/iterate_plan` | Refine an existing plan based on new information or feedback |
| `/implement_plan` | Execute a plan phase-by-phase |
| `/validate_plan` | Verify implementation against plan success criteria |
| `/research_codebase` | Document how existing code works |
| `/describe_pr` | Generate a PR description from branch changes |
| `/commit` | Create a git commit with user approval |
| `/founder_mode` | Retroactively create a JIRA ticket and PR for already-implemented work |
| `/create_handoff` | Write a handoff document to transfer context to another session |
| `/resume_handoff` | Pick up work from a handoff document |
| `/local_review` | Set up a worktree to review a branch |

Most commands have `_nt` variants that skip thoughts directory integration for repos that don't use it, and `_generic` variants for use outside of this repository.

## Development

```bash
cargo build
cargo test
```

## Acknowledgements

Inspired by [HumanLayer](https://humanlayer.dev).
