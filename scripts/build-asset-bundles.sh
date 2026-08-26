#!/usr/bin/env bash
# Build the per-harness release asset bundles from the live `assets/` tree.
#
# Usage: scripts/build-asset-bundles.sh <version>
#   version: the release version, with or without a leading `v` (1.6.0, v1.6.0)
#
# For each harness it writes `dist/hyprlayer-assets-<harness>-<version>.tar.gz`
# containing that harness's tree plus a generated `manifest.json`. Paths inside
# the tarball are relative to the harness root — there is no repo-root
# component, so extraction needs no stripping.
#
# The supported Claude and Codex bundles are cut from `assets/<harness>/`,
# never from the frozen root trees retained for pre-1.6.0 clients (see
# `assets/FROZEN.md`).
#
# Every bundle is verified after packing: the tarball is extracted to a temp
# dir, its `manifest.json` is re-read, and every recorded SHA256 is compared
# against the extracted file's actual hash. The file sets must match exactly
# in both directions, so neither a dropped file nor an unlisted stowaway
# survives.
#
# Requires bash, tar, and coreutils (or macOS `shasum`). Output is
# byte-reproducible under GNU tar: fixed mtimes, sorted entries, no gzip
# timestamp.

set -euo pipefail

# Oldest CLI that may consume these bundles: 1.6.0 is the first release whose
# CLI ships `hyprlayer orchestrate`, which the declarative skills invoke.
min_cli_version="1.6.0"

harnesses=(claude codex)

# Paths are emitted into JSON and into a tar archive unescaped, so restrict
# them to characters that need neither. A name outside this set is a build
# failure, not something to quietly mangle.
safe_path_re='^[A-Za-z0-9._/-]+$'

die() {
  echo "build-asset-bundles: $*" >&2
  exit 1
}

# Resolved once at startup: `die` inside a command substitution would only
# kill the subshell, so a missing hasher has to be caught before first use.
if command -v sha256sum >/dev/null 2>&1; then
  sha256_cmd=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
  sha256_cmd=(shasum -a 256)
else
  die "no sha256sum or shasum on PATH"
fi

sha256_of() {
  "${sha256_cmd[@]}" "$1" | cut -d' ' -f1
}

# Every file under $1, path-relative, LC_ALL=C-sorted for a stable manifest.
list_files() {
  (cd "$1" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
}

raw_version="${1:-}"
[ -n "$raw_version" ] || die "usage: $0 <version>"
version="${raw_version#v}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.]+)?$ ]] ||
  die "version must be semver, got '$raw_version'"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist_dir="$repo_root/dist"
mkdir -p "$dist_dir"

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

tar_flags=(--format=ustar)
tar_version="$(tar --version 2>/dev/null || true)"
case "$tar_version" in
*"GNU tar"*)
  # Deterministic packing: same tree in, same bytes out, so a rebuilt tag
  # still matches the digest the release recorded.
  tar_flags+=(--sort=name --owner=0 --group=0 --numeric-owner --mtime=@0)
  ;;
esac

build_bundle() {
  local harness="$1"
  local src_dir="$repo_root/assets/$harness"
  local stage="$work_dir/stage-$harness"
  local manifest="$stage/manifest.json"
  local tarball="$dist_dir/hyprlayer-assets-$harness-$version.tar.gz"

  [ -d "$src_dir" ] || die "no live tree at assets/$harness"
  if [ -e "$src_dir/manifest.json" ]; then
    die "assets/$harness/manifest.json exists; it would collide with the generated one"
  fi

  # Symlinks, devices and the like are rejected by the extractor, so refuse
  # to pack a bundle no client could install.
  local irregular
  irregular="$(find "$src_dir" -mindepth 1 ! -type f ! -type d)"
  [ -z "$irregular" ] || die "assets/$harness has non-regular entries:
$irregular"

  local files=()
  local path
  while IFS= read -r path; do
    [[ "$path" =~ $safe_path_re ]] || die "unsupported path in assets/$harness: '$path'"
    files+=("$path")
  done < <(list_files "$src_dir")
  [ "${#files[@]}" -gt 0 ] || die "assets/$harness is empty"

  mkdir -p "$stage"
  cp -a "$src_dir/." "$stage/"

  {
    printf '{\n'
    printf '  "version": "%s",\n' "$version"
    printf '  "harness": "%s",\n' "$harness"
    printf '  "min_cli_version": "%s",\n' "$min_cli_version"
    printf '  "files": [\n'
    local i sep
    for i in "${!files[@]}"; do
      if [ "$i" -eq $((${#files[@]} - 1)) ]; then sep=""; else sep=","; fi
      printf '    {"path": "%s", "sha256": "%s"}%s\n' \
        "${files[$i]}" "$(sha256_of "$stage/${files[$i]}")" "$sep"
    done
    printf '  ]\n'
    printf '}\n'
  } >"$manifest"

  # Pack the top-level entries by name rather than `.`, so archive paths are
  # bare (`agents/foo.md`) with no leading `./` component for the extractor
  # to reason about.
  local top=()
  local entry
  while IFS= read -r entry; do
    top+=("$entry")
  done < <(cd "$stage" && find . -mindepth 1 -maxdepth 1 | sed 's|^\./||' | LC_ALL=C sort)

  rm -f "$tarball"
  tar -c "${tar_flags[@]}" -C "$stage" -- "${top[@]}" | gzip -n -9 >"$tarball"

  verify_bundle "$harness" "$tarball" "${#files[@]}"
  printf '%s (%d files, %s)\n' "$tarball" "${#files[@]}" \
    "$(du -h "$tarball" | cut -f1)"
}

# Extract the finished tarball and re-hash every file the manifest claims.
verify_bundle() {
  local harness="$1" tarball="$2" expected_count="$3"
  local out="$work_dir/verify-$harness"

  mkdir -p "$out"
  tar -xzf "$tarball" -C "$out"
  [ -f "$out/manifest.json" ] || die "$harness bundle has no manifest.json"

  local listed=0 path want got
  while read -r path want; do
    [ -f "$out/$path" ] || die "$harness manifest lists '$path', missing from the tarball"
    got="$(sha256_of "$out/$path")"
    [ "$got" = "$want" ] ||
      die "$harness manifest digest for '$path' is $want, extracted file hashes to $got"
    listed=$((listed + 1))
  done < <(sed -n 's/^ *{"path": "\(.*\)", "sha256": "\([0-9a-f]\{64\}\)"},\{0,1\}$/\1 \2/p' \
    "$out/manifest.json")

  [ "$listed" -eq "$expected_count" ] ||
    die "$harness manifest lists $listed files, expected $expected_count"

  # Nothing may ride along unlisted: no Rust sources, no frozen-tree files.
  local packed
  packed="$(list_files "$out" | grep -vx 'manifest.json' | wc -l)"
  [ "$packed" -eq "$expected_count" ] ||
    die "$harness bundle carries $packed files but the manifest lists $expected_count"
}

for harness in "${harnesses[@]}"; do
  build_bundle "$harness"
done
