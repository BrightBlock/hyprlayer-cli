//! Cross-platform secure file open. Used for spool and config writes that
//! hold telemetry identifiers and the org-managed API key.
//!
//! On Unix:
//! - `mode(0o600)` so the file isn't world-readable under any umask
//! - `O_NOFOLLOW` so a pre-existing symlink at the leaf can't redirect
//!   the write
//! - post-open `set_permissions(0o600)` to tighten any pre-existing file
//!   created by an earlier hyprlayer version
//!
//! On Windows the user config dir is already access-controlled, so we
//! keep default semantics there.

use std::fs;
use std::path::Path;

pub fn open_secure(
    path: &Path,
    configure: impl FnOnce(&mut fs::OpenOptions),
) -> std::io::Result<fs::File> {
    let mut opts = fs::OpenOptions::new();
    configure(&mut opts);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    let file = opts.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
    Ok(file)
}
