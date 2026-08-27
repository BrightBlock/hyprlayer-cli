//! Lightweight `ureq`-based HTTP wrapper for telemetry- and agent-install
//! HTTP calls.
//!
//! All GitHub-facing network calls route through here rather than shelling
//! out to `curl` — see `agents.rs` for the release API and asset callers.

use std::io::Read as _;
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
    let body = get_text(url, timeout)?;
    Ok(serde_json::from_str(&body)?)
}

pub fn get_text(url: &str, timeout: Duration) -> Result<String, HttpError> {
    let resp = agent(timeout).get(url).call()?;
    Ok(resp.into_string()?)
}

/// Like `get_text` but refuses to buffer more than `max_bytes` into memory.
/// Use this for any endpoint whose response size isn't intrinsically bounded
/// by our own infrastructure (e.g. the GitHub releases API), so a hostile or
/// misconfigured source can't stream gigabytes into a `String`.
pub fn get_text_capped(url: &str, timeout: Duration, max_bytes: u64) -> Result<String, HttpError> {
    get_text_capped_with_headers(url, timeout, max_bytes, &[])
}

/// Like `get_text_capped`, but with extra request headers (e.g. GitHub's
/// `Accept: application/vnd.github.v3+json`).
pub fn get_text_capped_with_headers(
    url: &str,
    timeout: Duration,
    max_bytes: u64,
    headers: &[(&str, &str)],
) -> Result<String, HttpError> {
    let mut req = agent(timeout).get(url);
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let resp = req.call()?;
    let mut reader = resp.into_reader().take(max_bytes + 1);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    if buf.len() as u64 > max_bytes {
        return Err(HttpError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("response exceeded {max_bytes}-byte cap"),
        )));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
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
    download_file_capped(url, dest, timeout, None)
}

/// Streaming download with an optional byte cap. Once `max_bytes` is reached
/// the connection is aborted and the partial file deleted so we never end up
/// hashing/swapping an over-sized artifact.
pub fn download_file_capped(
    url: &str,
    dest: &Path,
    timeout: Duration,
    max_bytes: Option<u64>,
) -> Result<(), HttpError> {
    let resp = agent(timeout).get(url).call()?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(dest)?;
    let copy_result = match max_bytes {
        Some(cap) => {
            let mut reader = resp.into_reader().take(cap + 1);
            let copied = std::io::copy(&mut reader, &mut file)?;
            if copied > cap {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("download exceeded {cap}-byte cap"),
                ))
            } else {
                Ok(())
            }
        }
        None => {
            let mut reader = resp.into_reader();
            std::io::copy(&mut reader, &mut file).map(|_| ())
        }
    };
    if let Err(e) = copy_result {
        let _ = std::fs::remove_file(dest);
        return Err(e.into());
    }
    Ok(())
}
