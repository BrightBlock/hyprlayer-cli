pub mod commands;

use clap::{Parser, Subcommand};
pub use commands::*;

const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_COMMIT"), ")");

#[derive(Parser, Debug)]
#[command(name = "hyprlayer")]
#[command(version = VERSION)]
#[command(about = "Manage developer thoughts and notes", long_about = None)]
pub enum Cli {
    /// Manage developer thoughts and notes
    Thoughts {
        #[command(subcommand)]
        command: ThoughtsCommands,
    },
    /// Manage AI tool configuration
    Ai {
        #[command(subcommand)]
        command: AiCommands,
    },
    /// Inspect the active storage backend for thoughts content
    Storage {
        #[command(subcommand)]
        command: StorageCommands,
    },
    /// Process OpenAI Codex CLI JSONL output
    Codex {
        #[command(subcommand)]
        command: CodexCommands,
    },
    /// Manage usage telemetry
    Telemetry {
        #[command(subcommand)]
        command: TelemetryCommands,
    },
    /// Update hyprlayer through the detected install method
    SelfUpdate(SelfUpdateArgs),
}

impl Cli {
    /// Telemetry subcommands manage the spool/config themselves; we must
    /// not let `run_startup_checks` spawn a background `telemetry flush`
    /// before they run. `telemetry off` would otherwise POST queued events
    /// before disabling, and `telemetry flush` would recursively spawn
    /// another `telemetry flush`.
    pub fn allows_background_flush(&self) -> bool {
        !matches!(self, Cli::Telemetry { .. })
    }

    /// Stable dotted name for the dispatched leaf subcommand, used as the
    /// `command` property on `cli_command` events. Must stay
    /// argument-free (e.g. `ai.configure`, never `ai.configure --force`)
    /// so PostHog aggregations don't fragment per-flag.
    pub fn subcommand_name(&self) -> &'static str {
        match self {
            Cli::Thoughts { command } => match command {
                ThoughtsCommands::Init(_) => "thoughts.init",
                ThoughtsCommands::Uninit(_) => "thoughts.uninit",
                ThoughtsCommands::Sync(_) => "thoughts.sync",
                ThoughtsCommands::Status(_) => "thoughts.status",
                ThoughtsCommands::Config(_) => "thoughts.config",
                ThoughtsCommands::Profile { command } => match command {
                    ProfileCommands::Create(_) => "thoughts.profile.create",
                    ProfileCommands::List(_) => "thoughts.profile.list",
                    ProfileCommands::Show(_) => "thoughts.profile.show",
                    ProfileCommands::Delete(_) => "thoughts.profile.delete",
                },
            },
            Cli::Ai { command } => match command {
                AiCommands::Configure(_) => "ai.configure",
                AiCommands::Status(_) => "ai.status",
                AiCommands::Reinstall(_) => "ai.reinstall",
            },
            Cli::Storage { command } => match command {
                StorageCommands::Info(_) => "storage.info",
                StorageCommands::SetDatabaseId(_) => "storage.set_database_id",
                StorageCommands::SetTypeId(_) => "storage.set_type_id",
            },
            Cli::Codex { command } => match command {
                CodexCommands::Stream(_) => "codex.stream",
            },
            Cli::Telemetry { command } => match command {
                TelemetryCommands::Init(_) => "telemetry.init",
                TelemetryCommands::Status(_) => "telemetry.status",
                TelemetryCommands::On(_) => "telemetry.on",
                TelemetryCommands::Off(_) => "telemetry.off",
                TelemetryCommands::SkillStart(_) => "telemetry.skill_start",
                TelemetryCommands::SkillEnd(_) => "telemetry.skill_end",
                TelemetryCommands::RecordFromHook(_) => "telemetry.record_from_hook",
                TelemetryCommands::Hook(args) => match args.action {
                    TelemetryHookAction::Install(_) => "telemetry.hook.install",
                    TelemetryHookAction::Uninstall(_) => "telemetry.hook.uninstall",
                    TelemetryHookAction::Status(_) => "telemetry.hook.status",
                },
                TelemetryCommands::Flush(_) => "telemetry.flush",
                TelemetryCommands::Purge(_) => "telemetry.purge",
                TelemetryCommands::Config(_) => "telemetry.config",
            },
            Cli::SelfUpdate(_) => "self_update",
        }
    }

    /// Subcommands invoked from skill preambles, the Stop hook, or the
    /// detached background flush. They write to the spool directly, so
    /// dispatch must skip the wrapper `cli_command` event (recursion)
    /// and `run_startup_checks` (would print update / reinstall noise
    /// to stderr on every skill turn).
    fn is_silent_spool_writer(&self) -> bool {
        matches!(
            self,
            Cli::Telemetry {
                command: TelemetryCommands::SkillStart(_)
                    | TelemetryCommands::SkillEnd(_)
                    | TelemetryCommands::RecordFromHook(_)
                    | TelemetryCommands::Flush(_)
            }
        )
    }

    pub fn skip_dispatch_telemetry(&self) -> bool {
        self.is_silent_spool_writer()
    }

    pub fn skip_startup_checks(&self) -> bool {
        self.is_silent_spool_writer() || matches!(self, Cli::SelfUpdate(_))
    }

    /// The `ConfigArgs` of whichever leaf subcommand was selected, or
    /// `None` for subcommands that don't read config (e.g. `codex stream`,
    /// a stdin/stdout filter). Used by startup checks to honor
    /// `--config-file` and per-config `disableUpdateCheck` settings.
    pub fn config_args(&self) -> Option<&ConfigArgs> {
        match self {
            Cli::Thoughts { command } => Some(match command {
                ThoughtsCommands::Init(a) => &a.config,
                ThoughtsCommands::Uninit(a) => &a.config,
                ThoughtsCommands::Sync(a) => &a.config,
                ThoughtsCommands::Status(a) => &a.config,
                ThoughtsCommands::Config(a) => &a.config,
                ThoughtsCommands::Profile { command } => match command {
                    ProfileCommands::Create(a) => &a.config,
                    ProfileCommands::List(a) => &a.config,
                    ProfileCommands::Show(a) => &a.config,
                    ProfileCommands::Delete(a) => &a.config,
                },
            }),
            Cli::Ai { command } => Some(match command {
                AiCommands::Configure(a) => &a.config,
                AiCommands::Status(a) => &a.config,
                AiCommands::Reinstall(a) => &a.config,
            }),
            Cli::Storage { command } => Some(match command {
                StorageCommands::Info(a) => &a.config,
                StorageCommands::SetDatabaseId(a) => &a.config,
                StorageCommands::SetTypeId(a) => &a.config,
            }),
            Cli::Codex { .. } => None,
            Cli::Telemetry { command } => Some(match command {
                TelemetryCommands::Init(a) => &a.config,
                TelemetryCommands::Status(a) => &a.config,
                TelemetryCommands::On(a) => &a.config,
                TelemetryCommands::Off(a) => &a.config,
                TelemetryCommands::SkillStart(a) => &a.config,
                TelemetryCommands::SkillEnd(a) => &a.config,
                TelemetryCommands::RecordFromHook(a) => &a.config,
                TelemetryCommands::Hook(args) => match &args.action {
                    TelemetryHookAction::Install(a) => &a.config,
                    TelemetryHookAction::Uninstall(a) => &a.config,
                    TelemetryHookAction::Status(a) => &a.config,
                },
                TelemetryCommands::Flush(a) => &a.config,
                TelemetryCommands::Purge(a) => &a.config,
                TelemetryCommands::Config(a) => &a.config,
            }),
            Cli::SelfUpdate(args) => Some(&args.config),
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum AiCommands {
    Configure(AiConfigureArgs),
    Status(AiStatusArgs),
    Reinstall(AiReinstallArgs),
}

#[derive(Subcommand, Debug)]
pub enum ThoughtsCommands {
    Init(InitArgs),
    Uninit(UninitArgs),
    Sync(SyncArgs),
    Status(StatusArgs),
    Config(ConfigArgsCmd),
    /// Manage thoughts profiles
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProfileCommands {
    Create(ProfileCreateArgs),
    List(ProfileListArgs),
    Show(ProfileShowArgs),
    Delete(ProfileDeleteArgs),
}

#[derive(Subcommand, Debug)]
pub enum StorageCommands {
    Info(StorageInfoArgs),
    SetDatabaseId(StorageSetDatabaseIdArgs),
    SetTypeId(StorageSetTypeIdArgs),
}

#[derive(Subcommand, Debug)]
pub enum CodexCommands {
    /// Read codex --json output on stdin, write formatted lines to stdout
    Stream(CodexStreamArgs),
}

#[derive(Subcommand, Debug)]
pub enum TelemetryCommands {
    Init(TelemetryInitArgs),
    Status(TelemetryStatusArgs),
    On(TelemetryOnArgs),
    Off(TelemetryOffArgs),
    SkillStart(TelemetrySkillStartArgs),
    SkillEnd(TelemetrySkillEndArgs),
    /// Read a Claude Code Stop-hook payload from stdin and spool one
    /// `skill_run` event with token totals from the transcript.
    RecordFromHook(TelemetryRecordFromHookArgs),
    /// Manage the Claude Code Stop-hook installation in
    /// ~/.claude/settings.json
    Hook(TelemetryHookArgs),
    Flush(TelemetryFlushArgs),
    Purge(TelemetryPurgeArgs),
    Config(TelemetryConfigArgs),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_name(argv: &[&str]) -> &'static str {
        Cli::try_parse_from(argv).unwrap().subcommand_name()
    }

    #[test]
    fn subcommand_names_are_stable_dotted_strings() {
        assert_eq!(parsed_name(&["hyprlayer", "ai", "status"]), "ai.status");
        assert_eq!(
            parsed_name(&["hyprlayer", "ai", "configure"]),
            "ai.configure"
        );
        assert_eq!(
            parsed_name(&["hyprlayer", "thoughts", "status"]),
            "thoughts.status"
        );
        assert_eq!(
            parsed_name(&["hyprlayer", "thoughts", "profile", "list"]),
            "thoughts.profile.list"
        );
        assert_eq!(
            parsed_name(&["hyprlayer", "telemetry", "status"]),
            "telemetry.status"
        );
        assert_eq!(
            parsed_name(&["hyprlayer", "storage", "info"]),
            "storage.info"
        );
        assert_eq!(parsed_name(&["hyprlayer", "self-update"]), "self_update");
    }

    #[test]
    fn skip_dispatch_telemetry_only_for_spool_writers() {
        let yes = [
            "telemetry skill-start",
            "telemetry skill-end --skill x --session 0-deadbeef",
            "telemetry record-from-hook",
            "telemetry flush",
        ];
        for cmd in yes {
            let mut argv = vec!["hyprlayer"];
            argv.extend(cmd.split_whitespace());
            let cli = Cli::try_parse_from(argv).unwrap();
            assert!(cli.skip_dispatch_telemetry(), "expected skip for `{cmd}`");
        }
        let no = [
            "telemetry status",
            "telemetry on",
            "telemetry off",
            "telemetry purge",
            "telemetry init",
            "telemetry hook status",
            "ai status",
            "thoughts status",
        ];
        for cmd in no {
            let mut argv = vec!["hyprlayer"];
            argv.extend(cmd.split_whitespace());
            let cli = Cli::try_parse_from(argv).unwrap();
            assert!(
                !cli.skip_dispatch_telemetry(),
                "expected dispatch event for `{cmd}`"
            );
        }
    }

    #[test]
    fn self_update_skips_startup_checks_but_still_records_dispatch() {
        let cli = Cli::try_parse_from(["hyprlayer", "self-update", "--check"]).unwrap();
        assert!(cli.skip_startup_checks());
        assert!(!cli.skip_dispatch_telemetry());
    }
}
