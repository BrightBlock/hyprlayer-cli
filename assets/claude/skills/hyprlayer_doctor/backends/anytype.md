# Backend procedure: anytype

Anytype is MCP-backed, but unlike Notion, hyprlayer **does** register the MCP server itself with both available base CLIs (`claude mcp add` and `codex mcp add`, per `src/backends/anytype.rs`). The MCP server is `npx -y @any-org/anytype-mcp`, registered under the name `anytype`. Total budget: ~5–10s.

Inputs from the dispatcher: `AnytypeConfig { spaceId, typeId?, apiTokenEnv? }` and the resolved `user`.

## 1. MCP server registered

- Run both `claude mcp list` and `codex mcp list` when those CLIs are available, looking for the literal name `anytype` in each output.
- ❌ if neither available CLI has it registered. Remediation: `hyprlayer thoughts init --force` reconciles both registrations best-effort.
- Keep the probe result; step 2 needs it.

## 2. API token env var

- The env var name is `AnytypeConfig.apiTokenEnv` if set, otherwise the default `ANYTYPE_API_KEY` (constant `DEFAULT_ANYTYPE_TOKEN_ENV` in `src/backends/anytype.rs`).
- **If step 1 says the MCP server is already registered** → ⏭ `Token baked into MCP registration; runtime export not required.` Rationale: `resolve_mcp_env_pair` (`src/backends/common.rs`) expands the env var at `mcp add` time and stores the literal `KEY=value` in the registration, so the agent's spawned MCP server gets the token even when the shell no longer exports it. Validity of the stored token is verified end-to-end in step 4 — don't double-check it here.
- **If step 1 says the MCP server is not yet registered** → ❌ when the env var is unset, since the upcoming `hyprlayer thoughts init` re-registration would fail to capture a value.
- Remediation (unregistered + unset case): `Settings → API Keys` in Anytype → issue a key → `export <ENV_VAR>=<key>`, then re-run `hyprlayer thoughts init`. Note: the agent must have been *launched* with the var set; setting it in a child shell after the agent started won't help.

## 3. Anytype desktop app reachable

- The `@any-org/anytype-mcp` server talks to the local Anytype desktop app over its local API. The app must be running and the active account unlocked.
- macOS probe: `pgrep -f "Anytype" >/dev/null && echo running || echo not-running`. Linux: same. Windows: `tasklist | findstr /I anytype`.
- ⚠ (warn, don't fail) if not running — the next MCP call will surface the real error with a clearer message.
- Remediation: launch Anytype.app and unlock the active account.

## 4. Auth / MCP call works end-to-end

- Make a minimal read call against the `mcp__anytype__*` namespace. The exact tool depends on the installed `@any-org/anytype-mcp` surface; consult the registered tool list at runtime rather than hard-coding a name here.
- ❌ on errors mentioning auth/unauthorized/locked-vault. Remediation depends on the error: re-issue the API key, unlock the account, or restart Anytype.

## 5. Space root

- From `AnytypeConfig.spaceId` (required; empty would have failed `hyprlayer thoughts init`).
- Fetch the space metadata via the Anytype MCP. ❌ on not-found — wrong space, or the API key was issued for a different identity.
- Remediation: re-run `hyprlayer thoughts init` against the correct space, or update the config field directly.

## 6. Type schema

- `AnytypeConfig.typeId` is **optional** — hyprlayer auto-creates the Type on first write. If absent → ⏭ `Schema check skipped: type will be created on first write.`
- If present: fetch the Type definition and assert each required Relation from `THOUGHT_SCHEMA` (`src/backends/schema.rs`) exists with a matching format:

  | Relation | Format |
  |---|---|
  | project, author, ticket | text |
  | type, status, scope | select (with the same option lists Notion uses) |
  | date | date |
  | tags | tags / multi-select |
  | related | relation |

- **Do not check for a `title` Relation.** In Anytype, `title` maps to the built-in object `name` and is intentionally not created as a separate property (see `claude/skills/_thoughts/storage-backend.md` step 2, "except `title`, which maps to the object's `name` field and does not need a property"). Flagging a missing `title` Relation here would FAIL valid setups produced by the documented hyprlayer write flow.
- ❌ on missing required Relations (from the table above); ⚠ on option drift in select fields.
- Remediation: edit the Type in Anytype to add/rename the Relation, or delete the Type and let hyprlayer recreate it.

## 7. Write permission

- Create a throwaway Object of the configured Type (or any Object the API allows when Type is unset) in the space, with title `__hyprlayer_doctor_<unix_ts>`. Archive/delete it immediately. Clean up unconditionally.
- ❌ on permission errors. Remediation: in Anytype, grant Editor access to the active identity for the space.

## 8. Latency baseline (optional)

Only when the dispatcher requested a full report. Sum wall-clock of steps 4–7. If > 15s, note that Anytype is responding slowly — usually a local-app issue (large vault, slow disk, background sync).
