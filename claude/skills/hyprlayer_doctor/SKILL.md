---
name: hyprlayer_doctor
description: Verify the configured hyprlayer backend (git, obsidian, notion, or anytype) is ready before operations run against it. Use on first backend-touching call per session, on user request ("verify hyprlayer setup", "/hyprlayer_doctor"), and after any auth/404/schema error from a backend. Cheap (~1–15s depending on backend) and cached against the config-file content (auto-invalidates on any config change).
allowed-tools: Bash, Read, Write, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# hyprlayer_doctor

You are tasked with verifying that the user's hyprlayer environment is ready for the configured backend. Backend-agnostic concerns (config resolution, content-hash cache, per-repo mapping, dispatch) are handled here; backend-specific checks live in `./backends/<backend>.md`.

## When to run

- **First backend-touching call per session** — read the cache (see step 2). If valid, skip; otherwise run.
- **On user request** — `/hyprlayer_doctor`, "verify hyprlayer setup", "is my notion/anytype/obsidian/git backend connected".
- **On unexpected failure** — any 401, 404, schema-mismatch, lock, or permission error from a backend operation. Run once. Do not loop.

## Procedure

### 1. Resolve config

Hyprlayer's config lives at the OS config-dir (`dirs::config_dir()` in Rust):
- macOS: `~/Library/Application Support/hyprlayer/config.json`
- Linux: `${XDG_CONFIG_HOME:-~/.config}/hyprlayer/config.json`
- Windows: `%APPDATA%\hyprlayer\config.json`

Steps:
1. Read the file. If missing → ❌ `Config not found at <path>. Run \`hyprlayer thoughts init\` to create one.`
2. Compute a content hash for caching: `shasum -a 256 <config> | awk '{print substr($1,1,16)}'` (Linux: substitute `sha256sum`). Keep this hash — step 2 uses it as the cache-file name.
3. Parse the file as JSON. If malformed → ❌ `Config at <path> is not valid JSON: <error>.`
4. If `.thoughts` is missing or null → ❌ `Thoughts not configured (AI may be configured but \`hyprlayer thoughts init\` was never run).`
5. Resolve the active backend for the current repo:
   - Get `$PWD` (or the explicitly-passed repo path).
   - If `.thoughts.repoMappings[$PWD]` exists and is an object with a non-null `.profile`, use `.thoughts.profiles[<profile>].backend` — otherwise use `.thoughts.backend`. (The mapping can be either `"name"` or `{"repo": "name", "profile": "name-or-null"}`; only the object form carries a profile.)
   - The resolved backend has `kind` ∈ {`git`, `obsidian`, `notion`, `anytype`} (lowercase, per `BackendKind` serde rename). Any other value → ❌ with the actual value shown.
6. Identify the agent tool from `.ai.agentTool` if present — needed by the anytype procedure for the `mcp list` probe.

### 2. Cache check

The cache key is a hash of the config-file content, **not** a session identifier. `$SESSION_ID` is not reliably set in the Claude Code harness, and falling back to a constant (`default`) makes the cache global across sessions — that defeats the per-session intent and silently hides drift. Hashing the config gives us the actually-meaningful signal: the cache invalidates automatically whenever the user runs `hyprlayer thoughts init`, switches backends, changes a page ID, or edits the file any other way.

Steps:
1. Resolve the OS temp directory portably: `TMP="${TMPDIR:-${TEMP:-/tmp}}"`. macOS exports `$TMPDIR` (`/var/folders/...`); Linux usually has neither set and falls through to `/tmp`; Windows git-bash and WSL expose `$TEMP` (`/c/Users/<user>/AppData/Local/Temp`). Hard-coding `/tmp` would silently fail on Windows where the path doesn't exist.
2. Cache file: `$TMP/hyprlayer-doctor-<config-hash>.ok` (using the 16-char hash from step 1).
3. If the file exists AND its mtime is < 4 hours old → emit `✅ Already verified for this config (cached <human-readable-ago>)` and exit successfully.
4. If the file exists but mtime is >= 4 hours old → continue; the stale sentinel will be overwritten on pass.
5. Otherwise → continue to step 3.

Why the 4h TTL on top of content-hashing: auth tokens / connector state can drift externally even when the config is untouched. Hash-only would let a disconnected Notion connector remain "verified" indefinitely.

### 3. Dispatch

Read `./backends/<kind>.md` from this skill's directory and execute its procedure top-to-bottom. The file is a checklist, not a skill — it has no frontmatter and runs inline within this skill's tool grants.

Pass to the procedure: the resolved config path, the parsed backend object (just the matching variant — `GitConfig`, `ObsidianConfig`, `NotionConfig`, or `AnytypeConfig`), the resolved `.thoughts.user`, the resolved `mapped_name` (from `repoMappings[$PWD].repo` or its string form, or `null` if unmapped), and the resolved `agent_tool`.

### 4. Report

Each backend procedure emits per-step ✅/⏭/❌ lines. After it completes, aggregate into one consolidated report:

```
hyprlayer_doctor (backend: notion, repo: brightblock/hyprlayer-cli):
  ✅ MCP connector: Claude.ai Notion connector reachable
  ✅ Auth: 2.8s
  ✅ Parent page: <Page Title>
  ❌ Schema: Tasks.Status expected select, found status
  ⏭  Write permission: skipped (prior step failed)

  Status: FAIL (schema). Halt before running hyprlayer commands.
```

### 5. Cache on full pass

If — and only if — every step passed (❌ count is zero; ⏭ allowed only when explicitly marked optional or N/A by the procedure), write the sentinel using the config hash from step 1 and the portable `$TMP` resolved in step 2:

```bash
touch "${TMPDIR:-${TEMP:-/tmp}}/hyprlayer-doctor-<config-hash>.ok"
```

On any failure, do not write the sentinel; the next backend-touching call re-triggers the doctor.

## Backend procedures

| Backend | File | Notes |
|---|---|---|
| git (default) | `backends/git.md` | Local filesystem + git checkout; sync is real (commit / pull --rebase / push) |
| obsidian | `backends/obsidian.md` | Local filesystem under an Obsidian vault; sync is a no-op |
| notion | `backends/notion.md` | Agent-tool connector; hyprlayer does **not** register an MCP server |
| anytype | `backends/anytype.md` | Hyprlayer registers `npx -y @any-org/anytype-mcp` as an MCP server |

To add a 5th backend later: add a `BackendKind` variant in `src/config.rs`, define its `*Config` struct, then create `backends/<newkind>.md` here. The dispatcher needs no change.

## Do not

- Do not register as a SessionStart hook. The cost is wrong for sessions that never touch a hyprlayer backend.
- Do not run multiple backends' procedures in one pass. Hyprlayer has exactly one active backend per (repo, profile) — resolve, then route.
- Do not retry write checks more than once per run. A failure is a failure; report it.
- Do not invoke this skill recursively from a backend procedure.
- Do not assume `database_id` (notion) or `type_id` (anytype) are populated — both are optional and auto-created on first write. Skip dependent checks rather than failing.
- Do not key the cache on `$SESSION_ID` or any session-scoped identifier; the Claude Code harness does not reliably expose one, and a constant fallback (`default`) hides config drift across sessions.
