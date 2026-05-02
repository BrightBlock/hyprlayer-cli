---
name: create_handoff
description: Create a handoff document for transferring work to another session. Use when the user asks to create a handoff, summarize the session for a future agent, or capture context for a clean restart. Produces a thoughts artifact (a handoff).
allowed-tools: Bash, Read, Write, Edit, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# Create Handoff

You are tasked with writing a handoff document to hand off your work to another agent in a new session. You will create a handoff document that is thorough, but also **concise**. The goal is to compact and summarize your context without losing any of the key details of what you're working on.

## Storage backend dispatch

Read `~/.claude/skills/_thoughts/storage-backend.md` and follow it for where to save the artifact. Read `~/.claude/skills/_thoughts/required-metadata.md` for the schema-required fields and the backend-specific title format. For this command: artifact type is `handoff`; status is `active` (the handoff is actionable); the title is `ENG-XXXX: short description` (or `general: short description` when there is no ticket).

The `type: implementation_strategy` field used by the existing handoff template is domain-specific and continues to appear in the body/frontmatter as today — it supplements the schema-level `type: handoff`.

## Process
### 1. Filepath & Metadata
Use the following information to understand how to create your document:
    - For `git`/`obsidian`, create your file under `thoughts/shared/handoffs/ENG-XXXX/YYYY-MM-DD_HH-MM-SS_ENG-ZZZZ_description.md`, where:
        - YYYY-MM-DD is today's date
        - HH-MM-SS is the hours, minutes and seconds based on the current time, in 24-hour format (i.e. use `13:00` for `1:00 pm`)
        - ENG-XXXX is the ticket number (replace with `general` if no ticket)
        - ENG-ZZZZ is the ticket number (omit if no ticket)
        - description is a brief kebab-case description
    - For `notion`/`anytype`, use the human-readable title format from `required-metadata.md` and let the storage backend assign IDs.
    - Examples:
        - With ticket: `2025-01-08_13-55-22_ENG-2166_create-context-compaction.md`
        - Without ticket: `2025-01-08_13-55-22_create-context-compaction.md`
    - Collect metadata directly: current date/time (ISO with timezone), git commit hash (`git rev-parse HEAD`), branch (`git branch --show-current`), repository name, researcher name (from `hyprlayer thoughts config --json` or `git config user.name`).

### 2. Handoff writing.
Read `~/.claude/skills/_thoughts/templates/handoff.md` for the body template. Populate every placeholder using the metadata from step 1, then save the result to the destination resolved by the storage backend dispatch.

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
