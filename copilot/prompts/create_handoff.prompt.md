---
description: Create handoff document for transferring work to another session
agent: agent
---

# Create Handoff

You are tasked with writing a handoff document to hand off your work to another agent in a new session. You will create a handoff document that is thorough, but also **concise**. The goal is to compact and summarize your context without losing any of the key details of what you're working on.

## Storage backend dispatch

Before you start, run `hyprlayer storage info --json` and parse the output. The `backend` field tells you where to save the handoff. The `schema` field lists required metadata — **populate every required field** regardless of backend. If the `hyprlayer` binary is not available or the project isn't mapped, proceed with the `git` branch using relative `thoughts/shared/handoffs/...` paths.

### Where to save

- **`git`**: write to `thoughts/shared/handoffs/ENG-XXXX/<title>.md` (or `thoughts/shared/handoffs/general/<title>.md` if no ticket) via the symlink. Prepend the required metadata as YAML frontmatter. At the end, run `hyprlayer thoughts sync` so the handoff is available to the resuming session.
- **`obsidian`**: the project's `thoughts/` symlinks point into the user's vault. `thoughts/shared/handoffs/ENG-XXXX/<title>.md` works for writes. Prepend YAML frontmatter — Obsidian's Properties panel picks it up. Do NOT run sync.
- **`notion`**: do NOT write local files. Ensure the target database exists (retrieve-database → create-database if missing → persist with `hyprlayer storage set-database-id`), then create a row via `mcp__notion__create-page`, populating every required schema field as a typed property; the handoff narrative becomes the body. If the Notion MCP tools are not available, tell the user to run `hyprlayer thoughts init --backend notion` and stop.
- **`anytype`**: do NOT write local files. Ensure the target type exists (get-type → create-type + create-property if missing → persist with `hyprlayer storage set-type-id`), then create an object via `mcp__anytype__API-create-object`, populating every required schema field. If the Anytype MCP tools are not available, tell the user to start the Anytype app and run `hyprlayer thoughts init --backend anytype`, then stop.

### Required metadata

Populate every `required: true` field from `storage info`'s `schema` array. For this command: `type: handoff`, `status: active`, `project: <mappedName>`, `scope: shared`, `date: YYYY-MM-DD`, `author` from `hyprlayer thoughts config --json`, `ticket` if referenced, 2-5 `tags`, and a `title` like `"ENG-XXXX: short description"`. Legal `select` values are in `schema.options`. Render as YAML frontmatter for `git`/`obsidian`; typed properties for `notion`/`anytype`.

## Process
### 1. Filepath & Metadata
Use the following information to understand how to create your document:
    - create your file under `thoughts/shared/handoffs/ENG-XXXX/YYYY-MM-DD_HH-MM-SS_ENG-ZZZZ_description.md`, where:
        - YYYY-MM-DD is today's date
        - HH-MM-SS is the hours, minutes and seconds based on the current time, in 24-hour format (i.e. use `13:00` for `1:00 pm`)
        - ENG-XXXX is the ticket number (replace with `general` if no ticket)
        - ENG-ZZZZ is the ticket number (omit if no ticket)
        - description is a brief kebab-case description
    - Run the `scripts/spec_metadata.sh` script to generate all relevant metadata
    - Examples:
        - With ticket: `2025-01-08_13-55-22_ENG-2166_create-context-compaction.md`
        - Without ticket: `2025-01-08_13-55-22_create-context-compaction.md`

### 2. Handoff writing.
using the above conventions, write your document. use the defined filepath, and the following YAML frontmatter pattern. Use the metadata gathered in step 1, Structure the document with YAML frontmatter followed by content:

Use the following template structure:
```markdown
---
date: [Current date and time with timezone in ISO format]
researcher: [Researcher name from thoughts status]
git_commit: [Current commit hash]
branch: [Current branch name]
repository: [Repository name]
topic: "[Feature/Task Name] Implementation Strategy"
tags: [implementation, strategy, relevant-component-names]
status: complete
last_updated: [Current date in YYYY-MM-DD format]
last_updated_by: [Researcher name]
type: implementation_strategy
---

# Handoff: ENG-XXXX {very concise description}

## Task(s)
{description of the task(s) that you were working on, along with the status of each (completed, work in progress, planned/discussed). If you are working on an implementation plan, make sure to call out which phase you are on. Make sure to reference the plan document and/or research document(s) you are working from that were provided to you at the beginning of the session, if applicable.}

## Critical References
{List any critical specification documents, architectural decisions, or design docs that must be followed. Include only 2-3 most important file paths. Leave blank if none.}

## Recent changes
{describe recent changes made to the codebase that you made in line:file syntax}

## Learnings
{describe important things that you learned - e.g. patterns, root causes of bugs, or other important pieces of information someone that is picking up your work after you should know. consider listing explicit file paths.}

## Artifacts
{ an exhaustive list of artifacts you produced or updated as filepaths and/or file:line references - e.g. paths to feature documents, implementation plans, etc that should be read in order to resume your work.}

## Action Items & Next Steps
{ a list of action items and next steps for the next agent to accomplish based on your tasks and their statuses}

## Other Notes
{ other notes, references, or useful information - e.g. where relevant sections of the codebase are, where relevant documents are, or other important things you leanrned that you want to pass on but that don't fall into the above categories}
```
---

### 3. Approve and Sync
For `backend: git`, run `hyprlayer thoughts sync` so the handoff is pushed. For `obsidian`/`notion`/`anytype`, skip the sync.

Once this is completed, you should respond to the user with the template between <template_response></template_response> XML tags. do NOT include the tags in your response. The "path" field should be the local filepath for git/obsidian or the page/object ID for notion/anytype.

<template_response>
Handoff created! You can resume from this handoff in a new session with the following command:

```bash
/resume_handoff <path or id>
```
</template_response>

for example (between <example_response></example_response> XML tags - do NOT include these tags in your actual response to the user)

<example_response>
Handoff created! You can resume from this handoff in a new session with the following command:

```bash
/resume_handoff thoughts/shared/handoffs/ENG-2166/2025-01-08_13-44-55_ENG-2166_create-context-compaction.md
```
</example_response>

---
##.  Additional Notes & Instructions
- **more information, not less**. This is a guideline that defines the minimum of what a handoff should be. Always feel free to include more information if necessary.
- **be thorough and precise**. include both top-level objectives, and lower-level details as necessary.
- **avoid excessive code snippets**. While a brief snippet to describe some key change is important, avoid large code blocks or diffs; do not include one unless it's necessary (e.g. pertains to an error you're debugging). Prefer using `/path/to/file.ext:line` references that an agent can follow later when it's ready, e.g. `packages/dashboard/src/app/dashboard/page.tsx:12-24`
