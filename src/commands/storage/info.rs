use anyhow::Result;
use colored::Colorize;
use serde_json::{Value, json};

use crate::backends::schema::schema_as_json_value;
use crate::cli::StorageInfoArgs;
use crate::config::{BackendKind, EffectiveConfig, expand_path, get_current_repo_path};

fn expand_display(s: &str) -> String {
    expand_path(s).display().to_string()
}

pub fn info(args: StorageInfoArgs) -> Result<()> {
    let StorageInfoArgs {
        json: as_json,
        config,
    } = args;

    let current_repo = get_current_repo_path()?;
    let current_repo_str = current_repo.display().to_string();

    let effective = config
        .load_if_exists()?
        .as_ref()
        .and_then(|c| c.thoughts.as_ref())
        .map(|t| t.effective_config_for(&current_repo_str))
        .unwrap_or_else(default_effective);

    if as_json {
        let payload = build_json(&effective, &current_repo_str);
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    print_human(&effective, &current_repo_str);
    Ok(())
}

fn default_effective() -> EffectiveConfig {
    EffectiveConfig {
        thoughts_repo: String::new(),
        repos_dir: String::new(),
        global_dir: String::new(),
        user: String::new(),
        backend: BackendKind::default(),
        backend_settings: Default::default(),
        profile_name: None,
        mapped_name: None,
    }
}

fn build_json(eff: &EffectiveConfig, project_path: &str) -> Value {
    json!({
        "backend": eff.backend,
        "settings": backend_settings_json(eff),
        "projectPath": project_path,
        "mappedName": eff.mapped_name,
        "profile": eff.profile_name,
        "user": eff.user,
        "schema": schema_as_json_value(),
    })
}

fn backend_settings_json(eff: &EffectiveConfig) -> Value {
    match eff.backend {
        BackendKind::Git => json!({
            "thoughtsRepo": expand_display(&eff.thoughts_repo),
            "reposDir": eff.repos_dir,
            "globalDir": eff.global_dir,
        }),
        BackendKind::Obsidian => json!({
            "vaultPath": eff.backend_settings.vault_path.as_deref().map(expand_display).unwrap_or_default(),
            "vaultSubpath": eff.backend_settings.vault_subpath.clone().unwrap_or_default(),
            "contentRoot": eff.backend_settings.obsidian_root().map(|p| p.display().to_string()).unwrap_or_default(),
            "reposDir": eff.repos_dir,
            "globalDir": eff.global_dir,
        }),
        BackendKind::Notion => json!({
            "parentPageId": eff.backend_settings.parent_page_id,
            "databaseId": eff.backend_settings.database_id,
            "apiTokenEnv": eff.backend_settings.api_token_env,
        }),
        BackendKind::Anytype => json!({
            "spaceId": eff.backend_settings.space_id,
            "typeId": eff.backend_settings.type_id,
            "apiTokenEnv": eff.backend_settings.api_token_env,
        }),
    }
}

fn print_human(eff: &EffectiveConfig, project_path: &str) {
    println!("{}", "Storage Backend".blue());
    println!("{}", "=".repeat(50).bright_black());
    println!();
    println!("Backend: {}", eff.backend.as_str().cyan());
    println!("Project: {}", project_path.cyan());
    if let Some(profile) = eff.profile_name.as_deref() {
        println!("Profile: {}", profile.cyan());
    }
    if let Some(name) = eff.mapped_name.as_deref() {
        println!("Mapped name: {}", name.cyan());
    } else {
        println!(
            "Mapped name: {}",
            "(unmapped — falling back to defaults)".bright_black()
        );
    }
    println!();
    println!("{}", "Settings:".yellow());
    match eff.backend {
        BackendKind::Git => {
            println!(
                "  Thoughts repo: {}",
                expand_display(&eff.thoughts_repo).cyan()
            );
            println!("  Repos directory: {}", eff.repos_dir.cyan());
            println!("  Global directory: {}", eff.global_dir.cyan());
        }
        BackendKind::Obsidian => {
            let vault_path = eff
                .backend_settings
                .vault_path
                .as_deref()
                .map(expand_display)
                .unwrap_or_else(|| "(not set)".to_string());
            println!("  Vault path: {}", vault_path.cyan());
            println!(
                "  Vault subpath: {}",
                eff.backend_settings
                    .vault_subpath
                    .as_deref()
                    .unwrap_or("")
                    .cyan()
            );
            if let Some(root) = eff.backend_settings.obsidian_root() {
                println!("  Content root: {}", root.display().to_string().cyan());
            }
            println!("  Repos directory: {}", eff.repos_dir.cyan());
            println!("  Global directory: {}", eff.global_dir.cyan());
        }
        BackendKind::Notion => {
            print_opt(
                "  Parent page ID",
                eff.backend_settings.parent_page_id.as_deref(),
            );
            print_opt("  Database ID", eff.backend_settings.database_id.as_deref());
            print_env_ref(eff.backend_settings.api_token_env.as_deref());
        }
        BackendKind::Anytype => {
            print_opt("  Space ID", eff.backend_settings.space_id.as_deref());
            print_opt("  Type ID", eff.backend_settings.type_id.as_deref());
            print_env_ref(eff.backend_settings.api_token_env.as_deref());
        }
    }
}

fn print_opt(label: &str, value: Option<&str>) {
    match value {
        Some(v) => println!("{}: {}", label, v.cyan()),
        None => println!("{}: {}", label, "(not set)".bright_black()),
    }
}

fn print_env_ref(env_var: Option<&str>) {
    match env_var {
        Some(name) => {
            let set = std::env::var(name).is_ok();
            let status = if set {
                "(✓ set)".green().to_string()
            } else {
                "(✗ not set)".red().to_string()
            };
            println!("  API token env: {} {}", name.cyan(), status);
        }
        None => println!("  API token env: {}", "(not set)".bright_black()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BackendSettings;

    fn base_effective() -> EffectiveConfig {
        EffectiveConfig {
            thoughts_repo: "/tmp/thoughts".to_string(),
            repos_dir: "repos".to_string(),
            global_dir: "global".to_string(),
            user: "alice".to_string(),
            backend: BackendKind::Git,
            backend_settings: BackendSettings::default(),
            profile_name: None,
            mapped_name: Some("myproj".to_string()),
        }
    }

    #[test]
    fn json_payload_includes_schema_and_backend_for_git() {
        let eff = base_effective();
        let payload = build_json(&eff, "/code/myproj");
        assert_eq!(payload["backend"], "git");
        assert_eq!(payload["mappedName"], "myproj");
        assert_eq!(payload["projectPath"], "/code/myproj");
        assert!(payload["schema"].is_array());
        assert_eq!(payload["schema"].as_array().unwrap().len(), 10);
        assert_eq!(payload["settings"]["reposDir"], "repos");
        assert_eq!(payload["settings"]["globalDir"], "global");
        assert!(
            payload["settings"]["thoughtsRepo"]
                .as_str()
                .unwrap()
                .contains("thoughts")
        );
    }

    #[test]
    fn json_payload_for_obsidian_includes_content_root() {
        let eff = EffectiveConfig {
            backend: BackendKind::Obsidian,
            backend_settings: BackendSettings {
                vault_path: Some("/vault".to_string()),
                vault_subpath: Some("hyprlayer".to_string()),
                ..Default::default()
            },
            ..base_effective()
        };
        let payload = build_json(&eff, "/code/myproj");
        assert_eq!(payload["backend"], "obsidian");
        assert_eq!(payload["settings"]["vaultPath"], "/vault");
        assert_eq!(payload["settings"]["vaultSubpath"], "hyprlayer");
        assert_eq!(payload["settings"]["contentRoot"], "/vault/hyprlayer");
    }

    #[test]
    fn json_payload_for_unmapped_reports_git_and_null_mapped() {
        let eff = default_effective();
        let payload = build_json(&eff, "/code/nowhere");
        assert_eq!(payload["backend"], "git");
        assert_eq!(payload["mappedName"], serde_json::Value::Null);
    }

    #[test]
    fn json_payload_for_notion_includes_settings() {
        let eff = EffectiveConfig {
            backend: BackendKind::Notion,
            backend_settings: BackendSettings {
                parent_page_id: Some("p1".to_string()),
                database_id: Some("d1".to_string()),
                api_token_env: Some("NOTION_TOKEN".to_string()),
                ..Default::default()
            },
            ..base_effective()
        };
        let payload = build_json(&eff, "/code/myproj");
        assert_eq!(payload["backend"], "notion");
        assert_eq!(payload["settings"]["parentPageId"], "p1");
        assert_eq!(payload["settings"]["databaseId"], "d1");
        assert_eq!(payload["settings"]["apiTokenEnv"], "NOTION_TOKEN");
    }

    #[test]
    fn schema_array_has_all_ten_fields_in_order() {
        let eff = base_effective();
        let payload = build_json(&eff, "/code/x");
        let names: Vec<String> = payload["schema"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            names,
            vec![
                "title", "type", "date", "status", "ticket", "project", "scope", "tags", "author",
                "related",
            ]
        );
    }

    #[test]
    fn select_schema_fields_retain_options_verbatim() {
        let eff = base_effective();
        let payload = build_json(&eff, "/code/x");
        let schema = payload["schema"].as_array().unwrap();
        let type_field = schema.iter().find(|f| f["name"] == "type").unwrap();
        assert_eq!(
            type_field["options"],
            serde_json::json!(["plan", "research", "handoff", "note"])
        );
    }
}
