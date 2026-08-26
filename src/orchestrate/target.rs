//! One native platform whose agent namespace a block can be validated against.

/// OpenCode is intentionally absent: it is a transport harness whose selected
/// model resolves to one of these two bases before validation or execution.
/// `ValueEnum` lets clap own invalid-value diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum Target {
    Claude,
    Codex,
}

impl Target {
    pub const ALL: [Target; 2] = [Target::Claude, Target::Codex];

    pub fn as_str(&self) -> &'static str {
        match self {
            Target::Claude => "claude",
            Target::Codex => "codex",
        }
    }

    /// True when this platform's native persona root is present. A
    /// `--agents-dir` override never affects the default target set.
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
