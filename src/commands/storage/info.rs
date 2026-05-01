use anyhow::Result;
use colored::Colorize;
use serde_json::{Value, json};

use crate::backends::schema::schema_as_json_value;
use crate::cli::StorageInfoArgs;
use crate::config::{BackendConfig, EffectiveConfig, expand_path, get_current_repo_path};

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
        user: String::new(),
        backend: BackendConfig::default(),
        profile_name: None,
        mapped_name: None,
    }
}

fn build_json(eff: &EffectiveConfig, project_path: &str) -> Value {
    json!({
        "backend": eff.backend.kind(),
        "settings": backend_settings_json(eff),
        "projectPath": project_path,
        "mappedName": eff.mapped_name,
        "profile": eff.profile_name,
        "user": eff.user,
        "schema": schema_as_json_value(),
    })
}

fn backend_settings_json(eff: &EffectiveConfig) -> Value {
    match &eff.backend {
        BackendConfig::Git(g) => json!({
            "thoughtsRepo": expand_display(&g.thoughts_repo),
            "reposDir": g.repos_dir,
            "globalDir": g.global_dir,
        }),
        BackendConfig::Obsidian(o) => json!({
            "vaultPath": if o.vault_path.is_empty() { String::new() } else { expand_display(&o.vault_path) },
            "vaultSubpath": o.vault_subpath.clone().unwrap_or_default(),
            "contentRoot": o.obsidian_root().map(|p| p.display().to_string()).unwrap_or_default(),
            "reposDir": o.repos_dir,
            "globalDir": o.global_dir,
        }),
        // No `apiTokenEnv`: notion uses the agent's connector (see
        // `backends::notion`), and slash commands branch on the key's absence.
        BackendConfig::Notion(n) => json!({
            "parentPageId": if n.parent_page_id.is_empty() { Value::Null } else { Value::String(n.parent_page_id.clone()) },
            "databaseId": n.database_id,
        }),
        BackendConfig::Anytype(a) => json!({
            "spaceId": if a.space_id.is_empty() { Value::Null } else { Value::String(a.space_id.clone()) },
            "typeId": a.type_id,
            "apiTokenEnv": a.api_token_env,
        }),
    }
}

fn print_human(eff: &EffectiveConfig, project_path: &str) {
    println!("Backend: {}", eff.backend.kind().as_str().cyan());
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
    match &eff.backend {
        BackendConfig::Git(g) => {
            println!(
                "  Thoughts repo: {}",
                expand_display(&g.thoughts_repo).cyan()
            );
            println!("  Repos directory: {}", g.repos_dir.cyan());
            println!("  Global directory: {}", g.global_dir.cyan());
        }
        BackendConfig::Obsidian(o) => {
            let vault_path = if o.vault_path.is_empty() {
                "(not set)".to_string()
            } else {
                expand_display(&o.vault_path)
            };
            println!("  Vault path: {}", vault_path.cyan());
            println!(
                "  Vault subpath: {}",
                o.vault_subpath.as_deref().unwrap_or("").cyan()
            );
            if let Some(root) = o.obsidian_root() {
                println!("  Content root: {}", root.display().to_string().cyan());
            }
            println!("  Repos directory: {}", o.repos_dir.cyan());
            println!("  Global directory: {}", o.global_dir.cyan());
        }
        BackendConfig::Notion(n) => {
            print_opt(
                "  Parent page ID",
                if n.parent_page_id.is_empty() {
                    None
                } else {
                    Some(n.parent_page_id.as_str())
                },
            );
            print_opt("  Database ID", n.database_id.as_deref());
        }
        BackendConfig::Anytype(a) => {
            print_opt(
                "  Space ID",
                if a.space_id.is_empty() {
                    None
                } else {
                    Some(a.space_id.as_str())
                },
            );
            print_opt("  Type ID", a.type_id.as_deref());
            print_env_ref(a.api_token_env.as_deref());
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
    // Only render the env-var row for self-hosted installs. Connector/SSO
    // setups have no token, so rendering "(not set)" would be misleading.
    let Some(name) = env_var else {
        return;
    };
    let set = std::env::var(name).is_ok();
    let status = if set {
        "(set)".green().to_string()
    } else {
        "(not set)".red().to_string()
    };
    println!("  API token env: {} {}", name.cyan(), status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AnytypeConfig, GitConfig, NotionConfig, ObsidianConfig};

    fn base_effective() -> EffectiveConfig {
        EffectiveConfig {
            user: "alice".to_string(),
            backend: BackendConfig::Git(GitConfig {
                thoughts_repo: "/tmp/thoughts".to_string(),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
            }),
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
            backend: BackendConfig::Obsidian(ObsidianConfig {
                vault_path: "/vault".to_string(),
                vault_subpath: Some("hyprlayer".to_string()),
                repos_dir: "repos".to_string(),
                global_dir: "global".to_string(),
            }),
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
    fn json_payload_for_notion_includes_settings_without_token_env() {
        // Notion uses the agent tool's Notion connector (Claude.ai etc.),
        // not a self-hosted MCP server, so hyprlayer never stores a token env
        // name. `apiTokenEnv` must not appear under the notion branch — slash
        // commands rely on the missing key to decide not to surface
        // token-related guidance.
        let eff = EffectiveConfig {
            backend: BackendConfig::Notion(NotionConfig {
                parent_page_id: "p1".to_string(),
                database_id: Some("d1".to_string()),
            }),
            ..base_effective()
        };
        let payload = build_json(&eff, "/code/myproj");
        assert_eq!(payload["backend"], "notion");
        assert_eq!(payload["settings"]["parentPageId"], "p1");
        assert_eq!(payload["settings"]["databaseId"], "d1");
        assert!(
            payload["settings"].get("apiTokenEnv").is_none(),
            "notion settings must not expose apiTokenEnv: {}",
            payload["settings"]
        );
    }

    #[test]
    fn json_payload_for_anytype_includes_settings_and_null_type_id() {
        // Pins the contract every slash command relies on: when the type
        // hasn't been lazily created yet, `typeId` must serialize as JSON
        // null (not omitted, not an empty string) so the dispatch branches
        // `typeId == null` vs populated correctly.
        let eff = EffectiveConfig {
            backend: BackendConfig::Anytype(AnytypeConfig {
                space_id: "s1".to_string(),
                type_id: None,
                api_token_env: Some("ANYTYPE_API_KEY".to_string()),
            }),
            ..base_effective()
        };
        let payload = build_json(&eff, "/code/myproj");
        assert_eq!(payload["backend"], "anytype");
        assert_eq!(payload["settings"]["spaceId"], "s1");
        assert_eq!(payload["settings"]["typeId"], serde_json::Value::Null);
        assert_eq!(payload["settings"]["apiTokenEnv"], "ANYTYPE_API_KEY");
        assert_eq!(payload["schema"].as_array().unwrap().len(), 10);

        let with_type = EffectiveConfig {
            backend: BackendConfig::Anytype(AnytypeConfig {
                space_id: "s1".to_string(),
                type_id: Some("t1".to_string()),
                api_token_env: Some("ANYTYPE_API_KEY".to_string()),
            }),
            ..base_effective()
        };
        let payload = build_json(&with_type, "/code/myproj");
        assert_eq!(payload["settings"]["typeId"], "t1");
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
            serde_json::json!(["plan", "research", "handoff", "note", "pr"])
        );
    }
}
