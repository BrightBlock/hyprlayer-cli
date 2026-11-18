#!/usr/bin/env bash
# E2E: drives assets/claude/skills/{implement,validate}_plan through headless Claude on a
# fixture repo and asserts how manual items end up ticked. Spends real tokens (~5 sessions).
# Output: target/e2e/manual-verification/ (plan snapshots, transcripts, results.txt).
set -euo pipefail

WT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$WT/target/e2e/manual-verification"
FX="$OUT/fixture"
PLAN_REL="thoughts/shared/plans/2026-11-18-greet-flags.md"
PLAN="$FX/$PLAN_REL"
BUDGET="${E2E_BUDGET_USD:-8}"

rm -rf "$OUT" && mkdir -p "$OUT/bin" "$FX"
(cd "$WT" && cargo build -q)

# Real binary for orchestrate/storage; `thoughts sync` is a no-op so nothing leaves the fixture.
cat > "$OUT/bin/hyprlayer" <<EOF
#!/usr/bin/env bash
if [[ "\${1:-}" == thoughts && "\${2:-}" == sync ]]; then echo "e2e: thoughts sync disabled" >&2; exit 0; fi
exec "$WT/target/debug/hyprlayer" "\$@"
EOF
chmod +x "$OUT/bin/hyprlayer"
export PATH="$OUT/bin:$PATH"

# --- fixture -------------------------------------------------------------------------------
cd "$FX"
git init -q && git config user.email e2e@example.invalid && git config user.name e2e
mkdir -p tests scripts "$(dirname "$PLAN_REL")" .claude/agents
cp "$WT"/assets/claude/agents/*.md .claude/agents/   # project agents shadow the installed ones
printf '.claude/\ntarget/\n' > .gitignore

cat > greet.sh <<'EOF'
#!/usr/bin/env bash
name="${1:-world}"
echo "Hello, $name!"
EOF
cat > tests/test.sh <<'EOF'
#!/usr/bin/env bash
set -e
[ "$(./greet.sh bob)" = "Hello, bob!" ]
echo "tests ok"
EOF
printf 'check:\n\tbash -n greet.sh\ntest:\n\tbash tests/test.sh\n' > Makefile
# Exists but cannot run here: the agent must record the failure, not tick past it.
cat > scripts/term-capture.sh <<'EOF'
#!/usr/bin/env bash
echo "term-capture: no display available (TERM_CAPTURE_DISPLAY unset)" >&2
exit 1
EOF
chmod +x greet.sh tests/test.sh scripts/term-capture.sh

cat > "$PLAN_REL" <<'EOF'
---
title: greet.sh --shout and --times
date: 2026-11-18
type: plan
status: active
---

# greet.sh --shout and --times Implementation Plan

## Overview

Add two flags to `greet.sh`.

## What We're NOT Doing

- No other flags, no argument-parsing library.

## Phase 1: --shout

### Changes Required:

- `greet.sh`: when the first argument is `--shout`, print the whole greeting uppercased.
- `tests/test.sh`: add a `--shout` case.

### Success Criteria:

#### Automated Verification:
- [ ] Syntax check passes: `make check`
- [ ] Tests pass: `make test`

#### Manual Verification:
- [ ] `./greet.sh --shout bob` prints `HELLO, BOB!` — via running it in a shell and reading stdout
- [ ] The shouted greeting reads friendly rather than hostile (human): copy tone is a judgment call
- [ ] The shouted greeting renders in a real terminal at 80 columns — via `scripts/term-capture.sh ./greet.sh --shout bob`, which saves a capture to `target/e2e/capture.txt`

## Phase 2: --times

### Changes Required:

- `greet.sh`: `--times N <name>` prints the greeting N times.
- `tests/test.sh`: add a `--times` case.

### Success Criteria:

#### Automated Verification:
- [ ] Syntax check passes: `make check`
- [ ] Tests pass: `make test`

#### Manual Verification:
- [ ] `./greet.sh --times 3 bob` prints exactly three lines — via running it and counting stdout lines
EOF
git add -A && git commit -qm "base"

# --- helpers -------------------------------------------------------------------------------
FAILS=0
pass() { echo "PASS  $1" | tee -a "$OUT/results.txt"; }
fail() { echo "FAIL  $1" | tee -a "$OUT/results.txt"; FAILS=$((FAILS + 1)); }
check() { if eval "$2"; then pass "$1"; else fail "$1"; fi; }

# line <substring> -> the plan line holding it
line() { grep -F -- "$1" "$PLAN" | head -1; }
ticked() { line "$1" | grep -q '^- \[x\]'; }
open() { line "$1" | grep -q '^- \[ \]'; }
has() { line "$1" | grep -qF -- "$2"; }
status() { sed -n 's/^status: *//p' "$PLAN" | head -1; }
# evidence_shows <item> <expected>: the verified comment's value, or the file it names, holds <expected>
evidence_shows() {
  local ev; ev=$(line "$1" | sed -n 's/.*<!-- verified: *\(.*[^ ]\) *-->.*/\1/p')
  [ -n "$ev" ] || return 1
  case "$ev" in *"$2"*) return 0 ;; esac
  local p=${ev%% *}; [ -s "$p" ] && grep -qF -- "$2" "$p"
}

run_claude() { # <name> <model|""> <prompt> [extra args...]
  local name=$1 model=$2 prompt=$3; shift 3
  local args=(-p "$prompt" --output-format stream-json --verbose
    --permission-mode bypassPermissions --setting-sources project
    --add-dir "$WT/assets" --max-budget-usd "$BUDGET")
  [ -n "$model" ] && args+=(--model "$model")
  (cd "$FX" && claude "${args[@]}" "$@") > "$OUT/$name.jsonl" 2> "$OUT/$name.stderr" || true
  jq -r 'select(.type=="result") | .result // empty' "$OUT/$name.jsonl" > "$OUT/$name.final.md"
  cp "$PLAN" "$OUT/plan.after-$name.md"
}

skill_prompt() { # <skill> <invocation>
  cat <<EOF
Read $WT/assets/claude/skills/$1/SKILL.md in full and follow it exactly, as if the user had
invoked \`$2\`. That file is the version under test: do not invoke an installed $1 skill.
The files it loads live in $WT/assets/claude/skills/_thoughts/. This fixture has no thoughts
backend: the plan is a plain local file, and \`hyprlayer thoughts sync\` is not needed.
EOF
}

SHOUT='`./greet.sh --shout bob`'
TONE='reads friendly rather than hostile'
TERM80='renders in a real terminal at 80 columns'
TIMES='`./greet.sh --times 3 bob`'

# --- A: implement all phases ---------------------------------------------------------------
run_claude implement sonnet "$(skill_prompt implement_plan "/implement_plan $PLAN_REL implement all phases")"
SESSION=$(jq -r 'select(.type=="system" and .subtype=="init") | .session_id' "$OUT/implement.jsonl" | head -1)

check "A: runnable manual item ticked" "ticked '$SHOUT'"
check "A: ticked item carries verified evidence" "has '$SHOUT' '<!-- verified:'"
check "A: evidence shows HELLO, BOB" "evidence_shows '$SHOUT' 'HELLO, BOB'"
check "A: (human) item left open" "open '$TONE'"
check "A: failing-tool item left open" "open '$TERM80'"
check "A: failing-tool item records why" "has '$TERM80' '<!-- unverified:'"
check "A: phase 2 manual item ticked with evidence" "ticked '$TIMES' && has '$TIMES' '<!-- verified:'"
check "A: status left active" "[ \"\$(status)\" = active ]"
check "A: final notice asks about the (human) item" "grep -qiE 'friendly|tone' '$OUT/implement.final.md'"
check "A: final notice asks about the failed item" "grep -qiE 'term-capture|80 col' '$OUT/implement.final.md'"
# Phase 1's manual tick must land before any phase 2 work starts.
check "A: phase 1 manual run precedes phase 2 work" "python3 -I - '$OUT/implement.jsonl' '$PLAN' <<'PY'
import json, sys
tick = work = None
for i, raw in enumerate(open(sys.argv[1])):
    ev = json.loads(raw)
    for c in (ev.get('message') or {}).get('content') or []:
        if not isinstance(c, dict) or c.get('type') != 'tool_use':
            continue
        s = json.dumps(c.get('input'))
        if tick is None and 'HELLO, BOB' in s and 'verified:' in s:
            tick = i
        is_plan = sys.argv[2].split('/')[-1] in s and c['name'] != 'Agent'
        if work is None and not is_plan and c['name'] in ('Edit', 'Write', 'MultiEdit', 'Agent', 'Bash') and ('times' in s.lower() or (c['name'] == 'Agent' and 'phase 2' in s.lower())):
            work = i
print(f'tick event {tick}, first phase-2 work event {work}', file=sys.stderr)
sys.exit(0 if tick is not None and (work is None or tick < work) else 1)
PY"

git add -A && git commit -qm "implement" || true

# --- V1: validate with (human) and failed items open -> block -------------------------------
run_claude validate1 "" "$(skill_prompt validate_plan "/validate_plan $PLAN_REL")"
check "V1: status stays active" "[ \"\$(status)\" = active ]"
check "V1: report names the open (human) item" "grep -qiE 'friendly|tone' '$OUT/validate1.final.md'"
check "V1: report names the failed item" "grep -qiE 'term-capture|80 col' '$OUT/validate1.final.md'"

# --- B: human confirms the remaining items in the implement session -------------------------
run_claude confirm sonnet "I ran both remaining manual items by hand: the shouted greeting reads friendly, and it renders fine in my 80-column terminal. Go ahead." --resume "$SESSION"
check "B: (human) item ticked with confirmed comment" "ticked '$TONE' && has '$TONE' '<!-- confirmed:'"
check "B: failed item ticked with confirmed comment" "ticked '$TERM80' && has '$TERM80' '<!-- confirmed:'"
check "B: status still active" "[ \"\$(status)\" = active ]"
git add -A && git commit -qm "manual confirmed" || true

# --- V2: everything evidenced or confirmed -> promote ---------------------------------------
run_claude validate2 "" "$(skill_prompt validate_plan "/validate_plan $PLAN_REL")"
check "V2: status promoted to implemented" "[ \"\$(status)\" = implemented ]"

# --- V3: planted bad ticks -> block --------------------------------------------------------
python3 -I - "$PLAN" <<'PY'
import re, sys
p = sys.argv[1]; t = open(p).read()
t = re.sub(r'^status: .*$', 'status: active', t, count=1, flags=re.M)
out = []
for l in t.splitlines():
    if '`./greet.sh --shout bob`' in l:
        l = re.sub(r'\s*<!--.*?-->', '', l)                              # tick, no evidence
    elif '`./greet.sh --times 3 bob`' in l:
        l = re.sub(r'\s*<!--.*?-->', '', l) + ' <!-- verified: target/e2e/missing-capture.png -->'
    out.append(l)
open(p, 'w').write('\n'.join(out) + '\n')
PY
cp "$PLAN" "$OUT/plan.before-validate3.md"
git add -A && git commit -qm "plant bad ticks"
run_claude validate3 "" "$(skill_prompt validate_plan "/validate_plan $PLAN_REL")"
check "V3: status stays active" "[ \"\$(status)\" = active ]"
check "V3: report flags the evidence-less tick" "grep -qiE 'HELLO, BOB|--shout bob' '$OUT/validate3.final.md'"
check "V3: report flags the missing evidence path" "grep -qiE 'missing-capture|--times 3' '$OUT/validate3.final.md'"

echo "---" | tee -a "$OUT/results.txt"
echo "$FAILS failure(s); artifacts in $OUT" | tee -a "$OUT/results.txt"
exit $(( FAILS > 0 ))
