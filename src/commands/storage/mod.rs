pub mod info;
pub mod set_database_id;
pub mod set_type_id;

/// Shared test helpers for storage-command tests. Both `set_database_id` and
/// `set_type_id` tests mutate the process-wide current-working-directory;
/// without a shared mutex, cargo's parallel runner lets them race across
/// module boundaries and one test observes a cwd pointing at another test's
/// dropped tempdir. Keep the lock here so every caller picks up the same
/// static.
#[cfg(test)]
pub(crate) mod test_util {
    use std::path::Path;
    use std::sync::Mutex;

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    pub fn with_cwd<F: FnOnce()>(dir: &Path, f: F) {
        let _guard = CWD_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir).unwrap();
        f();
        std::env::set_current_dir(prev).unwrap();
    }
}
