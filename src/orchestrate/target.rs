//! One harness whose agent namespace a block can be validated against.

/// Closed value set, `ValueEnum`-derived so clap generates the "invalid
/// value" message rather than hand-rolling one. `rename_all = "lowercase"`
/// is required: the derive's default is kebab-case, which would turn
/// `OpenCode` into the CLI value `open-code` and break every
/// `--target opencode` in this feature. It also keeps the accepted CLI
/// values identical to `as_str()` — one spelling everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum Target {
    Claude,
    OpenCode,
    Codex,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Claude, Target::OpenCode, Target::Codex];

    pub fn as_str(&self) -> &'static str {
        match self {
            Target::Claude => "claude",
            Target::OpenCode => "opencode",
            Target::Codex => "codex",
        }
    }

    /// True when this harness has an agent directory on this machine —
    /// checked against the registry's own default `search_paths()`, never
    /// against a `--agents-dir` override, so passing that flag can never
    /// make an otherwise-absent target count as installed. Drives the
    /// default "every installed target" set.
    pub fn is_installed(&self) -> bool {
        super::agent_names::registry_for(*self)
            .search_paths()
            .iter()
            .any(|p| p.is_dir())
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
