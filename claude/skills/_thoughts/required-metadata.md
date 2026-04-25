# Required metadata

This is the shared schema-population protocol for skills that produce thoughts artifacts. The skill that loaded this file already knows which artifact `type` (`research`, `plan`, `handoff`, etc.) and `<title>` to use.

Read the `schema` array from `hyprlayer storage info --json`. Populate **every field marked `required: true`**.

| Field | How to determine |
|---|---|
| `title` | Format depends on backend (see "Title format" below). |
| `type` | Set to the artifact type the skill names (`plan`, `research`, `handoff`, etc.). |
| `date` | Today's date in `YYYY-MM-DD`. |
| `status` | `draft` for newly-created artifacts; `active` while research is ongoing; `implemented` once a plan is fully shipped. Skills that mutate an existing artifact promote `status` per their own rules. |
| `project` | `mappedName` from `storage info --json`. |
| `scope` | `shared` unless the user's task clearly implies `user` or `global`. |
| `author` | Pull from `hyprlayer thoughts config --json` (the `user` field) or derive from `git config user.name`. |
| `ticket` | If the task references `ENG-XXXX` or similar, capture it; otherwise null. |
| `tags` | Derive 2-5 topic tags from the task. |
| `related` | Leave empty unless the task explicitly references another plan/research doc. |

For `select` fields, the `schema.options` array lists the legal values — do not invent new ones.

## Title format

The `title` format depends on the backend, because the date is part of the filename on filesystem-backed backends and a property column on database-backed backends:

- **`git`** / **`obsidian`** — use kebab-case with a date prefix: `YYYY-MM-DD-ENG-XXXX-description` (omit the ticket chunk if there is none). Example: `2025-01-08-ENG-1478-parent-child-tracking` or `2025-01-08-authentication-flow`. This is also the filename, so it must be filesystem-safe and sortable.
- **`notion`** / **`anytype`** — use a normal human-readable heading: sentence case, no date prefix, no kebab-case. The `date` rides as a typed property and is shown in the database view. Examples: `Parent-child tracking for Claude sub-tasks`, `Authentication flow`, `is_installed() in src/agents.rs`. If a ticket is associated, name it inline (e.g. `Parent-child tracking (ENG-1478)`) rather than as a leading slug — the ticket is also captured as the `ticket` property.

## How metadata is rendered

- **`git`** / **`obsidian`**: render every required field as YAML frontmatter at the top of the markdown file. Obsidian's Properties panel reads these automatically.
- **`notion`** / **`anytype`**: set every required field as a typed database/object property. Do NOT duplicate schema fields (`date`, `author`, `project`, `status`, `scope`, `type`, `tags`, `ticket`, `title`) as bold-prefix lines in the document body — they already render from the property panel.

Supplementary fields that are NOT in the data source schema (e.g. `git_commit`, `branch`, `topic`, `last_updated`, `last_updated_by`, `last_updated_note`) may go in the body when they add value, regardless of backend.
