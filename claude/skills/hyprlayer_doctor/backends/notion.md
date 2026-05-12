# Backend procedure: notion

Hyprlayer's Notion backend relies on the **agent tool's connector** (Claude.ai's Notion integration, surfaced as `mcp__claude_ai_Notion__notion-*`). Hyprlayer does *not* register its own Notion MCP server — `src/backends/notion.rs` is explicit about this. Total budget: ~5–15s.

Inputs from the dispatcher: `NotionConfig { parentPageId, databaseId? }`, plus the resolved `user`.

## 1. Connector reachability (in lieu of an `mcp list` probe)

- `claude mcp list` does **not** see connectors — the comment in `src/backends/notion.rs` calls this out. Don't run it; the result is misleading.
- Instead, attempt a no-op tool call: `mcp__claude_ai_Notion__notion-search` with `{query: ".", page_size: 1, content_search_mode: "workspace_search", filters: {}}`. Expected ~3s.
- ❌ if `notion-search` is not callable at all → Notion connector is not enabled in the agent.
- Remediation: in Claude.ai, Settings → Connectors → Notion → Connect.

## 2. Auth

- The probe call in step 1 doubles as the auth check. 401 / "unauthorized" / "disconnected" → the connector is enabled but unauthed.
- Remediation: reconnect Notion in claude.ai settings; re-grant workspace access.

## 3. Parent page (root)

- From `NotionConfig.parentPageId` (required field; empty would have failed `hyprlayer thoughts init`).
- `mcp__claude_ai_Notion__notion-fetch` with `{id: <parentPageId>}`. ❌ on 404 — wrong workspace, the page was deleted, or the connector identity doesn't have access.
- Remediation: re-run `hyprlayer thoughts init` against a parent page the connector identity can read, or share the existing parent with the connector.

## 4. Database schema

- `NotionConfig.databaseId` is **optional** — hyprlayer auto-creates the database on first write. If absent → ⏭ `Schema check skipped: database will be created on first write.` (this is N/A, not a failure).
- If present: `notion-fetch` the database/data source and assert it contains every required property from `THOUGHT_SCHEMA` (`src/backends/schema.rs`):

  | Property | Kind | Options |
  |---|---|---|
  | title (required) | text | — |
  | type (required) | select | `plan`, `research`, `handoff`, `note`, `pr` |
  | date (required) | date | — |
  | status (required) | select | `draft`, `active`, `implemented`, `superseded`, `archived` |
  | project (required) | text | — |
  | scope (required) | select | `user`, `shared`, `global` |
  | author (required) | text | — |
  | ticket | text | — |
  | tags | tags / multi-select | — |
  | related | relation | — |

- On mismatch, name the property and show expected vs. actual. ❌ on any required field missing or with wrong kind; ⚠ (not fail) if a Select is missing one of the expected options.
- Remediation: rename / re-type in Notion to match, or delete the database and let hyprlayer recreate it on the next write.

- From the database response, capture the **data source ID** (the `data_source_url`'s ID, looks like `53b5216f-…`). Step 5 needs it as the `data_source_id` parent.

## 5. Write permission

The probe is a **single create-with-archived-status** call. No follow-up cleanup is needed: the row is born in the archived bucket and never appears in any view that filters on `status != archived` (the hyprlayer default). This is the cleanest shape because Notion's native page-archive bit is **not** exposed by `notion-update-page` — there is no `archived: true` parameter on the update tool. `replace_content` with an empty `new_str` only blanks the page body; the row itself remains visible. Doing the archival via a second `update_properties` call works but doubles the API hops and is slower.

Steps:
1. Single call: `mcp__claude_ai_Notion__notion-create-pages` with:
   - `parent`: when `databaseId` is set, `{type: "data_source_id", data_source_id: <data_source_id from step 4>}`; otherwise `{type: "page_id", page_id: <parentPageId>}` (and skip the per-row properties block — non-database pages only accept `title`).
   - `pages`: a single-element array. For database parents:
     ```
     {
       properties: {
         title: "__hyprlayer_doctor_<unix_ts>",
         type: "note",
         "date:date:start": "<today's date YYYY-MM-DD>",
         status: "archived",
         project: "hyprlayer_doctor",
         scope: "user",
         author: "<user from config>"
       },
       content: "hyprlayer_doctor verify"
     }
     ```
   - For non-database parents: only `{properties: {title: "__hyprlayer_doctor_<unix_ts>"}, content: "hyprlayer_doctor verify"}`.
   - Do **not** pass a top-level `title` on the page object — `notion-create-pages` rejects it with an `unrecognized_keys` validation error. Title goes inside `properties` only.
2. ❌ on permission errors. Remediation: ensure the parent / data source is shared with the connector identity as **Editor** (not Viewer).
3. For database parents the row is now in `status: archived` and the test is complete — no cleanup call.
4. For non-database parents the test page will be visible under the parent. ⚠ tell the user: "Test page `<title>` was left under `<parent>`. Notion's MCP surface does not expose archive on child pages; please remove it manually if desired." Do not attempt to delete via `replace_content` — that only clears the body.

## 6. Latency baseline (optional)

Only when the dispatcher requested a full report. Sum wall-clock of steps 1, 3, 4, and 5. If > 20s, note that Notion's API is degraded right now — operations will feel slow, but it's not a setup problem.
