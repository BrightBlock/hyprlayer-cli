//! Hyprlayer telemetry — opt-in usage analytics over PostHog Capture.
//!
//! Pipeline: `event::Event` → `spool::append` → `flush::flush` → PostHog
//! Capture. The `mode != Off` gate is checked at the top of every spool and
//! flush operation — no event ever leaves a machine that hasn't opted in.

use std::path::PathBuf;

pub mod disclosure;
pub mod error_class;
pub mod event;
pub mod flush;
pub mod identify;
pub mod lifecycle;
pub mod org_config;
pub mod posthog;
pub mod privacy;
pub mod spool;
pub mod verbose;

pub(crate) const DEFAULT_API_KEY: &str = "phc_rwYxPEHvLJ5tPw4wesujdip5tD2WRNdLPLHtAqg7XcqS";
pub(crate) const SPOOL_MAX_BYTES: u64 = 10 * 1024 * 1024;

const TELEMETRY_DIR: &str = "telemetry";
const SPOOL_FILE: &str = "spool.jsonl";

pub fn telemetry_dir() -> std::io::Result<PathBuf> {
    let base = dirs::config_dir()
        .ok_or_else(|| std::io::Error::other("could not determine config directory"))?;
    Ok(base.join("hyprlayer").join(TELEMETRY_DIR))
}

pub fn spool_path() -> std::io::Result<PathBuf> {
    Ok(telemetry_dir()?.join(SPOOL_FILE))
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn rfc3339_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 32 hex chars from `getrandom`. Used as the device salt for repo hashing.
pub fn generate_device_salt() -> String {
    let mut buf = [0u8; 16];
    if getrandom::getrandom(&mut buf).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = ((nanos.rotate_left(i as u32 * 7)) & 0xff) as u8;
        }
    }
    let mut s = String::with_capacity(32);
    for b in &buf {
        use std::fmt::Write;
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}
