use anyhow::Result;

use crate::cli::AiReinstallArgs;
use crate::commands::ai::{install_claude_hook_if_applicable, record_install};
use crate::config::HyprlayerConfig;

pub fn reinstall(args: AiReinstallArgs) -> Result<()> {
    let AiReinstallArgs {
        force,
        version,
        unpin,
        config,
    } = args;
    let config_path = config.path()?;

    let mut hyprlayer_config = config.load_if_exists()?.unwrap_or_default();

    apply_pin(&mut hyprlayer_config, version.as_deref(), unpin)?;

    let desired_version = hyprlayer_config.desired_assets_version().to_string();
    let sha = if force {
        crate::agents::install_bundle_set(hyprlayer_config.agents_pinned_version.as_deref(), false)?
            .sha
    } else {
        match crate::agents::repair_bundle_set_links(&desired_version)? {
            Some(_) => None,
            None => {
                crate::agents::install_bundle_set(
                    hyprlayer_config.agents_pinned_version.as_deref(),
                    false,
                )?
                .sha
            }
        }
    };
    if !crate::agents::bundle_set_is_installed(&desired_version) {
        anyhow::bail!(
            "Claude + Codex agent setup is incomplete because an existing path was left untouched. \
             Run 'hyprlayer ai status' for details, resolve the reported collision, then retry \
             'hyprlayer ai reinstall'."
        );
    }
    record_install(&mut hyprlayer_config, &config_path, sha)?;
    install_claude_hook_if_applicable(&hyprlayer_config);
    println!("  Agent files installed successfully.");

    Ok(())
}

/// Apply `--version` / `--unpin` to the in-memory config, before the install
/// reads `agents_pinned_version` to decide which bundle to fetch.
///
/// Deliberately only in memory: `record_install` is what saves, and it only
/// runs after a successful local repair or download. A pin this binary
/// refuses (`agents::verify_pin_is_supported`) or cannot obtain therefore
/// leaves no trace on disk, rather than persisting a pin whose bundle was
/// never installed and re-attempting it on every startup refresh.
///
/// clap has already rejected the both-flags case (`conflicts_with`), so the
/// two arms here cannot both fire.
fn apply_pin(config: &mut HyprlayerConfig, version: Option<&str>, unpin: bool) -> Result<()> {
    if unpin {
        config.agents_pinned_version = None;
        return Ok(());
    }
    if let Some(version) = version {
        config.agents_pinned_version = Some(normalize_pin(version)?);
    }
    Ok(())
}

/// Accept the version with or without the tag's leading `v`, and store it
/// the way asset names and `desired_assets_version` spell it — bare. A pin
/// of `v1.6.0` would otherwise resolve to `hyprlayer-assets-claude-v1.6.0
/// .tar.gz`, which no release carries, and fall through to the legacy tree.
fn normalize_pin(version: &str) -> Result<String> {
    let trimmed = version.trim().trim_start_matches('v');
    if trimmed.is_empty() {
        anyhow::bail!("--version needs a release version, for example '--version 1.6.0'");
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_pinned_to(version: Option<&str>) -> HyprlayerConfig {
        HyprlayerConfig {
            agents_pinned_version: version.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn reinstall_version_flag_sets_the_pin() {
        let mut config = config_pinned_to(None);
        apply_pin(&mut config, Some("1.5.9"), false).unwrap();
        assert_eq!(config.agents_pinned_version.as_deref(), Some("1.5.9"));
        assert_eq!(config.desired_assets_version(), "1.5.9");
    }

    #[test]
    fn reinstall_version_flag_accepts_a_tag_style_version() {
        let mut config = config_pinned_to(None);
        apply_pin(&mut config, Some("v1.6.0-rc.1"), false).unwrap();
        assert_eq!(config.agents_pinned_version.as_deref(), Some("1.6.0-rc.1"));
    }

    #[test]
    fn reinstall_version_flag_rejects_an_empty_version() {
        let mut config = config_pinned_to(None);
        assert!(apply_pin(&mut config, Some("  v "), false).is_err());
        assert!(config.agents_pinned_version.is_none());
    }

    #[test]
    fn reinstall_unpin_clears_the_pin() {
        let mut config = config_pinned_to(Some("1.5.9"));
        apply_pin(&mut config, None, true).unwrap();
        assert!(config.agents_pinned_version.is_none());
        assert_eq!(config.desired_assets_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn reinstall_without_pin_flags_leaves_an_existing_pin_alone() {
        let mut config = config_pinned_to(Some("1.5.9"));
        apply_pin(&mut config, None, false).unwrap();
        assert_eq!(config.agents_pinned_version.as_deref(), Some("1.5.9"));
    }
}
