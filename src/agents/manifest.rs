//! In-bundle `manifest.json` for a per-harness release asset
//! (`hyprlayer-assets-<harness>-<version>.tar.gz`, built by
//! `scripts/build-asset-bundles.sh` from `assets/<harness>/`).
//!
//! The manifest is the bundle's self-description: which release it was cut
//! from, which harness it targets, the oldest CLI that may consume it, and
//! the full owned-file list with per-file SHA256. Those hashes are what let
//! the installer check completeness, leave user-modified files alone, and
//! delete files a previous bundle owned that this one dropped.

// The install path starts reading manifests in a later phase; the builder is
// the only producer today. Drop this once `agents.rs` consumes it.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use crate::integrity::parse_sha256_digest;
use crate::version::is_newer_version;

/// Name of the manifest inside a bundle, relative to the harness root.
pub const MANIFEST_FILE_NAME: &str = "manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    /// Release version this bundle was cut from, without a leading `v`.
    pub version: String,
    /// Harness this bundle targets: "claude" | "copilot" | "opencode".
    pub harness: String,
    /// Oldest CLI version that can consume this bundle. Guards a forward
    /// pin: skills may reference CLI features an older binary lacks.
    pub min_cli_version: String,
    /// Every file the bundle owns, path relative to the harness root,
    /// with its SHA256. Drives completeness, orphan removal, and the
    /// user-modification check.
    pub files: Vec<ManifestEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug)]
pub enum ManifestError {
    Json(serde_json::Error),
    MalformedDigest { path: String, sha256: String },
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "manifest.json is not a valid bundle manifest: {e}"),
            Self::MalformedDigest { path, sha256 } => write!(
                f,
                "manifest entry {path:?} has a SHA256 that is not a 64-character hex string: {sha256:?}"
            ),
        }
    }
}

impl std::error::Error for ManifestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            Self::MalformedDigest { .. } => None,
        }
    }
}

impl BundleManifest {
    /// Parse and validate a bundle manifest. Every digest is checked with
    /// the same parser `integrity::verify_sha256` uses, so a manifest that
    /// parses here can never fail verification as `MalformedExpected`
    /// later — a mis-hashed file is then a genuine mismatch, not a typo.
    pub fn parse(json: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(json).map_err(ManifestError::Json)?;
        for entry in &manifest.files {
            if parse_sha256_digest(&entry.sha256).is_none() {
                return Err(ManifestError::MalformedDigest {
                    path: entry.path.clone(),
                    sha256: entry.sha256.clone(),
                });
            }
        }
        Ok(manifest)
    }

    /// Whether `cli_version` is new enough to install this bundle. Only the
    /// `major.minor.patch` core is compared (`version::is_newer_version`),
    /// so a prerelease of the floor version counts as supported.
    pub fn supports_cli_version(&self, cli_version: &str) -> bool {
        !is_newer_version(&self.min_cli_version, cli_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST_A: &str = "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447";
    const DIGEST_B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn sample() -> BundleManifest {
        BundleManifest {
            version: "1.6.0".to_string(),
            harness: "claude".to_string(),
            min_cli_version: "1.6.0".to_string(),
            files: vec![
                ManifestEntry {
                    path: "agents/cartographer.md".to_string(),
                    sha256: DIGEST_A.to_string(),
                },
                ManifestEntry {
                    path: "skills/create_plan/SKILL.md".to_string(),
                    sha256: DIGEST_B.to_string(),
                },
            ],
        }
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let json = serde_json::to_string_pretty(&sample()).unwrap();
        let back = BundleManifest::parse(&json).unwrap();
        assert_eq!(back.version, "1.6.0");
        assert_eq!(back.harness, "claude");
        assert_eq!(back.min_cli_version, "1.6.0");
        assert_eq!(back.files.len(), 2);
        assert_eq!(back.files[0].path, "agents/cartographer.md");
        assert_eq!(back.files[0].sha256, DIGEST_A);
        assert_eq!(back.files[1].path, "skills/create_plan/SKILL.md");
        assert_eq!(back.files[1].sha256, DIGEST_B);
    }

    #[test]
    fn manifest_field_names_are_snake_case_on_the_wire() {
        // The builder writes this JSON with plain shell, so the key names
        // are duplicated there — a rename here silently breaks the bundle.
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains(r#""min_cli_version":"1.6.0""#), "{json}");
        assert!(json.contains(r#""files":[{"path":"#), "{json}");
    }

    #[test]
    fn manifest_parses_the_builder_output_shape() {
        let json = format!(
            r#"{{
  "version": "1.6.0",
  "harness": "opencode",
  "min_cli_version": "1.6.0",
  "files": [
    {{"path": "agents/cartographer.md", "sha256": "{DIGEST_A}"}}
  ]
}}"#
        );
        let manifest = BundleManifest::parse(&json).unwrap();
        assert_eq!(manifest.harness, "opencode");
        assert_eq!(manifest.files.len(), 1);
    }

    #[test]
    fn parse_rejects_a_malformed_digest() {
        let mut manifest = sample();
        manifest.files[1].sha256 = "not-a-digest".to_string();
        let json = serde_json::to_string(&manifest).unwrap();
        match BundleManifest::parse(&json) {
            Err(ManifestError::MalformedDigest { path, sha256 }) => {
                assert_eq!(path, "skills/create_plan/SKILL.md");
                assert_eq!(sha256, "not-a-digest");
            }
            other => panic!("expected MalformedDigest, got {other:?}"),
        }

        // Right length, wrong alphabet — the case a length check misses.
        let mut manifest = sample();
        manifest.files[0].sha256 = "g".repeat(64);
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(matches!(
            BundleManifest::parse(&json),
            Err(ManifestError::MalformedDigest { .. })
        ));

        // Truncated by one character.
        let mut manifest = sample();
        manifest.files[0].sha256 = DIGEST_A[..63].to_string();
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(matches!(
            BundleManifest::parse(&json),
            Err(ManifestError::MalformedDigest { .. })
        ));
    }

    #[test]
    fn parse_rejects_json_that_is_not_a_manifest() {
        assert!(matches!(
            BundleManifest::parse("not json"),
            Err(ManifestError::Json(_))
        ));
        assert!(matches!(
            BundleManifest::parse(r#"{"version": "1.6.0"}"#),
            Err(ManifestError::Json(_))
        ));
    }

    #[test]
    fn parse_accepts_the_sha256_prefixed_digest_form() {
        // `integrity::verify_sha256` accepts it, so the manifest must not
        // reject what verification would have accepted.
        let mut manifest = sample();
        manifest.files[0].sha256 = format!("sha256:{DIGEST_A}");
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(BundleManifest::parse(&json).is_ok());
    }

    #[test]
    fn min_cli_version_compares_against_a_semver() {
        let mut manifest = sample();
        manifest.min_cli_version = "1.6.0".to_string();

        assert!(manifest.supports_cli_version("1.6.0"));
        assert!(manifest.supports_cli_version("1.6.1"));
        assert!(manifest.supports_cli_version("1.7.0"));
        assert!(manifest.supports_cli_version("2.0.0"));
        assert!(manifest.supports_cli_version("1.10.0"), "1.10 > 1.6");

        assert!(!manifest.supports_cli_version("1.5.9"));
        assert!(!manifest.supports_cli_version("1.5.10"));
        assert!(!manifest.supports_cli_version("0.9.0"));

        // Prerelease and build metadata are stripped before comparison, so a
        // 1.6.0 release candidate still satisfies a 1.6.0 floor.
        assert!(manifest.supports_cli_version("1.6.0-rc1"));
        assert!(manifest.supports_cli_version("1.6.0+build.3"));
    }
}
