# Storage backend dispatch

This is the shared protocol for skills that produce thoughts artifacts (research docs, plans, handoffs, validation reports). The skill that loaded this file already knows which artifact `type` and `<title>` to use.

> **Path convention**: the `thoughts/shared/...` paths in examples and templates below are literal on `git`/`obsidian` backends. On `notion`/`anytype`, substitute the matching `notion://<id>` / `anytype://<id>` identifier that `hyprlayer storage info` or `thoughts-locator` returns.

Before you start, run `hyprlayer storage info --json` and parse the output. The `backend` field tells you where to save the artifact. The `schema` field lists the metadata properties — see `required-metadata.md` for which to populate. If the `hyprlayer` binary is not available or the project isn't mapped, proceed with the `git` branch using relative `thoughts/shared/<type>s/...` paths.

## Where to save

- **`git`**: write local markdown files through the project's `thoughts/shared/...` symlinks exactly as today. Prepend the required metadata as YAML frontmatter (see `required-metadata.md`). `settings.thoughtsRepo` gives the absolute path. At the end, remind the user to run `hyprlayer thoughts sync` so the artifact is pushed.
- **`obsidian`**: the project's `thoughts/` symlinks are still present and point into the user's vault, so relative paths like `thoughts/shared/<type>s/<file>.md` continue to work for writes. Prepend the required metadata as YAML frontmatter — Obsidian's Properties panel picks it up automatically. For absolute on-disk paths, use `settings.contentRoot` + `settings.reposDir` + `mappedName`. Do NOT remind the user to sync — Obsidian has no sync step.
- **`notion`**: do NOT write local files. Ensure the target database exists:
  1. If `settings.databaseId` is populated, call `mcp__notion__retrieve-database` with that ID. If it resolves, skip to step 4.
  2. If `databaseId` is missing or retrieval returns not-found, call `mcp__notion__create-database` under `settings.parentPageId` with `title: "Hyprlayer Thoughts"` and one property per entry in `storage info`'s `schema` array (title → `title`; text → `rich_text`; date → `date`; select → `select` with `options`; tags → `multi_select`; relation → `relation` self-referential).
  3. Run `hyprlayer storage set-database-id <returned_id>` to persist. Proceed with step 4 using the new ID.
  4. Create a database row using `mcp__notion__create-page` with `parent.database_id = <id>`. Populate every required schema field as a typed property; the body receives the narrative content.
  If the Notion MCP tools are not available in this session, tell the user to run `hyprlayer thoughts init --backend notion` and stop.
- **`anytype`**: do NOT write local files. Ensure the target type + properties + tags exist before creating the object:
  1. **Resolve the type.** If `settings.typeId` is populated, call `mcp__anytype__API-get-type` with that ID + `settings.spaceId`. If it resolves, skip to step 4. If it returns not-found / 404 / 410, treat as missing and fall through.
  2. **Create the type + properties.** Call `mcp__anytype__API-create-type` in `spaceId` with `name: "Hyprlayer Thought"`, `plural_name: "Hyprlayer Thoughts"`, `key: "hyprlayer_thought"`, `layout: "basic"`. Anytype's `properties` array on this endpoint does NOT reliably attach all supplied properties to the type — create them explicitly via `mcp__anytype__API-create-property`, one call per field in `schema` (except `title`, which maps to the object's `name` field and does not need a property). Use `key: "hyprlayer_<field>"` (Anytype snake-cases keys automatically). For each schema field whose `kind` is `select` or `multi_select`/`tags`, pass `tags: [...]` in the create-property call with one entry per `schema.options` value — this bakes the legal tag set in up-front so future writes don't need to create tags inline. If a property key already exists (`bad request: property key "…" already exists`), treat that as success and continue — a prior invocation created it at space scope. **Then call `mcp__anytype__API-update-type`** on the newly-created type with `properties: [...]` listing every field (`{key, name, format}`) — this is what actually links the properties to the type so the UI renders them on every object. Without the update-type step, object creates silently accept typed values but Anytype's property sidebar shows only the built-in `tag`/`backlinks` entries.
  3. **Persist the type ID.** Run `hyprlayer storage set-type-id <returned_id>`. Proceed with step 4 using the new ID.
  4. **Ensure select tags exist** for the specific values this write uses. For `type`, `status`, `scope`, and each `tags` value you are about to set, call `mcp__anytype__API-list-tags` (filter by the matching `property_id`) and call `mcp__anytype__API-create-tag` for any values not yet present. Record the returned tag IDs — the object-create call takes tag IDs, not string names. Anytype snake-cases tag keys (e.g. `integration-test` → `integration_test`); the `name` is preserved verbatim, so filter / display by name.
  5. **Create the object.** Call `mcp__anytype__API-create-object` with `type_key: "hyprlayer_thought"`, `space_id: <spaceId>`, `name: <title>`, `body: <narrative markdown>`, and a `properties` array — one entry per required schema field, using the property `key` (e.g. `hyprlayer_type`) and the matching typed value (`select: <tag_id>`, `multi_select: [<tag_id>, ...]`, `date: "YYYY-MM-DD"`, `text: "..."`). Do NOT dump metadata into the body as frontmatter — Anytype's search relies on typed properties.
  If the Anytype MCP tools are not available, tell the user to start the Anytype app and run `hyprlayer thoughts init --backend anytype`, then stop. Do NOT silently fall back to writing a local markdown file — that would hide the misconfiguration.

## How to read existing artifacts

For skills that retrieve an artifact (e.g. `validate_plan`, `resume_handoff`, `implement_plan`, `iterate_plan`):

- **`git`** / **`obsidian`**: read the markdown file through the project's `thoughts/shared/<type>s/<name>.md` symlink (or the absolute path under `settings.thoughtsRepo` / `settings.contentRoot`). For `git`, you may first run `hyprlayer thoughts sync` to pull the latest. For `obsidian`, skip the sync. If the user passes only a ticket number, list `thoughts/shared/<type>s/ENG-XXXX/` (handoffs use ticket-scoped subdirs with `YYYY-MM-DD_HH-MM-SS` filename prefixes — pick the most recent).
- **`notion`**: call `mcp__notion__retrieve-page` with the page ID the user provides. To search by title or property, query the database (the data source URL appears in `<data-source>` tags from `mcp__notion__notion-fetch` on the database) filtered by `type = <artifact_type>` + `project = <mappedName>` (and `ticket = ENG-XXXX` for handoffs); sort by `date` descending to find the most recent.
- **`anytype`**: call `mcp__anytype__API-get-object` with the object ID + `settings.spaceId`. To search, call `mcp__anytype__API-list-objects` filtered by type + property (`type = <artifact_type>`); sort by `date` descending.

## How to update existing artifacts

For skills that mutate an artifact (e.g. `implement_plan` checking off items and promoting `status`, `iterate_plan` rewriting sections):

- **`git`** / **`obsidian`**: edit the file directly with the Edit tool. Update YAML frontmatter fields (e.g. `status`, `last_updated`, `last_updated_by`) when the schema supports them. For `git`, run `hyprlayer thoughts sync` at the end. For `obsidian`, skip the sync.
- **`notion`**: update body content via `mcp__notion__notion-update-page` with `command: "update_content"` (search-and-replace) for surgical edits, or `command: "replace_content"` for wholesale rewrites. Update typed properties (e.g. `status`, `last_updated`) via `command: "update_properties"`. Use only `select` values from `schema.options` — do not invent new ones.
- **`anytype`**: update via `mcp__anytype__API-update-object`, passing the new `body` and a `properties` array for changed fields. Tag values must be passed as tag IDs (call `mcp__anytype__API-list-tags` to resolve names → IDs first).

Do not drop required schema fields during edits. Promote `status` per the rules in `required-metadata.md` (`draft` → `active` → `implemented` for plans; the active skill names what its mutation should be).
