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

```bash
cd cli
cargo install --path .
```

## Usage

### Initialize Thoughts

```bash
# Interactive setup
hyprlayer thoughts init

# Use specific directory
hyprlayer thoughts init --directory my-project

# Use a profile
hyprlayer thoughts init --profile work
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
```

### Configuration

```bash
# View configuration
hyprlayer thoughts config

# Edit configuration
hyprlayer thoughts config --edit

# Output as JSON
hyprlayer thoughts config --json
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

## Agent Compatibility

This CLI is designed to work with:
- **thoughts-locator**: Finds documents in the `thoughts/` directory structure
- **thoughts-analyzer**: Extracts high-value insights from thoughts documents

## Development

```bash
cargo build
cargo test
cargo run -- thoughts init
```
