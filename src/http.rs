//! Lightweight `ureq`-based HTTP wrapper for telemetry-adjacent calls.
//!
//! New code should route through here rather than shelling out to `curl`.
//! The existing `curl_get_json` / `curl_download_file` callsites in
//! `agents.rs` and `version.rs` migrate in a separate follow-up.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

#[derive(Debug)]
pub enum HttpError {
    Network(Box<ureq::Error>),
    Status(u16, String),
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Network(e) => write!(f, "network error: {e}"),
            HttpError::Status(code, body) => write!(f, "HTTP {code}: {body}"),
            HttpError::Io(e) => write!(f, "io error: {e}"),
            HttpError::Json(e) => write!(f, "json error: {e}"),
        }
    }
}

impl std::error::Error for HttpError {}

impl From<ureq::Error> for HttpError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                HttpError::Status(code, body)
            }
            other => HttpError::Network(Box::new(other)),
        }
    }
}

impl From<std::io::Error> for HttpError {
    fn from(e: std::io::Error) -> Self {
        HttpError::Io(e)
    }
}

impl From<serde_json::Error> for HttpError {
    fn from(e: serde_json::Error) -> Self {
        HttpError::Json(e)
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    build_agent(timeout, true)
}

fn build_agent(timeout: Duration, follow_redirects: bool) -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new()
        .timeout(timeout)
        .user_agent(concat!("hyprlayer-cli/", env!("CARGO_PKG_VERSION")));
    if !follow_redirects {
        b = b.redirects(0);
    }
    b.build()
}

#[allow(dead_code)]
pub fn get_json<T: DeserializeOwned>(url: &str, timeout: Duration) -> Result<T, HttpError> {
    let resp = agent(timeout).get(url).call()?;
    let body = resp.into_string()?;
    Ok(serde_json::from_str(&body)?)
}

#[allow(dead_code)]
pub fn post_json<T: Serialize, R: DeserializeOwned>(
    url: &str,
    body: &T,
    timeout: Duration,
) -> Result<R, HttpError> {
    let resp = agent(timeout).post(url).send_json(body)?;
    let text = resp.into_string()?;
    Ok(serde_json::from_str(&text)?)
}

/// POST a JSON body and return only the HTTP status code. Redirects are
/// disabled — a 30x on a POST would otherwise replay the API key and
/// event batch to whatever URL the response named. The response body is
/// read to completion and discarded; PostHog Capture replies with a short
/// ack we don't need to parse.
pub fn post_json_no_response<T: Serialize>(
    url: &str,
    body: &T,
    timeout: Duration,
) -> Result<u16, HttpError> {
    let resp = build_agent(timeout, false).post(url).send_json(body)?;
    let status = resp.status();
    let _ = resp.into_string();
    Ok(status)
}

#[allow(dead_code)]
pub fn download_file(url: &str, dest: &Path, timeout: Duration) -> Result<(), HttpError> {
    let resp = agent(timeout).get(url).call()?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest)?;
    std::io::copy(&mut reader, &mut file)?;
    Ok(())
}
