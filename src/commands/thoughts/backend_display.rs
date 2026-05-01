use colored::{ColoredString, Colorize};

use crate::config::BackendConfig;

/// Render the per-backend field rows used by `thoughts config`,
/// `thoughts profile show`, and `thoughts profile list`. When `colorize` is
/// false, values print without ANSI styling (matches `profile list`'s
/// existing output).
pub fn print_backend_block(backend: &BackendConfig, indent: &str, colorize: bool) {
    let style = |s: &str| -> ColoredString { if colorize { s.cyan() } else { s.normal() } };

    match backend {
        BackendConfig::Git(g) => {
            println!("{indent}Thoughts repository: {}", style(&g.thoughts_repo));
            println!("{indent}Repos directory: {}", style(&g.repos_dir));
            println!("{indent}Global directory: {}", style(&g.global_dir));
        }
        BackendConfig::Obsidian(o) => {
            println!("{indent}Vault path: {}", style(&o.vault_path));
            if let Some(sub) = &o.vault_subpath {
                println!("{indent}Vault subpath: {}", style(sub));
            }
            println!("{indent}Repos directory: {}", style(&o.repos_dir));
            println!("{indent}Global directory: {}", style(&o.global_dir));
        }
        BackendConfig::Notion(n) => {
            println!("{indent}Parent page ID: {}", style(&n.parent_page_id));
            if let Some(db) = &n.database_id {
                println!("{indent}Database ID: {}", style(db));
            }
        }
        BackendConfig::Anytype(a) => {
            println!("{indent}Space ID: {}", style(&a.space_id));
            if let Some(t) = &a.type_id {
                println!("{indent}Type ID: {}", style(t));
            }
            if let Some(env) = &a.api_token_env {
                let env_text = format!("${} (env var name)", env);
                if colorize {
                    println!("{indent}API token env: {}", env_text.cyan());
                } else {
                    println!("{indent}API token env: {}", env_text);
                }
            }
        }
    }
}
