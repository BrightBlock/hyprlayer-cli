#!/usr/bin/env bash
# Generate the frozen-tree manifests the installer uses as its stand-in for
# the record a pre-1.6.0 install never wrote.
#
# Usage: scripts/build-frozen-manifests.sh
#
# For each harness it writes `src/agents/frozen/<harness>.json`: every file
# in the frozen root tree (`claude/` — see
# `assets/FROZEN.md`) with its SHA256, path-relative to the harness root.
# The binary embeds these with `include_str!`, and
# `agents::tests::frozen_manifests_match_the_frozen_trees` asserts they
# still describe the trees exactly.
#
# The trees are frozen, so this is a one-shot generator: it only needs
# re-running if a frozen tree is deliberately changed, which per
# `assets/FROZEN.md` should not happen.
#
# Path and digest conventions match `scripts/build-asset-bundles.sh`, so a
# frozen entry and a bundle entry for the same path compare directly.

set -euo pipefail

harnesses=(claude)

# Paths are emitted into JSON unescaped, so restrict them to characters that
# need no escaping. A name outside this set is a failure, not something to
# quietly mangle.
safe_path_re='^[A-Za-z0-9._/-]+$'

die() {
  echo "build-frozen-manifests: $*" >&2
  exit 1
}

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

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="$repo_root/src/agents/frozen"
mkdir -p "$out_dir"

for harness in "${harnesses[@]}"; do
  src_dir="$repo_root/$harness"
  out="$out_dir/$harness.json"
  [ -d "$src_dir" ] || die "no frozen tree at $harness/"

  files=()
  while IFS= read -r path; do
    [[ "$path" =~ $safe_path_re ]] || die "unsupported path in $harness/: '$path'"
    files+=("$path")
  done < <(cd "$src_dir" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
  [ "${#files[@]}" -gt 0 ] || die "$harness/ is empty"

  {
    printf '[\n'
    for i in "${!files[@]}"; do
      if [ "$i" -eq $((${#files[@]} - 1)) ]; then sep=""; else sep=","; fi
      printf '  {"path": "%s", "sha256": "%s"}%s\n' \
        "${files[$i]}" "$(sha256_of "$src_dir/${files[$i]}")" "$sep"
    done
    printf ']\n'
  } >"$out"

  printf '%s (%d files)\n' "${out#"$repo_root"/}" "${#files[@]}"
done
