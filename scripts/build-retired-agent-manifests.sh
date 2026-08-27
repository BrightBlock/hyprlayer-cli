#!/usr/bin/env bash
# Generate the digest allowlists used to remove Copilot and OpenCode files
# written by Hyprlayer clients before the paired Claude + Codex migration.
#
# The old clients downloaded these trees from `master`, not from their own
# release tag. Consequently the allowlist covers every distinct blob in the
# trees' history through the last commit that still carried them. OpenCode's
# installer also resolved model placeholders, so the three generated provider
# variants are included alongside each source blob.

set -euo pipefail

retired_freeze=d705a48094606e267f817226f553a5c5a9764072
safe_path_re='^[A-Za-z0-9._/-]+$'

die() {
  echo "build-retired-agent-manifests: $*" >&2
  exit 1
}

if command -v sha256sum >/dev/null 2>&1; then
  sha256_cmd=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
  sha256_cmd=(shasum -a 256)
else
  die "no sha256sum or shasum on PATH"
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out_dir="$repo_root/src/agents/frozen"
mkdir -p "$out_dir"

history_pairs() {
  local harness="$1"
  git -C "$repo_root" log --format=%H "$retired_freeze" -- "$harness" |
    while read -r commit; do
      git -C "$repo_root" ls-tree -r "$commit" -- "$harness"
    done |
    awk -v prefix="$harness/" '{ path=$4; sub("^" prefix, "", path); print path " " $3 }' |
    sort -u
}

hash_blob() {
  local blob="$1"
  shift
  git -C "$repo_root" cat-file blob "$blob" | "$@" | "${sha256_cmd[@]}" | cut -d' ' -f1
}

emit_digest() {
  local path="$1"
  local digest="$2"
  printf '%s\t%s\n' "$path" "$digest"
}

render_manifest() {
  local records="$1"
  local out="$2"
  local count index path digest separator
  count="$(wc -l <"$records" | tr -d ' ')"
  index=0
  {
    printf '[\n'
    while IFS=$'\t' read -r path digest; do
      index=$((index + 1))
      if [ "$index" -eq "$count" ]; then separator=""; else separator=","; fi
      printf '  {"path": "%s", "sha256": "%s"}%s\n' "$path" "$digest" "$separator"
    done <"$records"
    printf ']\n'
  } >"$out"
  printf '%s (%d ownership records)\n' "${out#"$repo_root"/}" "$count"
}

for harness in copilot opencode; do
  records="$(mktemp)"
  while read -r path blob; do
    [[ "$path" =~ $safe_path_re ]] || die "unsupported path in $harness history: '$path'"
    emit_digest "$path" "$(hash_blob "$blob" cat)"

    if [ "$harness" = opencode ]; then
      emit_digest "$path" "$(hash_blob "$blob" sed \
        -e 's|{{SONNET_MODEL}}|github-copilot/claude-sonnet-4.5|g' \
        -e 's|{{OPUS_MODEL}}|github-copilot/claude-opus-4.5|g' \
        -e 's|{{ADVERSARIAL_MODEL}}|github-copilot/gpt-5-codex|g')"
      emit_digest "$path" "$(hash_blob "$blob" sed \
        -e 's|{{SONNET_MODEL}}|anthropic/claude-sonnet-4-5|g' \
        -e 's|{{OPUS_MODEL}}|anthropic/claude-opus-4-5|g' \
        -e 's|{{ADVERSARIAL_MODEL}}|anthropic/claude-opus-4-5|g')"
      emit_digest "$path" "$(hash_blob "$blob" sed \
        -e 's|{{SONNET_MODEL}}|abacus/claude-sonnet-4-6|g' \
        -e 's|{{OPUS_MODEL}}|abacus/claude-opus-4-6|g' \
        -e 's|{{ADVERSARIAL_MODEL}}|abacus/gpt-5.3-codex-xhigh|g')"
    fi
  done < <(history_pairs "$harness") | sort -u >"$records"

  render_manifest "$records" "$out_dir/$harness-retired.json"
  rm -f -- "$records"
done
