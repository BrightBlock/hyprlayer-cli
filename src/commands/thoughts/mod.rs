pub mod config_cmd;
pub mod init;
pub mod profile;
pub mod status;
pub mod sync;
pub mod uninit;

/// Render a single `backendSettings` entry for display. `apiTokenEnv` is
/// shown as the env-var name (never the value — hyprlayer never reads the
/// token itself).
pub(crate) fn format_backend_setting(key: &str, val: &serde_json::Value) -> String {
    let raw = val.as_str().unwrap_or("");
    if key == "apiTokenEnv" {
        format!("${raw} (env var name)")
    } else {
        raw.to_string()
    }
}
