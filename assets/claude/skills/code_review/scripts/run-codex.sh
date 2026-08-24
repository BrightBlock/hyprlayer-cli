#!/usr/bin/env bash
# Run codex in challenge mode with JSONL streaming, piped through `hyprlayer codex stream`.
# Usage: run-codex.sh <prompt> <effort> [model]
#   prompt: the full adversarial prompt (must include the filesystem-boundary header)
#   effort: high | xhigh
#   model:  optional, passed via -m
# Reads `_REPO_ROOT` from the environment; falls back to `git rev-parse --show-toplevel`.
# Exit code: 0 on codex success, 124 if the inner timeout fired, otherwise codex's exit code.
# Stderr from codex is captured to a temp file and a one-line auth hint is printed if it
# looks like an auth/login error. The temp file is removed on exit.
#
# Cross-platform notes: requires bash (works on macOS, Linux, and Windows under git-bash /
# MSYS2 / WSL). Cwd-independent — locates the repo via `git rev-parse` or the inherited
# `_REPO_ROOT` env var. Uses `mktemp` without a hardcoded path so it honors `$TMPDIR` (and
# falls back to `/tmp` on systems without `$TMPDIR`).

set -u

PROMPT="${1:?adversarial prompt required}"
EFFORT="${2:?effort (high|xhigh) required}"
MODEL="${3:-}"

REPO_ROOT="${_REPO_ROOT:-$(git rev-parse --show-toplevel 2>/dev/null)}"
if [ -z "$REPO_ROOT" ]; then
  echo "not in a git repo" >&2
  exit 1
fi

TMPERR=$(mktemp -t codex-err.XXXXXX 2>/dev/null || mktemp "${TMPDIR:-/tmp}/codex-err-XXXXXX")
trap 'rm -f "$TMPERR"' EXIT

TIMEOUT_BIN=$(command -v gtimeout || command -v timeout || echo "")

CMD=(codex exec "$PROMPT" -C "$REPO_ROOT" -s read-only \
  -c "model_reasoning_effort=\"$EFFORT\"" \
  --enable web_search_cached --json)
[ -n "$MODEL" ] && CMD+=(-m "$MODEL")

if [ -n "$TIMEOUT_BIN" ]; then
  "$TIMEOUT_BIN" 600 "${CMD[@]}" < /dev/null 2>"$TMPERR" | hyprlayer codex stream
else
  "${CMD[@]}" < /dev/null 2>"$TMPERR" | hyprlayer codex stream
fi
EXIT=${PIPESTATUS[0]}

if [ "$EXIT" = "124" ]; then
  echo "codex stalled past 10 minutes. Re-run, or check ~/.codex/log/." >&2
fi

if grep -qiE "auth|login|unauthorized" "$TMPERR" 2>/dev/null; then
  echo "[codex auth error] $(head -1 "$TMPERR")" >&2
fi

exit "$EXIT"
