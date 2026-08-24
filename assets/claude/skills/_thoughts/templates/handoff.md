# Handoff artifact template

`create_handoff` uses this body structure. Populate every `[placeholder]`.

For `git`/`obsidian` backends, prepend the YAML frontmatter shown below. For `notion`/`anytype`, schema fields ride as typed properties (per `_thoughts/required-metadata.md`) — do NOT duplicate them in the body. The `type: implementation_strategy` body field is domain-specific to handoffs and supplements the schema-level `type: handoff`.

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
{Description of the task(s) you were working on, along with the status of each (completed, work in progress, planned/discussed). If you are working on an implementation plan, call out which phase you are on. Reference the plan document and/or research document(s) you are working from that were provided to you at the beginning of the session, if applicable.}

## Critical References
{List any critical specification documents, architectural decisions, or design docs that must be followed. Include only 2-3 most important file paths. Leave blank if none.}

## Recent changes
{Describe recent changes made to the codebase that you made in line:file syntax}

## Learnings
{Describe important things you learned — patterns, root causes of bugs, or other important pieces of information someone picking up your work after you should know. Consider listing explicit file paths.}

## Artifacts
{An exhaustive list of artifacts you produced or updated as filepaths and/or file:line references — paths to feature documents, implementation plans, etc. that should be read in order to resume your work.}

## Action Items & Next Steps
{A list of action items and next steps for the next agent to accomplish based on your tasks and their statuses.}

## Other Notes
{Other notes, references, or useful information — where relevant sections of the codebase are, where relevant documents are, or other important things you learned that you want to pass on but that don't fall into the above categories.}
```

## Notes

- **More information, not less.** This is the minimum; include more if necessary.
- **Be thorough and precise.** Include both top-level objectives and lower-level details.
- **Avoid excessive code snippets.** Prefer `path/to/file.ext:line` references that an agent can follow later. A brief snippet is fine when describing a key change or an error you are debugging.
