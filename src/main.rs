use anyhow::Result;
use clap::Parser;
use std::time::Instant;

pub mod agents;
mod backends;
mod cli;
mod commands;
mod config;
mod git_ops;
mod hooks;
mod http;
mod integrity;
mod secure_fs;
mod telemetry;
mod version;
mod version_source;

use cli::{
    AiCommands, CodexCommands, ProfileCommands, StorageCommands, TelemetryCommands,
    TelemetryHookAction, ThoughtsCommands,
};
use commands::ai::{configure as ai_configure, reinstall as ai_reinstall, status as ai_status};
use commands::codex::stream as codex_stream;
use commands::self_update as self_update_cmd;
use commands::storage::{
    info as storage_info, set_database_id as storage_set_database_id,
    set_type_id as storage_set_type_id,
};
use commands::telemetry as telemetry_cmd;
use commands::thoughts::profile::{
    create as profile_create, delete as profile_delete, list as profile_list, show as profile_show,
};
use commands::thoughts::{config_cmd, init, status, sync, uninit};

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    let config_path = cli.config_args().and_then(|a| a.path().ok());
    if !cli.skip_startup_checks() {
        version::run_startup_checks(config_path.as_deref(), cli.allows_background_flush());
    }

    let cmd_name = cli.subcommand_name();
    let skip_telemetry = cli.skip_dispatch_telemetry();
    let started = Instant::now();
    let result = dispatch(cli);

    if !skip_telemetry {
        record_dispatch_event(cmd_name, started, &result, config_path.as_deref());
    }

    if let Err(err) = &result
        && err
            .downcast_ref::<telemetry::lifecycle::LockedError>()
            .is_some()
    {
        eprintln!("{err}");
        std::process::exit(2);
    }

    result
}

fn dispatch(cli: cli::Cli) -> Result<()> {
    match cli {
        cli::Cli::Thoughts { command } => match command {
            ThoughtsCommands::Init(args) => init::init(args)?,
            ThoughtsCommands::Uninit(args) => uninit::uninit(args)?,
            ThoughtsCommands::Sync(args) => sync::sync(args)?,
            ThoughtsCommands::Status(args) => status::status(args)?,
            ThoughtsCommands::Config(args) => config_cmd::config(args)?,
            ThoughtsCommands::Profile { command } => match command {
                ProfileCommands::Create(args) => profile_create::create(args)?,
                ProfileCommands::List(args) => profile_list::list(args)?,
                ProfileCommands::Show(args) => profile_show::show(args)?,
                ProfileCommands::Delete(args) => profile_delete::delete(args)?,
            },
        },
        cli::Cli::Ai { command } => match command {
            AiCommands::Configure(args) => ai_configure::configure(args)?,
            AiCommands::Status(args) => ai_status::status(args)?,
            AiCommands::Reinstall(args) => ai_reinstall::reinstall(args)?,
        },
        cli::Cli::Storage { command } => match command {
            StorageCommands::Info(args) => storage_info::info(args)?,
            StorageCommands::SetDatabaseId(args) => storage_set_database_id::set_database_id(args)?,
            StorageCommands::SetTypeId(args) => storage_set_type_id::set_type_id(args)?,
        },
        cli::Cli::Codex { command } => match command {
            CodexCommands::Stream(args) => codex_stream::stream(args)?,
        },
        cli::Cli::Telemetry { command } => match command {
            TelemetryCommands::Init(args) => telemetry_cmd::init::init(args)?,
            TelemetryCommands::Status(args) => telemetry_cmd::status::status(args)?,
            TelemetryCommands::On(args) => telemetry_cmd::on::on(args)?,
            TelemetryCommands::Off(args) => telemetry_cmd::off::off(args)?,
            TelemetryCommands::SkillStart(args) => telemetry_cmd::skill_start::skill_start(args)?,
            TelemetryCommands::SkillEnd(args) => telemetry_cmd::skill_end::skill_end(args)?,
            TelemetryCommands::RecordFromHook(args) => {
                telemetry_cmd::record_from_hook::record_from_hook(args)?
            }
            TelemetryCommands::Hook(args) => match args.action {
                TelemetryHookAction::Install(a) => telemetry_cmd::hook::install_cmd(a)?,
                TelemetryHookAction::Uninstall(a) => telemetry_cmd::hook::uninstall_cmd(a)?,
                TelemetryHookAction::Status(a) => telemetry_cmd::hook::status_cmd(a)?,
            },
            TelemetryCommands::Flush(args) => telemetry_cmd::flush::flush_cmd(args)?,
            TelemetryCommands::Purge(args) => telemetry_cmd::purge::purge(args)?,
            TelemetryCommands::Config(args) => telemetry_cmd::config_cmd::config(args)?,
        },
        cli::Cli::SelfUpdate(args) => self_update_cmd::run(args)?,
    }
    Ok(())
}

/// Append one `cli_command` event without affecting command execution.
fn record_dispatch_event(
    cmd_name: &str,
    started: Instant,
    result: &Result<()>,
    config_path: Option<&std::path::Path>,
) {
    let resolved = match config_path {
        Some(p) => p.to_path_buf(),
        None => match config::get_default_config_path() {
            Ok(p) => p,
            Err(_) => return,
        },
    };
    let Ok(cfg) = config::HyprlayerConfig::load(&resolved) else {
        return;
    };
    if !cfg.telemetry.is_recording() {
        return;
    }

    let outcome = match result {
        Ok(_) => telemetry::event::Outcome::Success,
        Err(_) => telemetry::event::Outcome::Failure,
    };
    let error_class = result
        .as_ref()
        .err()
        .map(|e| telemetry::error_class::classify_error(e.as_ref()));

    let duration_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    let event =
        telemetry::event::Event::cli_command(cmd_name, duration_ms, outcome, error_class, &cfg);
    let _ = telemetry::spool::append(&event);
}
