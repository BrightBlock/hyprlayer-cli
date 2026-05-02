pub mod configure;
pub mod reinstall;
pub mod status;

use anyhow::Result;
use std::path::Path;

use crate::config::HyprlayerConfig;

/// Persist the SHA after a successful `AgentTool::install` and clear
/// `last_agent_check` so the next startup-time check re-evaluates
/// immediately instead of waiting for the throttle window.
///
/// `sha = None` (commits API was unreachable) leaves the cached SHA
/// untouched but still clears the throttle, so the next startup check
/// will retry the SHA fetch.
pub(crate) fn record_install(
    config: &mut HyprlayerConfig,
    config_path: &Path,
    sha: Option<String>,
) -> Result<()> {
    if sha.is_some() {
        config.agents_installed_sha = sha;
    }
    config.last_agent_check = None;
    config.save(config_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn record_install_persists_sha_and_clears_throttle() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_record_install_test");
        fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("config.json");

        let mut cfg = HyprlayerConfig {
            agents_installed_sha: Some("old".to_string()),
            last_agent_check: Some(1_700_000_000),
            ..Default::default()
        };
        cfg.save(&config_path).unwrap();

        record_install(&mut cfg, &config_path, Some("new".to_string())).unwrap();

        assert_eq!(cfg.agents_installed_sha.as_deref(), Some("new"));
        assert!(cfg.last_agent_check.is_none());

        let reloaded = HyprlayerConfig::load(&config_path).unwrap();
        assert_eq!(reloaded.agents_installed_sha.as_deref(), Some("new"));
        assert!(reloaded.last_agent_check.is_none());

        fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn record_install_with_none_keeps_existing_sha() {
        let temp_dir = std::env::temp_dir().join("hyprlayer_record_install_none_test");
        fs::create_dir_all(&temp_dir).unwrap();
        let config_path = temp_dir.join("config.json");

        let mut cfg = HyprlayerConfig {
            agents_installed_sha: Some("existing".to_string()),
            last_agent_check: Some(1_700_000_000),
            ..Default::default()
        };
        cfg.save(&config_path).unwrap();

        record_install(&mut cfg, &config_path, None).unwrap();

        assert_eq!(cfg.agents_installed_sha.as_deref(), Some("existing"));
        assert!(cfg.last_agent_check.is_none());

        fs::remove_dir_all(&temp_dir).ok();
    }
}
