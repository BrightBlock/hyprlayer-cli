# Research artifact template

Skills that document the codebase use this body structure. Populate every `[placeholder]`. Omit any section that doesn't apply to the variant you're running (e.g., `Historical Context` is `git`/`obsidian`-only with a thoughts directory).

For `git`/`obsidian` backends, prepend the YAML frontmatter shown below. For `notion`/`anytype`, the schema fields ride as typed properties (per `_thoughts/required-metadata.md`) — do NOT duplicate them in the body.

```markdown
---
date: [Current date and time with timezone in ISO format]
researcher: [Researcher name]
git_commit: [Current commit hash]
branch: [Current branch name]
repository: [Repository name]
topic: "[User's Question/Topic]"
tags: [research, codebase, relevant-component-names]
status: complete
last_updated: [Current date in YYYY-MM-DD format]
last_updated_by: [Researcher name]
---

# Research: [User's Question/Topic]

**Git Commit**: [Current commit hash]
**Branch**: [Current branch name]

## Research Question
[Original user query]

## Summary
[High-level documentation of what was found, answering the user's question by describing what exists]

## Detailed Findings

### [Component/Area 1]
- Description of what exists ([file.ext:line](link))
- How it connects to other components
- Current implementation details

### [Component/Area 2]
...

## Code References
- `path/to/file.py:123` - Description of what's there
- `another/file.ts:45-67` - Description of the code block

## Architecture Documentation
[Current patterns, conventions, and design implementations found in the codebase]

## Historical Context (from thoughts/)
[Only for skills that use the thoughts directory. Reference items as `thoughts/shared/something.md` — historical decision about X. Paths exclude "searchable/" even if found there.]

## Related Research
[Links to other research documents in thoughts/shared/research/]

## Open Questions
[Any areas that need further investigation]
```

## Frontmatter rules

- Use snake_case for multi-word fields (`last_updated`, `git_commit`).
- Tags should be relevant to the topic and components studied.
- Update `last_updated`, `last_updated_by`, and add `last_updated_note: "..."` when appending follow-up research; add a `## Follow-up Research [timestamp]` section.
- For `notion`/`anytype`: update typed properties via the relevant MCP call rather than editing frontmatter.
