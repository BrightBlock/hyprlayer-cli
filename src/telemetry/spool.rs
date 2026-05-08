use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::secure_fs::open_secure;

use super::{SPOOL_MAX_BYTES, event::Event, spool_path};

/// Orphan claim/rotate files older than this are assumed to belong to a
/// crashed previous run and are safe to delete. In-flight drains finish in
/// milliseconds, so an hour is plenty of headroom and far below any
/// practical "another live drain is still running" interval.
const ORPHAN_TTL: Duration = Duration::from_secs(3600);

fn open_secure_append(path: &Path) -> std::io::Result<fs::File> {
    open_secure(path, |o| {
        o.create(true).append(true);
    })
}

fn create_secure(path: &Path) -> std::io::Result<fs::File> {
    open_secure(path, |o| {
        o.write(true).create(true).truncate(true);
    })
}

/// Append one JSONL line. Single `write_all` of `<json>\n` keeps the
/// record atomic against concurrent appenders under PIPE_BUF + O_APPEND.
pub fn append(event: &Event) -> std::io::Result<()> {
    append_to(&spool_path()?, event)
}

pub fn append_to(path: &Path, event: &Event) -> std::io::Result<()> {
    rotate_if_needed_at(path)?;

    let json = serde_json::to_string(event).map_err(std::io::Error::other)?;
    let mut bytes = json.into_bytes();
    bytes.push(b'\n');

    // Steady-state hot path: parent dir exists, open succeeds, no
    // `create_dir_all` syscall walk. Only on first-record-after-purge
    // (or a missing parent) do we lazily create.
    let mut file = match open_secure_append(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            open_secure_append(path)?
        }
        Err(e) => return Err(e),
    };
    file.write_all(&bytes)?;
    Ok(())
}

/// Read every spooled event and remove the file. Caller is responsible for
/// re-spooling on partial failure.
///
/// Atomically claims the live spool by renaming it aside before reading.
/// Concurrent appends after the rename land in a fresh file at `path` and
/// are picked up on the next drain — they cannot collide with our reader.
pub fn drain() -> std::io::Result<Vec<Event>> {
    drain_at(&spool_path()?)
}

pub fn drain_at(path: &Path) -> std::io::Result<Vec<Event>> {
    sweep_orphan_drains(path, SystemTime::now());

    let claim = claim_path(path);
    match fs::rename(path, &claim) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    }

    let events = match read_events(&claim) {
        Ok(evs) => evs,
        Err(e) => {
            // Reading the claimed file failed mid-stream. Leave the claim
            // file in place so the next drain (or a human) can recover it
            // rather than silently dropping events. The TTL-based sweep
            // will eventually clean it up if no one revisits it.
            return Err(e);
        }
    };

    let _ = fs::remove_file(&claim);
    Ok(events)
}

/// Distinct read errors from parse errors:
/// - `BufRead::lines()` errors (I/O) propagate.
/// - `serde_json::from_str` errors are dropped silently — a malformed JSONL
///   line is unrecoverable, but the surrounding events are still fine.
fn read_events(path: &Path) -> std::io::Result<Vec<Event>> {
    let file = fs::File::open(path)?;
    let mut events = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Event>(&line) {
            events.push(event);
        }
    }
    Ok(events)
}

pub fn purge() -> std::io::Result<()> {
    purge_at(&spool_path()?)
}

pub fn purge_at(path: &Path) -> std::io::Result<()> {
    // `purge` is the user-explicit "drop everything" path; it's safe to
    // sweep aggressively here without an age cutoff because the user has
    // already committed to losing in-flight state.
    sweep_orphan_drains_force(path);
    match fs::remove_file(path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn depth() -> std::io::Result<(u64, usize)> {
    depth_at(&spool_path()?)
}

pub fn depth_at(path: &Path) -> std::io::Result<(u64, usize)> {
    let bytes = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(e) => return Err(e),
    };
    let count = BufReader::new(fs::File::open(path)?).lines().count();
    Ok((bytes, count))
}

fn rotate_if_needed_at(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if metadata.len() <= SPOOL_MAX_BYTES {
        return Ok(());
    }

    // Propagate read errors: a partial read followed by a rename would
    // overwrite the live spool with truncated content. Better to surface
    // the I/O failure and leave the original in place.
    let lines: Vec<String> = BufReader::new(fs::File::open(path)?)
        .lines()
        .collect::<std::io::Result<_>>()?;
    let drop_count = lines.len() / 2;
    let kept = lines.iter().skip(drop_count);

    let tmp = sibling_with_suffix(path, "rotate", "spool.jsonl");
    {
        let mut writer = create_secure(&tmp)?;
        writeln!(
            writer,
            "{}",
            serde_json::json!({
                "schema_version": super::event::SCHEMA_VERSION,
                "event_type": "truncated",
                "dropped_count": drop_count,
                "event_timestamp": super::rfc3339_now()
            })
        )?;
        for line in kept {
            writeln!(writer, "{line}")?;
        }
    }

    // Concurrent append between our read of `path` and this rename can
    // be lost — accepted because rotation only triggers at the 10 MiB
    // cap, which is rare.
    fs::rename(&tmp, path)?;
    Ok(())
}

fn claim_path(path: &Path) -> PathBuf {
    sibling_with_suffix(path, "draining", "spool.jsonl")
}

/// Build a sibling path of `path` with a unique `.<tag>.<pid>.<nanos>`
/// suffix. Both the pid and a high-resolution timestamp are folded in so
/// repeated drains within the same process don't collide.
fn sibling_with_suffix(path: &Path, tag: &str, fallback_name: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut buf = path.to_path_buf();
    buf.set_file_name(format!(
        "{}.{tag}.{pid}.{nanos}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| fallback_name.to_string())
    ));
    buf
}

/// Best-effort cleanup of `<spool>.draining.*` and `<spool>.rotate.*`
/// siblings left over from a crashed previous drain or rotate. Only
/// touches files whose mtime is older than `now - ORPHAN_TTL` — a
/// concurrent drain in flight has a fresh-mtime claim file that we
/// must not delete. Skipped silently on directory-read or metadata
/// errors (the next iteration will retry).
fn sweep_orphan_drains(path: &Path, now: SystemTime) {
    let Some(cutoff) = now.checked_sub(ORPHAN_TTL) else {
        return;
    };
    sweep_with_filter(path, |meta| {
        meta.modified().map(|m| m < cutoff).unwrap_or(false)
    });
}

/// Unconditional sweep — every matching sibling gets removed regardless
/// of age. Reserved for the `purge` user command, which has explicit
/// "throw it all away" semantics.
fn sweep_orphan_drains_force(path: &Path) {
    sweep_with_filter(path, |_| true);
}

fn sweep_with_filter(path: &Path, should_remove: impl Fn(&fs::Metadata) -> bool) {
    let Some(parent) = path.parent() else {
        return;
    };
    let Some(base) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let prefixes = [format!("{base}.draining."), format!("{base}.rotate.")];
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !prefixes.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if should_remove(&meta) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::HyprlayerConfig;
    use crate::telemetry::event::{Event, Outcome};

    fn temp_spool() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spool.jsonl");
        (dir, path)
    }

    #[test]
    fn append_and_drain_round_trip() {
        let (_dir, path) = temp_spool();
        let cfg = HyprlayerConfig::default();
        let e1 = Event::cli_command("test.one", 1, Outcome::Success, None, &cfg);
        let e2 = Event::cli_command("test.two", 2, Outcome::Failure, None, &cfg);

        append_to(&path, &e1).unwrap();
        append_to(&path, &e2).unwrap();

        let drained = drain_at(&path).unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].command.as_deref(), Some("test.one"));
        assert_eq!(drained[1].command.as_deref(), Some("test.two"));

        let drained_again = drain_at(&path).unwrap();
        assert!(drained_again.is_empty());
    }

    #[test]
    fn purge_removes_file() {
        let (_dir, path) = temp_spool();
        let cfg = HyprlayerConfig::default();
        let ev = Event::cli_command("x", 0, Outcome::Success, None, &cfg);
        append_to(&path, &ev).unwrap();
        assert!(path.exists());
        purge_at(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn depth_reports_bytes_and_count() {
        let (_dir, path) = temp_spool();
        let cfg = HyprlayerConfig::default();
        for i in 0..3 {
            let ev = Event::cli_command(&format!("c{i}"), i, Outcome::Success, None, &cfg);
            append_to(&path, &ev).unwrap();
        }
        let (bytes, count) = depth_at(&path).unwrap();
        assert_eq!(count, 3);
        assert!(bytes > 0);
    }

    #[test]
    fn drain_skips_unparseable_lines() {
        let (_dir, path) = temp_spool();
        let cfg = HyprlayerConfig::default();
        let ev = Event::cli_command("ok", 0, Outcome::Success, None, &cfg);
        append_to(&path, &ev).unwrap();

        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "not-valid-json").unwrap();
        drop(file);

        let drained = drain_at(&path).unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].command.as_deref(), Some("ok"));
    }

    #[test]
    fn purge_at_missing_file_is_ok() {
        let (_dir, path) = temp_spool();
        purge_at(&path).unwrap();
    }

    /// rename-then-drain semantics: an append landing AFTER drain claims
    /// the spool ends up in a fresh file at the original path and is
    /// picked up on the next drain rather than lost.
    #[test]
    fn append_after_drain_starts_lands_in_next_drain() {
        let (_dir, path) = temp_spool();
        let cfg = HyprlayerConfig::default();
        let pre = Event::cli_command("pre", 0, Outcome::Success, None, &cfg);
        append_to(&path, &pre).unwrap();

        // Simulate "drain claims, then a writer appends, then drain reads"
        // by claiming manually via rename and only dropping it after the
        // post-claim append has happened.
        let claim = claim_path(&path);
        fs::rename(&path, &claim).unwrap();
        let post = Event::cli_command("post", 0, Outcome::Success, None, &cfg);
        append_to(&path, &post).unwrap();
        let early = read_events(&claim).unwrap();
        let _ = fs::remove_file(&claim);

        assert_eq!(early.len(), 1);
        assert_eq!(early[0].command.as_deref(), Some("pre"));

        let later = drain_at(&path).unwrap();
        assert_eq!(later.len(), 1);
        assert_eq!(later[0].command.as_deref(), Some("post"));
    }

    /// Drain only sweeps claim files that are older than ORPHAN_TTL —
    /// fresh ones might belong to a concurrent live drain.
    #[test]
    fn sweep_skips_fresh_claims_keeps_old_ones() {
        let (_dir, path) = temp_spool();
        let fresh = path.with_file_name("spool.jsonl.draining.fresh");
        let stale = path.with_file_name("spool.jsonl.draining.stale");
        fs::write(&fresh, "live").unwrap();
        fs::write(&stale, "junk").unwrap();

        // Both files exist with mtime ≈ now. With `now` set to ORPHAN_TTL
        // in the past, neither has crossed the cutoff yet — no removal.
        let in_the_past = SystemTime::now()
            .checked_sub(ORPHAN_TTL)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        sweep_orphan_drains(&path, in_the_past);
        assert!(fresh.exists());
        assert!(stale.exists());

        // Pretend a long time has passed: now both files are older than
        // the cutoff and get swept.
        let way_in_the_future = SystemTime::now() + ORPHAN_TTL * 2;
        sweep_orphan_drains(&path, way_in_the_future);
        assert!(!fresh.exists());
        assert!(!stale.exists());
    }

    /// A live concurrent drain must NOT have its claim file removed by
    /// another drain that ran the sweep first. This is the regression
    /// the previous unconditional sweep introduced.
    #[test]
    fn concurrent_drains_do_not_clobber_each_other() {
        let (_dir, path) = temp_spool();
        let cfg = HyprlayerConfig::default();
        append_to(
            &path,
            &Event::cli_command("a", 0, Outcome::Success, None, &cfg),
        )
        .unwrap();

        // Drain A claims the spool but hasn't read yet.
        let claim_a = claim_path(&path);
        fs::rename(&path, &claim_a).unwrap();
        assert!(claim_a.exists());

        // Drain B starts: its sweep must leave A's fresh claim alone.
        let _ = drain_at(&path).unwrap();
        assert!(
            claim_a.exists(),
            "concurrent drain must not delete a fresh claim file"
        );

        // Cleanup so the test doesn't leak.
        let _ = fs::remove_file(&claim_a);
    }

    /// `purge` runs the unconditional sweep — it's the user's explicit
    /// "wipe everything" hatch and shouldn't honor the TTL.
    #[test]
    fn purge_force_sweeps_fresh_claims() {
        let (_dir, path) = temp_spool();
        let fresh = path.with_file_name("spool.jsonl.draining.fresh");
        fs::write(&fresh, "junk").unwrap();
        purge_at(&path).unwrap();
        assert!(!fresh.exists());
    }

    #[cfg(unix)]
    #[test]
    fn append_creates_file_with_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = temp_spool();
        let cfg = HyprlayerConfig::default();
        append_to(
            &path,
            &Event::cli_command("a", 0, Outcome::Success, None, &cfg),
        )
        .unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "spool file must not be world-readable");
    }

    /// `O_NOFOLLOW` must reject a pre-existing symlink at the spool path
    /// — otherwise an attacker who can pre-create the file gets a write
    /// redirect (and a chmod) on the link target.
    #[cfg(unix)]
    #[test]
    fn append_refuses_to_follow_symlink() {
        let (dir, path) = temp_spool();
        let target = dir.path().join("decoy.txt");
        fs::write(&target, b"original").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();

        let cfg = HyprlayerConfig::default();
        let err = append_to(
            &path,
            &Event::cli_command("a", 0, Outcome::Success, None, &cfg),
        )
        .unwrap_err();

        // Linux returns ELOOP, BSDs return EMLINK — accept either.
        let raw = err.raw_os_error();
        assert!(
            raw == Some(libc::ELOOP) || raw == Some(libc::EMLINK),
            "expected ELOOP/EMLINK on symlink, got {err:?} (errno {raw:?})"
        );

        // The decoy must not have been touched.
        assert_eq!(fs::read(&target).unwrap(), b"original");
    }
}
