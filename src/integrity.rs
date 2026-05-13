//! Release-artifact SHA256 verification against GitHub's per-asset `digest`
//! field (`"sha256:<hex>"`, computed server-side, on the public API).

#![allow(dead_code)]

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const READ_BUF_SIZE: usize = 64 * 1024;

#[derive(Debug)]
pub enum IntegrityError {
    Mismatch { expected: String, actual: String },
    MalformedExpected(String),
    Io(io::Error),
}

impl std::fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Mismatch { expected, actual } => {
                write!(f, "SHA256 mismatch: expected {expected}, got {actual}")
            }
            Self::MalformedExpected(s) => write!(
                f,
                "expected SHA256 digest is not a 64-character hex string: {s:?}"
            ),
            Self::Io(e) => write!(f, "IO error while verifying: {e}"),
        }
    }
}

impl std::error::Error for IntegrityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for IntegrityError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Accepts a bare 64-char hex digest or the `sha256:<hex>` form. Returns
/// the bare-hex slice on success.
fn parse_sha256_digest(raw: &str) -> Option<&str> {
    let hex = raw.strip_prefix("sha256:").unwrap_or(raw);
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hex)
    } else {
        None
    }
}

/// Accepts a bare 64-char hex digest or the `sha256:<hex>` form.
pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), IntegrityError> {
    let Some(expected_hex) = parse_sha256_digest(expected) else {
        return Err(IntegrityError::MalformedExpected(expected.to_string()));
    };

    let mut hasher = Sha256::new();
    let mut file = File::open(path)?;
    let mut buf = vec![0u8; READ_BUF_SIZE];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = hex::encode(hasher.finalize());

    if actual.eq_ignore_ascii_case(expected_hex) {
        Ok(())
    } else {
        Err(IntegrityError::Mismatch {
            expected: expected_hex.to_ascii_lowercase(),
            actual,
        })
    }
}

/// Skips assets without a `digest`, with a non-`sha256:` algorithm, or with
/// a malformed digest. Callers fail closed via `HashMap::get` returning None.
pub fn digests_from_release_json(body: &str) -> HashMap<String, String> {
    let Ok(release) = serde_json::from_str::<ReleaseEnvelope>(body) else {
        return HashMap::new();
    };
    let mut out = HashMap::with_capacity(release.assets.len());
    for asset in release.assets {
        let Some(raw) = asset.digest else {
            continue;
        };
        // sha256: prefix is required here (algorithm filter), so skip
        // non-sha256 entries before delegating to the shared parser.
        let Some(rest) = raw.strip_prefix("sha256:") else {
            continue;
        };
        if let Some(hex) = parse_sha256_digest(rest) {
            out.insert(asset.name, hex.to_ascii_lowercase());
        }
    }
    out
}

#[derive(Deserialize)]
struct ReleaseEnvelope {
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    #[serde(default)]
    digest: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    const HELLO_WORLD_SHA: &str =
        "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447";

    fn make_file(contents: &[u8]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(contents).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn verify_sha256_accepts_correct_digest() {
        let f = make_file(b"hello world\n");
        assert!(verify_sha256(f.path(), HELLO_WORLD_SHA).is_ok());
    }

    #[test]
    fn verify_sha256_accepts_uppercase_expected() {
        let f = make_file(b"hello world\n");
        let upper = HELLO_WORLD_SHA.to_ascii_uppercase();
        assert!(verify_sha256(f.path(), &upper).is_ok());
    }

    #[test]
    fn verify_sha256_accepts_sha256_prefixed_form() {
        let f = make_file(b"hello world\n");
        let prefixed = format!("sha256:{HELLO_WORLD_SHA}");
        assert!(verify_sha256(f.path(), &prefixed).is_ok());
    }

    #[test]
    fn verify_sha256_rejects_wrong_digest() {
        let f = make_file(b"hello world\n");
        let wrong = "0".repeat(64);
        match verify_sha256(f.path(), &wrong) {
            Err(IntegrityError::Mismatch { expected, actual }) => {
                assert_eq!(expected, wrong);
                assert_eq!(actual, HELLO_WORLD_SHA);
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
    }

    #[test]
    fn verify_sha256_rejects_malformed_expected() {
        let f = make_file(b"hello world\n");
        assert!(matches!(
            verify_sha256(f.path(), "abc123"),
            Err(IntegrityError::MalformedExpected(_))
        ));
        let bad = "g".repeat(64);
        assert!(matches!(
            verify_sha256(f.path(), &bad),
            Err(IntegrityError::MalformedExpected(_))
        ));
        assert!(matches!(
            verify_sha256(f.path(), "sha256:not-a-hex-string"),
            Err(IntegrityError::MalformedExpected(_))
        ));
    }

    #[test]
    fn verify_sha256_missing_file_is_io_error() {
        let path = Path::new("/nonexistent/path/to/hyprlayer-binary");
        assert!(matches!(
            verify_sha256(path, HELLO_WORLD_SHA),
            Err(IntegrityError::Io(_))
        ));
    }

    #[test]
    fn verify_sha256_handles_empty_file() {
        let f = make_file(b"");
        let empty_sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_sha256(f.path(), empty_sha).is_ok());
    }

    #[test]
    fn verify_sha256_handles_large_file() {
        let bytes = vec![0xAB_u8; 200 * 1024];
        let f = make_file(&bytes);
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let expected = hex::encode(hasher.finalize());
        assert!(verify_sha256(f.path(), &expected).is_ok());
    }

    #[test]
    fn digests_from_release_json_happy_path() {
        let body = format!(
            r#"{{
                "tag_name": "v1.6.0",
                "assets": [
                    {{ "name": "hyprlayer-aarch64-apple-darwin",      "digest": "sha256:{HELLO_WORLD_SHA}" }},
                    {{ "name": "hyprlayer-x86_64-unknown-linux-gnu",  "digest": "sha256:{HELLO_WORLD_SHA}" }},
                    {{ "name": "hyprlayer-x86_64-pc-windows-msvc.exe","digest": "sha256:{HELLO_WORLD_SHA}" }}
                ]
            }}"#
        );
        let m = digests_from_release_json(&body);
        assert_eq!(m.len(), 3);
        assert_eq!(
            m.get("hyprlayer-aarch64-apple-darwin").map(String::as_str),
            Some(HELLO_WORLD_SHA)
        );
    }

    #[test]
    fn digests_from_release_json_lowercases_hex() {
        let upper = HELLO_WORLD_SHA.to_ascii_uppercase();
        let body = format!(r#"{{ "assets": [{{ "name": "x", "digest": "sha256:{upper}" }}] }}"#);
        let m = digests_from_release_json(&body);
        assert_eq!(m.get("x").map(String::as_str), Some(HELLO_WORLD_SHA));
    }

    #[test]
    fn digests_from_release_json_skips_assets_without_digest() {
        let body = format!(
            r#"{{ "assets": [
                {{ "name": "missing-digest" }},
                {{ "name": "have-digest", "digest": "sha256:{HELLO_WORLD_SHA}" }}
            ] }}"#
        );
        let m = digests_from_release_json(&body);
        assert_eq!(m.len(), 1);
        assert!(m.contains_key("have-digest"));
    }

    #[test]
    fn digests_from_release_json_skips_non_sha256_algorithms() {
        let body = format!(
            r#"{{ "assets": [
                {{ "name": "sha512-asset", "digest": "sha512:{HELLO_WORLD_SHA}" }},
                {{ "name": "sha256-asset", "digest": "sha256:{HELLO_WORLD_SHA}" }}
            ] }}"#
        );
        let m = digests_from_release_json(&body);
        assert_eq!(m.len(), 1);
        assert!(m.contains_key("sha256-asset"));
        assert!(!m.contains_key("sha512-asset"));
    }

    #[test]
    fn digests_from_release_json_skips_malformed_digest() {
        let body = r#"{ "assets": [
            { "name": "short",   "digest": "sha256:abc" },
            { "name": "non-hex", "digest": "sha256:gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg" }
        ] }"#;
        let m = digests_from_release_json(body);
        assert!(m.is_empty());
    }

    #[test]
    fn digests_from_release_json_handles_empty_and_malformed_input() {
        assert!(digests_from_release_json("").is_empty());
        assert!(digests_from_release_json("not json").is_empty());
        assert!(digests_from_release_json(r#"{ "tag_name": "v1.6.0" }"#).is_empty());
        assert!(digests_from_release_json(r#"{ "message": "Not Found" }"#).is_empty());
    }
}
