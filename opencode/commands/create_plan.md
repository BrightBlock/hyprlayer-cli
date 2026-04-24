---
description: Create detailed implementation plans through interactive research and iteration
model: {{OPUS_MODEL}}
subtask: false
---

> **Path convention**: the `thoughts/shared/...` paths in examples and templates below are literal on `git`/`obsidian` backends. On `notion`/`anytype`, substitute the matching `notion://<id>` / `anytype://<id>` identifier that `hyprlayer storage info` or `thoughts-locator` returns.

# Implementation Plan

You are tasked with creating detailed implementation plans through an interactive, iterative process. You should be skeptical, thorough, and work collaboratively with the user to produce high-quality technical specifications.

## Storage backend dispatch

Before you start, run `hyprlayer storage info --json` and parse the output. The `backend` field tells you where to save any artifacts this command produces. The `schema` field tells you which metadata properties are required — **populate every required field** regardless of backend. If the `hyprlayer` binary is not available or the project isn't mapped, proceed with the `git` branch below using relative `thoughts/shared/...` paths.

### Where to save

- **`git`**: write local markdown files through the project's `thoughts/shared/...` symlinks exactly as today. Prepend the required metadata as YAML frontmatter (see "Required metadata" below). `settings.thoughtsRepo` gives the absolute path. At the end, remind the user to run `hyprlayer thoughts sync` so the artifact is pushed.
- **`obsidian`**: the project's `thoughts/` symlinks are still present and point into the user's vault, so relative paths like `thoughts/shared/plans/<file>.md` continue to work for writes. Prepend the required metadata as YAML frontmatter — Obsidian's Properties panel picks it up automatically. For absolute on-disk paths, use `settings.contentRoot` + `settings.reposDir` + `mappedName`. Do NOT remind the user to sync — Obsidian has no sync step.
- **`notion`**: do NOT write local files. Ensure the target database exists:
  1. If `settings.databaseId` is populated, call `mcp__notion__retrieve-database` with that ID. If it resolves, skip to step 4.
  2. If `databaseId` is missing or retrieval returns not-found, call `mcp__notion__create-database` under `settings.parentPageId` with `title: "Hyprlayer Thoughts"` and one property per entry in `storage info`'s `schema` array (title → `title`; text → `rich_text`; date → `date`; select → `select` with `options`; tags → `multi_select`; relation → `relation` self-referential).
  3. Run `hyprlayer storage set-database-id <returned_id>` to persist. Proceed with step 4 using the new ID.
  4. Create a database row using `mcp__notion__create-page` with `parent.database_id = <id>`. Populate every required schema field as a typed property; the body receives the narrative content from the template below.
  If the Notion MCP tools are not available in this session, tell the user to run `hyprlayer thoughts init --backend notion` and stop.
- **`anytype`**: do NOT write local files. Ensure the target type + properties + tags exist before creating the object:
  1. **Resolve the type.** If `settings.typeId` is populated, call `mcp__anytype__API-get-type` with that ID + `settings.spaceId`. If it resolves, skip to step 4. If it returns not-found / 404 / 410, treat as missing and fall through.
  2. **Create the type + properties.** Call `mcp__anytype__API-create-type` in `spaceId` with `name: "Hyprlayer Thought"`, `plural_name: "Hyprlayer Thoughts"`, `key: "hyprlayer_thought"`, `layout: "basic"`. Then create each schema field (except `title` — maps to `name`) via `mcp__anytype__API-create-property` with `key: "hyprlayer_<field>"`. For `select` / `multi_select` fields, pass `tags: [...]` in the create-property call with one entry per `schema.options` value. Treat "property key already exists" as success. **Then call `mcp__anytype__API-update-type`** with `properties: [...]` listing every field — this is what links the properties to the type so the UI renders them.
  3. **Persist the type ID.** Run `hyprlayer storage set-type-id <returned_id>`. Proceed with step 4 using the new ID.
  4. **Ensure select tags exist** for this write's values. For `type`, `status`, `scope`, and each `tags` value, call `mcp__anytype__API-list-tags` and create missing ones via `mcp__anytype__API-create-tag`. Record returned tag IDs — the object-create call takes IDs, not string names.
  5. **Create the object.** `mcp__anytype__API-create-object` with `type_key: "hyprlayer_thought"`, `space_id`, `name: <title>`, `body: <narrative>`, and a `properties` array of typed values (`select: <tag_id>`, `multi_select: [<tag_id>, …]`, `date: "YYYY-MM-DD"`, `text: "…"`). Do NOT dump metadata as frontmatter in the body — Anytype's search relies on typed properties.
  If the Anytype MCP tools are not available, tell the user to start the Anytype app and run `hyprlayer thoughts init --backend anytype`, then stop.

### Required metadata

Read the `schema` array from `storage info --json`. Populate **every field marked `required: true`**. For this command:

| Field | How to determine |
|---|---|
| `title` | Derive from the task — short, human-readable |
| `type` | `plan` |
| `date` | Today's date in `YYYY-MM-DD` |
| `status` | `draft` for newly-created plans |
| `project` | `mappedName` from the same JSON output |
| `scope` | `shared` unless the user's task clearly implies `user` or `global` |
| `author` | Pull from `hyprlayer thoughts config --json` (the `user` field) or derive from `git config user.name` |
| `ticket` | If the task references `ENG-XXXX` or similar, capture it; otherwise null |
| `tags` | Derive 2-5 topic tags from the task |
| `related` | Leave empty unless the task explicitly references another plan/research doc |

For `select` fields, the `schema.options` array lists the legal values — do not invent new ones. For `git`/`obsidian`, render the above as YAML frontmatter at the top of the file. For `notion`/`anytype`, set them as typed database/object properties.

## Initial Response

When this command is invoked:

1. **Check if parameters were provided**:
   - If a file path or ticket reference was provided as a parameter, skip the default message
   - Immediately read any provided files FULLY
   - Begin the research process

2. **If no parameters provided**, respond with:
```
I'll help you create a detailed implementation plan. Let me start by understanding what we're building.

Please provide:
1. The task/ticket description (or reference to a ticket file)
2. Any relevant context, constraints, or specific requirements
3. Links to related research or previous implementations

I'll analyze this information and work with you to create a comprehensive plan.

Tip: You can also invoke this command with a ticket file directly: `/create_plan thoughts/allison/tickets/eng_1234.md`
For deeper analysis, try: `/create_plan think deeply about thoughts/allison/tickets/eng_1234.md`
```

Then wait for the user's input.

## Process Steps

### Step 1: Context Gathering & Initial Analysis

1. **Read all mentioned files immediately and FULLY**:
   - Ticket files (e.g., `thoughts/allison/tickets/eng_1234.md`)
   - Research documents
   - Related implementation plans
   - Any JSON/data files mentioned
   - **IMPORTANT**: Use the Read tool WITHOUT limit/offset parameters to read entire files
   - **CRITICAL**: DO NOT delegate to sub-agents before reading these files yourself in the main context
   - **NEVER** read files partially - if a file is mentioned, read it completely

2. **Delegate initial research to sub-agents to gather context**:
   Before asking the user any questions, use specialized agents to research:

   - `@codebase-locator` -- Find all files related to the ticket/task
   - `@codebase-analyzer` -- Understand how the current implementation works
   - If relevant, `@thoughts-locator` -- Find any existing thoughts documents about this feature
   - If a JIRA ticket is mentioned, `@jira-ticket-reader` -- Get full details

   These agents will:
   - Find relevant source files, configs, and tests
    - Identify the specific directories to focus on (e.g., if CLI is mentioned, they'll focus on src/)
   - Trace data flow and key functions
   - Return detailed explanations with file:line references

3. **Read all files identified by research tasks**:
   - After research tasks complete, read ALL files they identified as relevant
   - Read them FULLY into the main context
   - This ensures you have complete understanding before proceeding

4. **Analyze and verify understanding**:
   - Cross-reference the ticket requirements with actual code
   - Identify any discrepancies or misunderstandings
   - Note assumptions that need verification
   - Determine true scope based on codebase reality

5. **Present informed understanding and focused questions**:
   ```
   Based on the ticket and my research of the codebase, I understand we need to [accurate summary].

   I've found that:
   - [Current implementation detail with file:line reference]
   - [Relevant pattern or constraint discovered]
   - [Potential complexity or edge case identified]

   Questions that my research couldn't answer:
   - [Specific technical question that requires human judgment]
   - [Business logic clarification]
   - [Design preference that affects implementation]
   ```

   Only ask questions that you genuinely cannot answer through code investigation.

### Step 2: Research & Discovery

After getting initial clarifications:

1. **If the user corrects any misunderstanding**:
   - DO NOT just accept the correction
   - Delegate new research to sub-agents to verify the correct information
   - Read the specific files/directories they mention
   - Only proceed once you've verified the facts yourself

2. **Create a research checklist** as a markdown checklist to track exploration tasks

3. **Delegate to sub-agents for comprehensive research**:
   - Use the right agent for each type of research:

   **For deeper investigation:**
   - `@codebase-locator` -- Find more specific files (e.g., "find all files that handle [specific component]")
   - `@codebase-analyzer` -- Understand implementation details (e.g., "analyze how [system] works")
   - `@codebase-pattern-finder` -- Find similar features we can model after

   **For historical context:**
   - `@thoughts-locator` -- Find any research, plans, or decisions about this area
   - `@thoughts-analyzer` -- Extract key insights from the most relevant documents

   **For related tickets:**
   - `@jira-searcher` -- Find similar issues or past implementations

   Each agent knows how to:
   - Find the right files and code patterns
   - Identify conventions and patterns to follow
   - Look for integration points and dependencies
   - Return specific file:line references
   - Find tests and examples

3. **Wait for ALL sub-tasks to complete** before proceeding

4. **Present findings and design options**:
   ```
   Based on my research, here's what I found:

   **Current State:**
   - [Key discovery about existing code]
   - [Pattern or convention to follow]

   **Design Options:**
   1. [Option A] - [pros/cons]
   2. [Option B] - [pros/cons]

   **Open Questions:**
   - [Technical uncertainty]
   - [Design decision needed]

   Which approach aligns best with your vision?
   ```

### Step 3: Plan Structure Development

Once aligned on approach:

1. **Create initial plan outline**:
   ```
   Here's my proposed plan structure:

   ## Overview
   [1-2 sentence summary]

   ## Implementation Phases:
   1. [Phase name] - [what it accomplishes]
   2. [Phase name] - [what it accomplishes]
   3. [Phase name] - [what it accomplishes]

   Does this phasing make sense? Should I adjust the order or granularity?
   ```

2. **Get feedback on structure** before writing details

### Step 4: Detailed Plan Writing

After structure approval:

1. **Save the plan** following the storage backend dispatch from the top of this command. The title convention is `YYYY-MM-DD-ENG-XXXX-description` (omit the ticket chunk if there is none), e.g. `2025-01-08-ENG-1478-parent-child-tracking` or `2025-01-08-improve-error-handling`.
   - For `git`/`obsidian`: write to `thoughts/shared/plans/<title>.md` with YAML frontmatter containing every required schema field.
   - For `notion`/`anytype`: create the database row / object with every required property populated; the narrative content below becomes the body.
2. **Use this template structure**:

````markdown
# [Feature/Task Name] Implementation Plan

## Overview

[Brief description of what we're implementing and why]

## Current State Analysis

[What exists now, what's missing, key constraints discovered]

## Desired End State

[A Specification of the desired end state after this plan is complete, and how to verify it]

### Key Discoveries:
- [Important finding with file:line reference]
- [Pattern to follow]
- [Constraint to work within]

## What We're NOT Doing

[Explicitly list out-of-scope items to prevent scope creep]

## Implementation Approach

[High-level strategy and reasoning]

## Phase 1: [Descriptive Name]

### Overview
[What this phase accomplishes]

### Changes Required:

#### 1. [Component/File Group]
**File**: `path/to/file.ext`
**Changes**: [Summary of changes]

```[language]
// Specific code to add/modify
```

### Success Criteria:

#### Automated Verification:
- [ ] Migration applies cleanly: `make migrate`
- [ ] Unit tests pass: `make test-component`
- [ ] Type checking passes: `npm run typecheck`
- [ ] Linting passes: `make lint`
- [ ] Integration tests pass: `make test-integration`

#### Manual Verification:
- [ ] Feature works as expected when tested via UI
- [ ] Performance is acceptable under load
- [ ] Edge case handling verified manually
- [ ] No regressions in related features

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 2: [Descriptive Name]

[Similar structure with both automated and manual success criteria...]

---

## Testing Strategy

### Unit Tests:
- [What to test]
- [Key edge cases]

### Integration Tests:
- [End-to-end scenarios]

### Manual Testing Steps:
1. [Specific step to verify feature]
2. [Another verification step]
3. [Edge case to test manually]

## Performance Considerations

[Any performance implications or optimizations needed]

## Migration Notes

[If applicable, how to handle existing data/systems]

## References

- Original ticket: `thoughts/allison/tickets/eng_XXXX.md`
- Related research: `thoughts/shared/research/[relevant].md`
- Similar implementation: `[file:line]`
````

### Step 5: Sync and Review

1. **Sync (git backend only)**:
   - For `backend: git`, run `hyprlayer thoughts sync` so the plan is pushed upstream.
   - For `obsidian`/`notion`/`anytype`, skip this step.

2. **Present the draft plan location**:
   ```
   I've created the initial implementation plan at [path or database row link].

   Please review it and let me know:
   - Are the phases properly scoped?
   - Are the success criteria specific enough?
   - Any technical details that need adjustment?
   - Missing edge cases or considerations?
   ```

3. **Iterate based on feedback** - be ready to:
   - Add missing phases
   - Adjust technical approach
   - Clarify success criteria (both automated and manual)
   - Add/remove scope items
   - For `backend: git`, re-run `hyprlayer thoughts sync` after each round of edits

4. **Continue refining** until the user is satisfied

## Important Guidelines

1. **Be Skeptical**:
   - Question vague requirements
   - Identify potential issues early
   - Ask "why" and "what about"
   - Don't assume - verify with code

2. **Be Interactive**:
   - Don't write the full plan in one shot
   - Get buy-in at each major step
   - Allow course corrections
   - Work collaboratively

3. **Be Thorough**:
   - Read all context files COMPLETELY before planning
   - Research actual code patterns using parallel sub-tasks
   - Include specific file paths and line numbers
   - Write measurable success criteria with clear automated vs manual distinction
    - automated steps should use `cargo` whenever possible - for example `cargo check`, `cargo test`, `cargo clippy`, `cargo fmt --check`

4. **Be Practical**:
   - Focus on incremental, testable changes
   - Consider migration and rollback
   - Think about edge cases
   - Include "what we're NOT doing"

5. **Track Progress**:
   - Use a markdown checklist to track planning tasks
   - Update the checklist as you complete research
   - Mark planning tasks complete when done

6. **No Open Questions in Final Plan**:
   - If you encounter open questions during planning, STOP
   - Research or ask for clarification immediately
   - Do NOT write the plan with unresolved questions
   - The implementation plan must be complete and actionable
   - Every decision must be made before finalizing the plan

## Success Criteria Guidelines

**Always separate success criteria into two categories:**

1. **Automated Verification** (can be run by execution agents):
   - Commands that can be run: `make test`, `npm run lint`, etc.
   - Specific files that should exist
   - Code compilation/type checking
   - Automated test suites

2. **Manual Verification** (requires human testing):
   - UI/UX functionality
   - Performance under real conditions
   - Edge cases that are hard to automate
   - User acceptance criteria

**Format example:**
```markdown
### Success Criteria:

#### Automated Verification:
- [ ] Database migration runs successfully: `make migrate`
- [ ] All unit tests pass: `go test ./...`
- [ ] No linting errors: `golangci-lint run`
- [ ] API endpoint returns 200: `curl localhost:8080/api/new-endpoint`

#### Manual Verification:
- [ ] New feature appears correctly in the UI
- [ ] Performance is acceptable with 1000+ items
- [ ] Error messages are user-friendly
- [ ] Feature works correctly on mobile devices
```

## Common Patterns

### For Database Changes:
- Start with schema/migration
- Add store methods
- Update business logic
- Expose via API
- Update clients

### For New Features:
- Research existing patterns first
- Start with data model
- Build backend logic
- Add API endpoints
- Implement UI last

### For Refactoring:
- Document current behavior
- Plan incremental changes
- Maintain backwards compatibility
- Include migration strategy

## Sub-agent Delegation Best Practices

When delegating research to sub-agents:

1. **Delegate to multiple agents** for efficiency
2. **Each agent request should be focused** on a specific area
3. **Provide detailed instructions** including:
   - Exactly what to search for
   - Which directories to focus on
   - What information to extract
   - Expected output format
4. **Be EXTREMELY specific about directories**:
    - If the ticket mentions "CLI", specify `src/` directory
    - If it mentions "daemon", specify `hld/` directory
    - Never use generic terms - be specific about which module or crate
   - Include the full path context in your requests
5. **Specify read-only operations** only
6. **Request specific file:line references** in responses
7. **Wait for all agents to complete** before synthesizing
8. **Verify agent results**:
   - If an agent returns unexpected results, delegate follow-up requests
   - Cross-check findings against the actual codebase
   - Don't accept results that seem incorrect

Example of delegating to multiple agents:
```
@codebase-locator find all database schema files and migrations
@codebase-analyzer analyze the API endpoint patterns in src/api/
@codebase-pattern-finder find UI component patterns similar to [component]
@codebase-locator find all test files related to [feature]
```

## Example Interaction Flow

```
User: /create_plan
Assistant: I'll help you create a detailed implementation plan...

User: We need to add parent-child tracking for Claude sub-tasks. See thoughts/allison/tickets/eng_1478.md
Assistant: Let me read that ticket file completely first...

[Reads file fully]

Based on the ticket, I understand we need to track parent-child relationships for Claude sub-task events in the hld daemon. Before I start planning, I have some questions...

[Interactive process continues...]
```
