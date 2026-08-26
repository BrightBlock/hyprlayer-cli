//! Hardened tar.gz extraction for the two bundle shapes an install can
//! come from: a per-harness release asset (`extract_bundle`, paths already
//! relative to the harness root) and a whole-repo `codeload.github.com`
//! tarball from which one tool's subtree is taken (`extract_subdir`).
//!
//! `codeload.github.com` is not on the REST core rate-limit bucket, so the
//! repo-tarball path replaces ~30 rate-limited Contents API calls per
//! install with one plain HTTPS download plus local extraction. See
//! `download_directory` in `agents.rs` for the API-based walk this augments
//! and falls back to.

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use std::fs;
use std::path::{Component, Path};
use std::time::Duration;
use tar::Archive as TarArchive;

use super::REPO;
use crate::http;

/// Full repo tarball is ~360 KB today and a per-harness release bundle is
/// smaller still. 64 MiB is two orders of magnitude of headroom and stops a
/// hostile or misconfigured source from filling the temp dir before we ever
/// open the archive.
pub(crate) const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn codeload_url(git_ref: &str) -> String {
    format!("https://codeload.github.com/{REPO}/tar.gz/{git_ref}")
}

/// Download the repo archive pinned to `git_ref` and extract `repo_path`'s
/// subtree into `dest`. Returns the number of files written.
pub(crate) fn fetch_and_extract(repo_path: &str, git_ref: &str, dest: &Path) -> Result<usize> {
    let url = codeload_url(git_ref);
    let tmp = tempfile::NamedTempFile::new()
        .context("Failed to create a temp file for the archive download")?;
    http::download_file_capped(
        &url,
        tmp.path(),
        Duration::from_secs(30),
        Some(MAX_ARCHIVE_BYTES),
    )
    .map_err(|e| anyhow::anyhow!("Failed to download archive from {url}: {e}"))?;
    extract_subdir(tmp.path(), repo_path, dest)
}

/// How the archive's leading path component is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootHandling {
    /// The archive wraps everything in exactly one root component, derived
    /// from the first entry rather than hardcoded — codeload's prefix is
    /// `hyprlayer-cli-master/` for a branch ref but `hyprlayer-cli-<sha>/`
    /// for a SHA ref. Every subsequent entry must share it, and it is
    /// stripped before matching `subdir`.
    StripSingleRoot,
    /// Entries are already relative to the bundle root, as in the
    /// per-harness release assets `scripts/build-asset-bundles.sh` packs.
    /// Nothing is stripped and no shared prefix is required.
    NoRootComponent,
}

/// Extract every entry under `<root>/<subdir>/` into `dest`, stripping both
/// the archive's single root component and `subdir`. Returns the file
/// count. This is the codeload repo-tarball shape.
pub(crate) fn extract_subdir(archive: &Path, subdir: &str, dest: &Path) -> Result<usize> {
    extract(archive, dest, RootHandling::StripSingleRoot, Some(subdir))
}

/// Extract every entry of a release-asset bundle into `dest`. The bundle's
/// paths are already relative to the harness root (`agents/foo.md`), so
/// there is no root component to require or strip and no subtree to select
/// — but every hardening rule in `extract` applies unchanged.
pub(crate) fn extract_bundle(archive: &Path, dest: &Path) -> Result<usize> {
    extract(archive, dest, RootHandling::NoRootComponent, None)
}

/// Shared extraction core behind `extract_subdir` and `extract_bundle`.
/// `subdir` selects a subtree to extract (after any root stripping);
/// `None` takes the whole archive.
///
/// Every rule below is a rejection, not a skip, applied to *every* entry in
/// the archive — not just the ones under `subdir` — because a bundle that
/// fails any of them is not a bundle we published and the whole archive is
/// untrusted:
///
/// - Entry type must be `Regular` or `Directory`. Symlinks, hardlinks,
///   char/block devices, FIFOs and GNU long-name extensions are rejected.
///   `dest` is a live user directory (e.g. `~/.claude`); a symlink entry is
///   an arbitrary-file-write primitive. The one exception is a PAX global
///   extended header (`git archive` always emits one, carrying metadata
///   like `comment=<commit sha>`) — it names no path and is skipped rather
///   than rejected.
/// - No path component may be `..`, a root component, or a Windows prefix
///   (`C:`). This is checked via `Component` variants rather than string
///   matching, so backslash separators or a bare leading `/` can't slip
///   through on any platform.
/// - Under `RootHandling::StripSingleRoot`, the archive must have exactly
///   one root component and every entry must share it.
///
/// Entries outside `<root>/<subdir>/` are silently skipped once they've
/// passed the checks above — that's just the other content in the repo.
fn extract(
    archive: &Path,
    dest: &Path,
    root_handling: RootHandling,
    subdir: Option<&str>,
) -> Result<usize> {
    fs::create_dir_all(dest)
        .with_context(|| format!("Failed to create destination {}", dest.display()))?;

    let file =
        fs::File::open(archive).with_context(|| format!("Failed to open {}", archive.display()))?;
    let decoder = GzDecoder::new(file);
    let mut tar = TarArchive::new(decoder);

    let mut root: Option<String> = None;
    let mut count = 0usize;

    let entries = tar.entries().context("Failed to read archive entries")?;
    for entry in entries {
        let mut entry = entry.context("Failed to read an archive entry")?;
        let entry_type = entry.header().entry_type();
        // A PAX global extended header (`git archive`'s tarballs carry one,
        // holding e.g. `comment=<commit sha>`) applies to the archive as a
        // whole rather than naming a file, and isn't auto-consumed by the
        // `tar` crate the way a per-entry local extension header is. It
        // carries no path data, so it's metadata to skip, not a rejection.
        if entry_type.is_pax_global_extensions() {
            continue;
        }
        if !(entry_type.is_file() || entry_type.is_dir()) {
            let path = entry.path().ok();
            anyhow::bail!(
                "archive entry {:?} has disallowed type {:?}",
                path,
                entry_type
            );
        }

        let path = entry
            .path()
            .with_context(|| "archive entry has an invalid (non-UTF8 or malformed) path")?
            .into_owned();

        let mut parts: Vec<std::ffi::OsString> = Vec::new();
        for component in path.components() {
            match component {
                // A `./` prefix is a harmless, common tar quirk.
                Component::CurDir => {}
                Component::Normal(part) => parts.push(part.to_owned()),
                other => anyhow::bail!(
                    "archive entry has a disallowed path component {:?}: {}",
                    other,
                    path.display()
                ),
            }
        }
        if parts.is_empty() {
            anyhow::bail!("archive entry has an empty path");
        }

        // Everything the entry contributes below the archive's own root.
        let rest: &[std::ffi::OsString] = match root_handling {
            RootHandling::StripSingleRoot => {
                let (root_component, rest) = parts
                    .split_first()
                    .expect("parts is non-empty, checked above");
                let root_component = root_component.to_string_lossy().into_owned();
                match &root {
                    None => root = Some(root_component),
                    Some(expected) if *expected == root_component => {}
                    Some(expected) => anyhow::bail!(
                        "archive has multiple root components: expected {:?}, found {:?}",
                        expected,
                        root_component
                    ),
                }
                rest
            }
            RootHandling::NoRootComponent => &parts,
        };

        // Compare `rest` against `subdir`'s own components rather than
        // joining into a `PathBuf` and calling `strip_prefix`, so this still
        // works if `subdir` itself contains a path separator. With no
        // `subdir` the list is empty and every entry matches.
        let subdir_parts: Vec<&str> = subdir.map(|s| s.split('/').collect()).unwrap_or_default();
        if rest.len() < subdir_parts.len()
            || !rest
                .iter()
                .zip(subdir_parts.iter())
                .all(|(part, want)| part == want)
        {
            continue; // outside the requested subdir — not ours
        }
        let relative = &rest[subdir_parts.len()..];
        if relative.is_empty() {
            continue; // the subdir's own root entry — nothing to write
        }

        let mut dest_path = dest.to_path_buf();
        dest_path.extend(relative);

        if entry_type.is_dir() {
            fs::create_dir_all(&dest_path)
                .with_context(|| format!("Failed to create {}", dest_path.display()))?;
            assert_within(dest, &dest_path)?;
            continue;
        }

        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        assert_within(dest, dest_path.parent().unwrap_or(dest))?;

        let mut out = fs::File::create(&dest_path)
            .with_context(|| format!("Failed to create {}", dest_path.display()))?;
        std::io::copy(&mut entry, &mut out)
            .with_context(|| format!("Failed to write {}", dest_path.display()))?;
        count += 1;
    }

    Ok(count)
}

/// Belt-and-suspenders check on top of the component-level validation
/// above: canonicalize `candidate` and assert it is still inside `root`.
fn assert_within(root: &Path, candidate: &Path) -> Result<()> {
    let canon_root = root
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize {}", root.display()))?;
    let canon = candidate
        .canonicalize()
        .with_context(|| format!("Failed to canonicalize {}", candidate.display()))?;
    if !canon.starts_with(&canon_root) {
        anyhow::bail!(
            "archive entry {} escaped destination {}",
            candidate.display(),
            root.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    enum TestEntry {
        Dir(&'static str),
        File(&'static str, &'static [u8]),
        Symlink(&'static str, &'static str),
        HardLink(&'static str, &'static str),
        /// An absolute path, via the crate's own (validated) absolute-path
        /// support — exercises the "root component" rejection path.
        AbsoluteFile(&'static str, &'static [u8]),
        /// A raw, unvalidated name written directly into the header bytes.
        /// `tar::Header::set_path` refuses to build a `..`-containing path
        /// at all, so a `..`-traversal entry can only be constructed this
        /// way — exactly how a hand-crafted hostile archive would do it.
        RawPathFile(&'static str, &'static [u8]),
        /// A PAX global extended header, as `git archive` always emits at
        /// the start of a real tarball.
        GlobalPaxHeader(&'static [u8]),
    }

    fn build_archive(entries: &[TestEntry]) -> Vec<u8> {
        let mut tar_bytes: Vec<u8> = Vec::new();
        {
            let enc = GzEncoder::new(&mut tar_bytes, Compression::default());
            let mut builder = tar::Builder::new(enc);
            for entry in entries {
                let mut header = tar::Header::new_gnu();
                match entry {
                    TestEntry::Dir(path) => {
                        header.set_entry_type(tar::EntryType::Directory);
                        header.set_path(path).unwrap();
                        header.set_size(0);
                        header.set_mode(0o755);
                        header.set_cksum();
                        builder.append(&header, std::io::empty()).unwrap();
                    }
                    TestEntry::File(path, data) => {
                        header.set_entry_type(tar::EntryType::Regular);
                        header.set_path(path).unwrap();
                        header.set_size(data.len() as u64);
                        header.set_mode(0o644);
                        header.set_cksum();
                        builder.append(&header, *data).unwrap();
                    }
                    TestEntry::Symlink(path, target) => {
                        header.set_entry_type(tar::EntryType::Symlink);
                        header.set_path(path).unwrap();
                        header.set_link_name(target).unwrap();
                        header.set_size(0);
                        header.set_cksum();
                        builder.append(&header, std::io::empty()).unwrap();
                    }
                    TestEntry::HardLink(path, target) => {
                        header.set_entry_type(tar::EntryType::Link);
                        header.set_path(path).unwrap();
                        header.set_link_name(target).unwrap();
                        header.set_size(0);
                        header.set_cksum();
                        builder.append(&header, std::io::empty()).unwrap();
                    }
                    TestEntry::AbsoluteFile(path, data) => {
                        header.set_entry_type(tar::EntryType::Regular);
                        header.set_path_absolute(path).unwrap();
                        header.set_size(data.len() as u64);
                        header.set_mode(0o644);
                        header.set_cksum();
                        builder.append(&header, *data).unwrap();
                    }
                    TestEntry::RawPathFile(name, data) => {
                        header.set_entry_type(tar::EntryType::Regular);
                        let bytes = header.as_mut_bytes();
                        bytes[0..100].fill(0);
                        let name_bytes = name.as_bytes();
                        bytes[0..name_bytes.len()].copy_from_slice(name_bytes);
                        header.set_size(data.len() as u64);
                        header.set_mode(0o644);
                        header.set_cksum();
                        builder.append(&header, *data).unwrap();
                    }
                    TestEntry::GlobalPaxHeader(data) => {
                        header.set_entry_type(tar::EntryType::XGlobalHeader);
                        header.set_path("pax_global_header").unwrap();
                        header.set_size(data.len() as u64);
                        header.set_cksum();
                        builder.append(&header, *data).unwrap();
                    }
                }
            }
            let enc = builder.into_inner().unwrap();
            enc.finish().unwrap();
        }
        tar_bytes
    }

    fn write_archive(dir: &Path, entries: &[TestEntry]) -> std::path::PathBuf {
        let bytes = build_archive(entries);
        let path = dir.join("bundle.tar.gz");
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn happy_path_with_nested_subdirectories() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::Dir("repo-abc/"),
                TestEntry::Dir("repo-abc/claude/"),
                TestEntry::Dir("repo-abc/claude/skills/"),
                TestEntry::Dir("repo-abc/claude/skills/foo/"),
                TestEntry::File(
                    "repo-abc/claude/skills/foo/SKILL.md",
                    b"---\nname: foo\n---\n",
                ),
                TestEntry::File("repo-abc/claude/settings.json", b"{}"),
            ],
        );
        let dest = tmp.path().join("dest");
        let count = extract_subdir(&archive, "claude", &dest).unwrap();
        assert_eq!(count, 2);
        assert_eq!(
            fs::read_to_string(dest.join("skills/foo/SKILL.md")).unwrap(),
            "---\nname: foo\n---\n"
        );
        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn pax_global_header_skipped_not_rejected() {
        // `git archive`-generated tarballs (what codeload serves) always
        // lead with a PAX global extended header before the real root
        // entry. It must not be treated as a disallowed entry type.
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::GlobalPaxHeader(
                    b"52 comment=1f7370976053d293da0718c00aab5faa78396e6a\n",
                ),
                TestEntry::Dir("repo-abc/"),
                TestEntry::File("repo-abc/claude/settings.json", b"{}"),
            ],
        );
        let dest = tmp.path().join("dest");
        let count = extract_subdir(&archive, "claude", &dest).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            fs::read_to_string(dest.join("settings.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn traversal_entry_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("repo-abc/claude/settings.json", b"{}"),
                TestEntry::RawPathFile("repo-abc/claude/../../evil", b"pwned"),
            ],
        );
        let dest = tmp.path().join("dest");
        let err = extract_subdir(&archive, "claude", &dest).unwrap_err();
        assert!(
            err.to_string().contains("disallowed path component"),
            "unexpected error: {err}"
        );
        assert!(!dest.parent().unwrap().join("evil").exists());
    }

    #[test]
    fn absolute_path_entry_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("repo-abc/claude/settings.json", b"{}"),
                TestEntry::AbsoluteFile("/etc/passwd", b"pwned"),
            ],
        );
        let dest = tmp.path().join("dest");
        assert!(extract_subdir(&archive, "claude", &dest).is_err());
        assert!(!dest.parent().unwrap().join("etc/passwd").exists());
    }

    #[test]
    fn symlink_entry_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[TestEntry::Symlink(
                "repo-abc/claude/evil-link",
                "/etc/passwd",
            )],
        );
        let dest = tmp.path().join("dest");
        assert!(extract_subdir(&archive, "claude", &dest).is_err());
    }

    #[test]
    fn hardlink_entry_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("repo-abc/claude/real", b"data"),
                TestEntry::HardLink("repo-abc/claude/evil-hardlink", "repo-abc/claude/real"),
            ],
        );
        let dest = tmp.path().join("dest");
        assert!(extract_subdir(&archive, "claude", &dest).is_err());
    }

    #[test]
    fn multiple_roots_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("root-a/claude/foo.md", b"a"),
                TestEntry::File("root-b/claude/bar.md", b"b"),
            ],
        );
        let dest = tmp.path().join("dest");
        let err = extract_subdir(&archive, "claude", &dest).unwrap_err();
        assert!(
            err.to_string().contains("multiple root"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn entries_outside_subdir_not_written() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("repo-abc/claude/agents/a.md", b"a"),
                TestEntry::File("repo-abc/unrelated/agents/b.md", b"b"),
                TestEntry::File("repo-abc/README.md", b"c"),
            ],
        );
        let dest = tmp.path().join("dest");
        let count = extract_subdir(&archive, "claude", &dest).unwrap();
        assert_eq!(count, 1);
        assert!(dest.join("agents/a.md").is_file());
        assert!(!dest.join("../unrelated").exists());
        let mut walked = Vec::new();
        for entry in walkdir(&dest) {
            walked.push(entry);
        }
        assert_eq!(walked, vec![dest.join("agents/a.md")]);
    }

    fn walkdir(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Ok(read) = fs::read_dir(dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    out.extend(walkdir(&path));
                } else {
                    out.push(path);
                }
            }
        }
        out
    }

    #[test]
    fn empty_archive_yields_zero_files() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(tmp.path(), &[]);
        let dest = tmp.path().join("dest");
        let count = extract_subdir(&archive, "claude", &dest).unwrap();
        assert_eq!(count, 0);
        assert!(dest.is_dir());
    }

    /// A release-asset bundle has no `<repo>-<sha>/` wrapper: paths are
    /// already relative to the harness root.
    #[test]
    fn extract_bundle_takes_root_relative_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::Dir("agents/"),
                TestEntry::File("agents/codebase-locator.md", b"locator"),
                TestEntry::Dir("skills/"),
                TestEntry::Dir("skills/code_review/"),
                TestEntry::File("skills/code_review/SKILL.md", b"---\nname: cr\n---\n"),
                TestEntry::File("manifest.json", b"{}"),
            ],
        );
        let dest = tmp.path().join("dest");
        let count = extract_bundle(&archive, &dest).unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            fs::read_to_string(dest.join("agents/codebase-locator.md")).unwrap(),
            "locator"
        );
        assert_eq!(
            fs::read_to_string(dest.join("skills/code_review/SKILL.md")).unwrap(),
            "---\nname: cr\n---\n"
        );
        assert_eq!(
            fs::read_to_string(dest.join("manifest.json")).unwrap(),
            "{}"
        );
    }

    /// The whole point of the second entry point: `extract_subdir` would
    /// treat `agents` as the archive root and find nothing, so the bundle
    /// path must not silently reuse it.
    #[test]
    fn extract_subdir_finds_nothing_in_a_root_relative_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[TestEntry::File("agents/codebase-locator.md", b"locator")],
        );
        let dest = tmp.path().join("dest");
        assert_eq!(extract_subdir(&archive, "claude", &dest).unwrap(), 0);
    }

    /// Multiple top-level directories are normal in a bundle — the
    /// single-root rule must not apply to this entry point.
    #[test]
    fn extract_bundle_allows_multiple_top_level_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("agents/a.md", b"a"),
                TestEntry::File("skills/b/SKILL.md", b"b"),
                TestEntry::File("settings.json", b"{}"),
            ],
        );
        let dest = tmp.path().join("dest");
        assert_eq!(extract_bundle(&archive, &dest).unwrap(), 3);
    }

    #[test]
    fn extract_bundle_rejects_traversal_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("agents/a.md", b"a"),
                TestEntry::RawPathFile("../evil", b"pwned"),
            ],
        );
        let dest = tmp.path().join("dest");
        let err = extract_bundle(&archive, &dest).unwrap_err();
        assert!(
            err.to_string().contains("disallowed path component"),
            "unexpected error: {err}"
        );
        assert!(!dest.parent().unwrap().join("evil").exists());
    }

    #[test]
    fn extract_bundle_rejects_absolute_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("agents/a.md", b"a"),
                TestEntry::AbsoluteFile("/etc/passwd", b"pwned"),
            ],
        );
        let dest = tmp.path().join("dest");
        assert!(extract_bundle(&archive, &dest).is_err());
        assert!(!dest.parent().unwrap().join("etc/passwd").exists());
    }

    #[test]
    fn extract_bundle_rejects_symlink_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[TestEntry::Symlink("evil-link", "/etc/passwd")],
        );
        let dest = tmp.path().join("dest");
        assert!(extract_bundle(&archive, &dest).is_err());
    }

    #[test]
    fn extract_bundle_rejects_hardlink_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::File("agents/real", b"data"),
                TestEntry::HardLink("agents/evil-hardlink", "agents/real"),
            ],
        );
        let dest = tmp.path().join("dest");
        assert!(extract_bundle(&archive, &dest).is_err());
    }

    #[test]
    fn extract_bundle_skips_pax_global_header() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(
            tmp.path(),
            &[
                TestEntry::GlobalPaxHeader(b"20 comment=whatever\n"),
                TestEntry::File("agents/a.md", b"a"),
            ],
        );
        let dest = tmp.path().join("dest");
        assert_eq!(extract_bundle(&archive, &dest).unwrap(), 1);
    }

    #[test]
    fn extract_bundle_empty_archive_yields_zero_files() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = write_archive(tmp.path(), &[]);
        let dest = tmp.path().join("dest");
        assert_eq!(extract_bundle(&archive, &dest).unwrap(), 0);
        assert!(dest.is_dir());
    }

    #[test]
    fn codeload_url_branch_ref_shape() {
        let url = codeload_url("master");
        assert_eq!(
            url,
            "https://codeload.github.com/BrightBlock/hyprlayer-cli/tar.gz/master"
        );
    }

    #[test]
    fn codeload_url_sha_ref_shape() {
        let sha = "1f7370976053d293da0718c00aab5faa78396e6a";
        let url = codeload_url(sha);
        assert_eq!(
            url,
            format!("https://codeload.github.com/BrightBlock/hyprlayer-cli/tar.gz/{sha}")
        );
    }

    /// Sanity check that our in-memory tarball builder round-trips through
    /// `flate2`/`tar` the way a real codeload response would.
    #[test]
    fn build_archive_round_trips_through_gzip() {
        let bytes = build_archive(&[TestEntry::File("root/claude/x.md", b"hello")]);
        let decoder = GzDecoder::new(bytes.as_slice());
        let mut tar = TarArchive::new(decoder);
        let mut names = Vec::new();
        for entry in tar.entries().unwrap() {
            let entry = entry.unwrap();
            names.push(entry.path().unwrap().to_string_lossy().into_owned());
        }
        assert_eq!(names, vec!["root/claude/x.md".to_string()]);
    }
}
