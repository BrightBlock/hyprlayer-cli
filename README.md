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
# Run the installer (rustup-style)
curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/BrightBlock/hyprlayer-cli/v1.0.0/install.sh | sh
```

This will:
- Download the latest binary for your OS and architecture
- Install it to `~/.hyprlayer/bin/`
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

## Agent Compatibility

This CLI is designed to work with:
- **thoughts-locator**: Finds documents in the `thoughts/` directory structure
- **thoughts-analyzer**: Extracts high-value insights from thoughts documents

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
