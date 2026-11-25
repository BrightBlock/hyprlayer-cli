<p align="center">
  <img src="https://raw.githubusercontent.com/BrightBlock/hyprlayer-app/main/docs/assets/mark.svg" width="56" alt="" />
</p>

<h1 align="center">Hyprlayer CLI</h1>

<p align="center">
  <strong>Spec-driven workflows for your coding agents.</strong><br />
  Research, plan, implement and validate, from a thoughts store your whole team shares.
</p>

<p align="center">
  <a href="https://hyprlayer.dev">Website</a> ·
  <a href="https://hyprlayer.dev/cli/getting-started/installation">Docs</a> ·
  <a href="https://hyprlayer.dev/desktop/install">Desktop app</a> ·
  <a href="https://discord.gg/QFyX2FVkZM">Discord</a> ·
  <a href="https://github.com/BrightBlock/hyprlayer-cli/releases">Changelog</a>
</p>

The CLI installs Hyprlayer's slash commands, agents and skills into Claude Code
and Codex, and keeps your research, plans and handoffs in a thoughts directory
that any agent or teammate can read. It owns your config and your thoughts
storage: git, Obsidian, Notion or Anytype.

Want to run several agents at once? The
[desktop app](https://hyprlayer.dev) is a multi-agent harness orchestrator
that pairs with this CLI. It runs Claude Code, Codex and OpenCode side by side,
each workstream in its own git worktree.

## Install

```bash
brew tap brightblock/tap && brew install hyprlayer   # macOS, Linux
winget install BrightBlock.Hyprlayer                 # Windows
yay -S hyprlayer-bin                                 # Arch
```

Scoop, the PowerShell script and Cargo are in the
[install guide](https://hyprlayer.dev/cli/getting-started/installation).
Then, in a project:

```bash
hyprlayer thoughts init
```

## Questions

Ask in [Discord](https://discord.gg/QFyX2FVkZM), or open an issue.

## Contributing

`cargo build` and `cargo test`. Read [CONTRIBUTING.md](CONTRIBUTING.md) first:
your first pull request asks you to sign the [CLA](CLA.md). The orchestration
validator and agent bundles are documented in [docs/](docs/).

Inspired by [HumanLayer](https://humanlayer.dev). Licensed under
[AGPL-3.0](LICENSE).
