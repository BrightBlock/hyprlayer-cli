use anyhow::Result;
use clap::Args;
use std::fs;
use std::path::PathBuf;

use crate::config::{BackendKind, HyprlayerConfig, expand_path, get_default_config_path};

/// Common config file argument shared across commands
#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    #[arg(long, help = "Path to config file")]
    pub config_file: Option<String>,
}

impl ConfigArgs {
    /// Resolve the config file path (from arg or default)
    pub fn path(&self) -> Result<PathBuf> {
        self.config_file
            .as_ref()
            .map_or_else(get_default_config_path, |p| Ok(expand_path(p)))
    }

    /// Load existing config, error if not found or incomplete
    pub fn load(&self) -> Result<HyprlayerConfig> {
        let config = self.load_if_exists()?.ok_or_else(|| {
            anyhow::anyhow!("No configuration found. Run 'hyprlayer thoughts init' first.")
        })?;
        if config
            .thoughts
            .as_ref()
            .is_none_or(|t| !t.is_thoughts_configured())
        {
            return Err(anyhow::anyhow!(
                "Thoughts not fully configured. Run 'hyprlayer thoughts init' to complete setup."
            ));
        }
        Ok(config)
    }

    /// Load config if exists, returns None if config file doesn't exist
    pub fn load_if_exists(&self) -> Result<Option<HyprlayerConfig>> {
        let path = self.path()?;
        if !path.exists() {
            return Ok(None);
        }
        HyprlayerConfig::load(&path).map(Some)
    }

    /// Load existing config or fall back to a fresh default. For handlers
    /// that mutate state and don't require `thoughts init` to have run.
    pub fn load_or_default(&self) -> Result<HyprlayerConfig> {
        Ok(self.load_if_exists()?.unwrap_or_default())
    }

    /// Load raw JSON config, error if not found
    pub fn load_raw(&self) -> Result<(PathBuf, serde_json::Value)> {
        let path = self.path()?;
        if !path.exists() {
            return Err(anyhow::anyhow!("No thoughts configuration found"));
        }
        let content = fs::read_to_string(&path)?;
        let value = serde_json::from_str(&content)?;
        Ok((path, value))
    }
}

#[derive(Debug, Args)]
#[command(name = "init", about = "Initialize thoughts for current repository")]
pub struct InitArgs {
    #[arg(long, help = "Force reconfiguration even if already set up")]
    pub force: bool,
    #[arg(
        long,
        help = "Specify the repository directory name (skips interactive prompt)"
    )]
    pub directory: Option<String>,
    #[arg(long, help = "Use a specific thoughts profile")]
    pub profile: Option<String>,
    #[arg(long, value_enum, help = "Storage backend for thoughts")]
    pub backend: Option<BackendKind>,
    #[arg(
        long,
        help = "Obsidian vault path (required when --backend obsidian with --yes)"
    )]
    pub vault_path: Option<String>,
    #[arg(
        long,
        help = "Subfolder within the Obsidian vault for hyprlayer content (default: hyprlayer)"
    )]
    pub vault_subpath: Option<String>,
    #[arg(
        long,
        help = "Notion parent page ID (required when --backend notion with --yes)"
    )]
    pub parent_page_id: Option<String>,
    #[arg(
        long,
        help = "Existing Notion database ID to reuse (skips lazy creation)"
    )]
    pub database_id: Option<String>,
    #[arg(
        long,
        help = "Anytype space ID (required when --backend anytype with --yes)"
    )]
    pub space_id: Option<String>,
    #[arg(long, help = "Existing Anytype type ID to reuse (skips lazy creation)")]
    pub type_id: Option<String>,
    #[arg(
        long,
        help = "Env var name holding the Anytype API token (default: ANYTYPE_API_KEY). \
                Ignored for notion (uses agent tool's connector)."
    )]
    pub api_token_env: Option<String>,
    #[arg(
        long,
        short = 'y',
        help = "Run without interactive prompts (requires existing config and --directory)"
    )]
    pub yes: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "uninit",
    about = "Remove thoughts setup from current repository"
)]
pub struct UninitArgs {
    #[arg(long, help = "Force removal even if not in configuration")]
    pub force: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "sync", about = "Manually sync thoughts to thoughts repository")]
pub struct SyncArgs {
    #[arg(short, long, help = "Commit message for sync")]
    pub message: Option<String>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "status", about = "Show status of thoughts repository")]
pub struct StatusArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "config", about = "View or edit thoughts configuration")]
pub struct ConfigArgsCmd {
    #[arg(long, help = "Open configuration in editor")]
    pub edit: bool,
    #[arg(long, help = "Output configuration as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "create", about = "Create a new thoughts profile")]
pub struct ProfileCreateArgs {
    pub name: String,
    #[arg(long, help = "Thoughts repository path")]
    pub repo: Option<String>,
    #[arg(long, help = "Repos directory name")]
    pub repos_dir: Option<String>,
    #[arg(long, help = "Global directory name")]
    pub global_dir: Option<String>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "list", about = "List all thoughts profiles")]
pub struct ProfileListArgs {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "show", about = "Show details of a specific profile")]
pub struct ProfileShowArgs {
    pub name: String,
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "delete", about = "Delete a thoughts profile")]
pub struct ProfileDeleteArgs {
    pub name: String,
    #[arg(long, help = "Force deletion even if in use")]
    pub force: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

// AI command argument structs

#[derive(Debug, Args)]
#[command(name = "status", about = "Show Claude and Codex agent bundle status")]
pub struct AiStatusArgs {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "reinstall",
    about = "Repair or reinstall Claude and Codex agent files"
)]
pub struct AiReinstallArgs {
    // The pin is persisted and survives binary upgrades, so a bundle that
    // regressed can be held back until it is fixed.
    #[arg(
        long,
        value_name = "VERSION",
        conflicts_with = "unpin",
        help = "Pin the agent bundle to this version and install it"
    )]
    pub version: Option<String>,
    // `conflicts_with` is what makes the pair mutually exclusive: clap
    // rejects both at parse time, rather than the handler having to pick a
    // winner between a pin and its removal.
    #[arg(long, help = "Clear the version pin and install this binary's bundle")]
    pub unpin: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "versions",
    about = "List release versions that carry an agent bundle"
)]
pub struct AiVersionsArgs {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    // Passed straight through as the releases API's `per_page`, which
    // caps at 100.
    #[arg(
        long,
        default_value_t = 10,
        value_parser = clap::value_parser!(u32).range(1..=100),
        help = "How many recent releases to examine"
    )]
    pub limit: u32,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "info",
    about = "Show the active storage backend and its settings"
)]
pub struct StorageInfoArgs {
    #[arg(long, help = "Output as JSON for slash-command consumption")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "set-database-id",
    about = "Persist a Notion database ID to the active profile's backend settings"
)]
pub struct StorageSetDatabaseIdArgs {
    pub id: String,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "set-type-id",
    about = "Persist an Anytype type ID to the active profile's backend settings"
)]
pub struct StorageSetTypeIdArgs {
    pub id: String,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "self-update",
    about = "Update hyprlayer to the latest version installable for this install method"
)]
pub struct SelfUpdateArgs {
    /// Only check whether an update is available; do not perform the update.
    #[arg(long, help = "Report available update without performing it")]
    pub check: bool,
    /// Update even if the local version equals or exceeds the latest release.
    #[arg(long, help = "Skip the \"already on latest\" short-circuit")]
    pub force: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "check", about = "Validate a skill's `orchestration:` block")]
pub struct OrchestrateCheckArgs {
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Emit findings as JSON for editor / app consumption
    #[arg(long)]
    pub json: bool,
    /// Resolve agent names from these directories only, instead of the
    /// target's defaults. Repeatable; requires exactly one `--target`.
    #[arg(long)]
    pub agents_dir: Vec<PathBuf>,
    /// Harness whose agent namespace to validate against. Repeatable.
    /// Default: every target with an agent directory on this machine.
    #[arg(long, value_enum)]
    pub target: Vec<crate::orchestrate::target::Target>,
}

#[derive(Debug, Args)]
#[command(
    name = "compile",
    about = "Compute the wave schedule for a skill's orchestration block"
)]
pub struct OrchestrateCompileArgs {
    pub file: PathBuf,
    /// The user's request text, bound to every `matches(<field>, ...)` leaf
    #[arg(long, conflicts_with = "request_file")]
    pub request: Option<String>,
    #[arg(long)]
    pub request_file: Option<PathBuf>,
    /// Size of a named `over:` list, e.g. --fanout areas=4. Repeatable.
    #[arg(long, value_name = "NAME=N")]
    pub fanout: Vec<String>,
    /// Sugar for --fanout areas=N
    #[arg(long)]
    pub areas: Option<usize>,
    /// Pin any guard leaf by its canonical key, e.g. --fact backend=git.
    /// Repeatable. Always wins over a probe.
    #[arg(long, value_name = "KEY=VALUE")]
    pub fact: Vec<String>,
    /// Resolve nothing by execution or environment inspection
    #[arg(long)]
    pub no_probe: bool,
    /// Accepted for argv symmetry with `check`, so a caller can build one
    /// argument list for both. Currently unused: `compile` schedules and
    /// counts spawns but never resolves agent names — that is `check`'s
    /// job (check 6). Records `agent:` values verbatim, unvalidated.
    #[arg(long)]
    pub agents_dir: Vec<PathBuf>,
    /// Harness this plan will be executed by. Exactly one — a plan is run
    /// by a single harness. Defaults to `claude`. (`check` takes many;
    /// `compile` takes one. See --help.)
    #[arg(long, value_enum)]
    pub target: Vec<crate::orchestrate::target::Target>,
    /// Print a colored wave listing instead of JSON
    #[arg(long)]
    pub human: bool,
}

#[derive(Debug, Args)]
#[command(name = "grammar", about = "Print the `when:` guard grammar")]
pub struct OrchestrateGrammarArgs {
    /// Emit the markdown table for orchestration-runtime.md's generated region
    #[arg(long, conflicts_with = "json")]
    pub markdown: bool,
    /// Emit the machine-readable grammar description
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(
    name = "stream",
    about = "Read codex --json output on stdin, write formatted lines to stdout"
)]
pub struct CodexStreamArgs {
    /// Suppress [codex thinking] reasoning lines
    #[arg(long)]
    pub no_thinking: bool,
    /// Suppress [codex ran] command execution lines
    #[arg(long)]
    pub no_tool_calls: bool,
}

#[derive(Debug, Clone, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum TelemetryModeArg {
    Anonymous,
    Identified,
}

impl From<TelemetryModeArg> for crate::config::TelemetryMode {
    fn from(arg: TelemetryModeArg) -> Self {
        match arg {
            TelemetryModeArg::Anonymous => Self::Anonymous,
            TelemetryModeArg::Identified => Self::Identified,
        }
    }
}

#[derive(Debug, Args)]
#[command(name = "init", about = "Enable telemetry on this installation")]
pub struct TelemetryInitArgs {
    #[arg(long, value_enum, default_value_t = TelemetryModeArg::Anonymous)]
    pub mode: TelemetryModeArg,
    #[arg(long, help = "Override the PostHog API key")]
    pub api_key: Option<String>,
    #[arg(long, help = "Tag events with an organization identifier")]
    pub org_id: Option<String>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "status",
    about = "Show current telemetry mode, endpoint, and spool depth"
)]
pub struct TelemetryStatusArgs {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[arg(
        long,
        help = "Probe the org-managed-key resolution (gh/git) and explain, on stderr, why the current mode was chosen"
    )]
    pub verbose: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "on",
    about = "Re-enable telemetry in anonymous mode (idempotent)"
)]
pub struct TelemetryOnArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "off", about = "Disable telemetry; preserves installation_id")]
pub struct TelemetryOffArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "skill-start",
    about = "Print a session token for a skill run; pair with `skill-end`"
)]
pub struct TelemetrySkillStartArgs {
    #[arg(long, help = "Skill name (informational; the token is skill-agnostic)")]
    pub skill: Option<String>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "skill-end",
    about = "Spool one skill_run event with duration computed from --session"
)]
pub struct TelemetrySkillEndArgs {
    #[arg(long, help = "Skill name")]
    pub skill: String,
    #[arg(long, help = "Session token printed by `telemetry skill-start`")]
    pub session: String,
    #[arg(
        long,
        help = "Outcome: success, failure, or aborted (default: success)"
    )]
    pub outcome: Option<String>,
    #[arg(long, help = "Stable error class (no message text)")]
    pub error_class: Option<String>,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "record-from-hook",
    about = "Read a Claude Code Stop-hook payload from stdin, summarize the active skill turn from the transcript, and spool one skill_run event with token totals."
)]
pub struct TelemetryRecordFromHookArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "hook",
    about = "Manage the Claude Code Stop-hook integration in ~/.claude/settings.json"
)]
pub struct TelemetryHookArgs {
    #[command(subcommand)]
    pub action: TelemetryHookAction,
}

#[derive(Debug, clap::Subcommand)]
pub enum TelemetryHookAction {
    /// Install (or refresh) the hyprlayer Stop hook in ~/.claude/settings.json
    Install(TelemetryHookInstallArgs),
    /// Remove the hyprlayer Stop hook from ~/.claude/settings.json (other hooks preserved)
    Uninstall(TelemetryHookUninstallArgs),
    /// Print whether the hyprlayer Stop hook is installed and the resolved settings.json path
    Status(TelemetryHookStatusArgs),
}

#[derive(Debug, Args)]
pub struct TelemetryHookInstallArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
pub struct TelemetryHookUninstallArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
pub struct TelemetryHookStatusArgs {
    #[arg(long, help = "Output as JSON")]
    pub json: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "flush",
    about = "Drain the spool and POST events to PostHog (foreground)"
)]
pub struct TelemetryFlushArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(name = "purge", about = "Delete the local event spool")]
pub struct TelemetryPurgeArgs {
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[derive(Debug, Args)]
#[command(
    name = "config",
    about = "Set, reset, refresh, or show the telemetry API key override"
)]
pub struct TelemetryConfigArgs {
    #[arg(long, help = "Set a manual PostHog API key (sticky)")]
    pub api_key: Option<String>,
    #[arg(long, help = "Set a manual org_id (sticky)")]
    pub org_id: Option<String>,
    #[arg(long, help = "Clear manual override; falls back to default/github")]
    pub reset: bool,
    #[arg(
        long,
        help = "Force re-pull of HYPRLAYER_TELEMETRY_KEY / HYPRLAYER_ORG_ID from the thoughts repo's GitHub variables"
    )]
    pub refresh: bool,
    #[arg(long, help = "Print the effective resolved config")]
    pub show: bool,
    #[arg(
        long,
        help = "Trace the gh/git org-managed-key resolution on stderr (useful with --refresh when telemetry stays anonymous)"
    )]
    pub verbose: bool,
    #[command(flatten)]
    pub config: ConfigArgs,
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use clap::Parser;

    /// Parse a full `hyprlayer ai ...` invocation. The pin flags are only
    /// worth testing through the real parser: `AiReinstallArgs` derives
    /// `Args`, so it has no `try_parse_from` of its own, and the top-level
    /// `#[command(version)]` is exactly the sort of thing that could shadow
    /// a subcommand's own `--version`.
    fn parse_ai(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut argv = vec!["hyprlayer", "ai"];
        argv.extend_from_slice(args);
        Cli::try_parse_from(argv)
    }

    fn reinstall_args(args: &[&str]) -> super::AiReinstallArgs {
        match parse_ai(args).expect("should parse") {
            Cli::Ai {
                command: crate::cli::AiCommands::Reinstall(args),
            } => args,
            other => panic!("expected an ai reinstall command, got {other:?}"),
        }
    }

    #[test]
    fn reinstall_args_rejects_version_with_unpin() {
        let err = parse_ai(&["reinstall", "--version", "1.5.9", "--unpin"])
            .expect_err("--version and --unpin are mutually exclusive");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::ArgumentConflict,
            "expected a conflict error, got: {err}"
        );
    }

    #[test]
    fn reinstall_args_accept_version_on_its_own() {
        let args = reinstall_args(&["reinstall", "--version", "1.5.9"]);
        assert_eq!(args.version.as_deref(), Some("1.5.9"));
        assert!(!args.unpin);
    }

    #[test]
    fn reinstall_args_accept_unpin_on_its_own() {
        let args = reinstall_args(&["reinstall", "--unpin"]);
        assert!(args.unpin);
        assert!(args.version.is_none());
    }

    #[test]
    fn reinstall_args_default_to_neither_pin_flag() {
        let args = reinstall_args(&["reinstall"]);
        assert!(args.version.is_none());
        assert!(!args.unpin);
    }

    #[test]
    fn versions_args_default_the_limit_and_reject_an_out_of_range_one() {
        let Cli::Ai {
            command: crate::cli::AiCommands::Versions(args),
        } = parse_ai(&["versions"]).expect("should parse")
        else {
            panic!("expected an ai versions command");
        };
        assert_eq!(args.limit, 10);
        assert!(!args.json);

        // `per_page` caps at 100 upstream; asking for more is a parse error
        // rather than a request GitHub silently truncates.
        assert!(parse_ai(&["versions", "--limit", "101"]).is_err());
        assert!(parse_ai(&["versions", "--limit", "0"]).is_err());
    }
}
