---
description: Document codebase as-is with thoughts directory for historical context
model: opus
---

# Research Codebase

You are tasked with conducting comprehensive research across the codebase to answer user questions by spawning parallel sub-agents and synthesizing their findings.

## Storage backend dispatch

Before you start, run `hyprlayer storage info --json` and parse the output. The `backend` field tells you where to save the research document. The `schema` field lists required metadata — **populate every required field** regardless of backend. If the `hyprlayer` binary is not available or the project isn't mapped, proceed with the `git` branch using relative `thoughts/shared/research/...` paths.

### Where to save

- **`git`**: write to `thoughts/shared/research/<title>.md` via the symlink. Prepend the required metadata as YAML frontmatter (see below). `settings.thoughtsRepo` gives the absolute path. At the end, remind the user to run `hyprlayer thoughts sync`.
- **`obsidian`**: the project's `thoughts/` symlinks point into the user's vault, so `thoughts/shared/research/<title>.md` works for writes. Prepend YAML frontmatter — Obsidian's Properties panel picks it up. Do NOT remind the user to sync.
- **`notion`**: do NOT write local files. Ensure the target database exists:
  1. If `settings.databaseId` is populated, call `mcp__notion__retrieve-database` with that ID. If it resolves, skip to step 4.
  2. If `databaseId` is missing or retrieval returns not-found, call `mcp__notion__create-database` under `settings.parentPageId` with `title: "Hyprlayer Thoughts"` and one property per entry in `storage info`'s `schema` array (title → `title`; text → `rich_text`; date → `date`; select → `select` with `options`; tags → `multi_select`; relation → `relation` self-referential).
  3. Run `hyprlayer storage set-database-id <returned_id>` to persist.
  4. Create a database row using `mcp__notion__create-page` with `parent.database_id = <id>`. Populate every required schema field as a typed property; the body receives the narrative content.
  If the Notion MCP tools are not available, tell the user to run `hyprlayer thoughts init --backend notion` and stop.
- **`anytype`**: do NOT write local files. Ensure the target type exists:
  1. If `settings.typeId` is populated, call `mcp__anytype__API-get-type` with that ID + `settings.spaceId`. If it resolves, skip to step 4.
  2. If `typeId` is missing or resolution fails, call `mcp__anytype__API-create-type` + `mcp__anytype__API-create-property` for each schema field.
  3. Run `hyprlayer storage set-type-id <returned_id>` to persist.
  4. Create an object of that type using `mcp__anytype__API-create-object`. Populate every required schema field as a property.
  If the Anytype MCP tools are not available, tell the user to start the Anytype app and run `hyprlayer thoughts init --backend anytype`, then stop.

### Required metadata

Populate every `required: true` field from `storage info`'s `schema` array. For this command: `type: research`, `status: draft` (or `active` if research is ongoing), `project: <mappedName>`, `scope: shared`, `date: YYYY-MM-DD`, `author` from `hyprlayer thoughts config --json`, `ticket` if the user references one, 2-5 `tags` naming the components/topics researched, and `title` matching the research question. Legal values for `select` fields are in `schema.options`.

For `git`/`obsidian`, render as YAML frontmatter. For `notion`/`anytype`, set as typed properties. The research-specific fields below (`researcher`, `git_commit`, `branch`, `repository`, `last_updated`, `last_updated_by`, `status`, `topic`) continue to be included in the document body/frontmatter as today — they supplement, not replace, the schema fields.

## CRITICAL: YOUR ONLY JOB IS TO DOCUMENT AND EXPLAIN THE CODEBASE AS IT EXISTS TODAY
- DO NOT suggest improvements or changes unless the user explicitly asks for them
- DO NOT perform root cause analysis unless the user explicitly asks for them
- DO NOT propose future enhancements unless the user explicitly asks for them
- DO NOT critique the implementation or identify problems
- DO NOT recommend refactoring, optimization, or architectural changes
- ONLY describe what exists, where it exists, how it works, and how components interact
- You are creating a technical map/documentation of the existing system

## Initial Setup:

When this command is invoked, respond with:
```
I'm ready to research the codebase. Please provide your research question or area of interest, and I'll analyze it thoroughly by exploring relevant components and connections.
```

Then wait for the user's research query.

## Steps to follow after receiving the research query:

1. **Read any directly mentioned files first:**
   - If the user mentions specific files (tickets, docs, JSON), read them FULLY first
   - **IMPORTANT**: Use the Read tool WITHOUT limit/offset parameters to read entire files
   - **CRITICAL**: Read these files yourself in the main context before spawning any sub-tasks
   - This ensures you have full context before decomposing the research

2. **Analyze and decompose the research question:**
   - Break down the user's query into composable research areas
   - Take time to ultrathink about the underlying patterns, connections, and architectural implications the user might be seeking
   - Identify specific components, patterns, or concepts to investigate
   - Create a research plan using TodoWrite to track all subtasks
   - Consider which directories, files, or architectural patterns are relevant

3. **Spawn parallel sub-agent tasks for comprehensive research:**
   - Create multiple Task agents to research different aspects concurrently
   - We now have specialized agents that know how to do specific research tasks:

   **For codebase research:**
   - Use the **codebase-locator** agent to find WHERE files and components live
   - Use the **codebase-analyzer** agent to understand HOW specific code works (without critiquing it)
   - Use the **codebase-pattern-finder** agent to find examples of existing patterns (without evaluating them)

   **IMPORTANT**: All agents are documentarians, not critics. They will describe what exists without suggesting improvements or identifying issues.

   **For thoughts directory:**
   - Use the **thoughts-locator** agent to discover what documents exist about the topic
   - Use the **thoughts-analyzer** agent to extract key insights from specific documents (only the most relevant ones)

   **For web research (only if user explicitly asks):**
   - Use the **web-search-researcher** agent for external documentation and resources
   - IF you use web-research agents, instruct them to return LINKS with their findings, and please INCLUDE those links in your final report

   **For JIRA tickets (if relevant):**
   - Use the **jira-ticket-reader** agent to get full details of a specific ticket
   - Use the **jira-searcher** agent to find related tickets or historical context

   The key is to use these agents intelligently:
   - Start with locator agents to find what exists
   - Then use analyzer agents on the most promising findings to document how they work
   - Run multiple agents in parallel when they're searching for different things
   - Each agent knows its job - just tell it what you're looking for
   - Don't write detailed prompts about HOW to search - the agents already know
   - Remind agents they are documenting, not evaluating or improving

4. **Wait for all sub-agents to complete and synthesize findings:**
   - IMPORTANT: Wait for ALL sub-agent tasks to complete before proceeding
   - Compile all sub-agent results (both codebase and thoughts findings)
   - Prioritize live codebase findings as primary source of truth
   - Use thoughts/ findings as supplementary historical context
   - Connect findings across different components
   - Include specific file paths and line numbers for reference
   - Verify all thoughts/ paths are correct (e.g., thoughts/allison/ not thoughts/shared/ for personal files)
   - Highlight patterns, connections, and architectural decisions
   - Answer the user's specific questions with concrete evidence

5. **Gather metadata for the research document:**
   - Run the `hack/spec_metadata.sh` script to generate all relevant metadata
   - Determine the artifact title as `YYYY-MM-DD-ENG-XXXX-description` (omit the ticket chunk if there is none), e.g. `2025-01-08-ENG-1478-parent-child-tracking` or `2025-01-08-authentication-flow`.
   - Destination is resolved by the storage backend dispatch at the top of this command:
     - For `git`/`obsidian`: `thoughts/shared/research/<title>.md`
     - For `notion`/`anytype`: a database row / object with `type: research`

6. **Generate research document:**
   - Use the metadata gathered in step 4
   - Structure the document with YAML frontmatter followed by content:
     ```markdown
     ---
     date: [Current date and time with timezone in ISO format]
     researcher: [Researcher name from thoughts status]
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

     **Date**: [Current date and time with timezone from step 4]
     **Researcher**: [Researcher name from thoughts status]
     **Git Commit**: [Current commit hash from step 4]
     **Branch**: [Current branch name from step 4]
     **Repository**: [Repository name]

     ## Research Question
     [Original user query]

     ## Summary
     [High-level documentation of what was found, answering the user's question by describing what exists]

     ## Detailed Findings

     ### [Component/Area 1]
     - Description of what exists ([file.ext:line](link))
     - How it connects to other components
     - Current implementation details (without evaluation)

     ### [Component/Area 2]
     ...

     ## Code References
     - `path/to/file.py:123` - Description of what's there
     - `another/file.ts:45-67` - Description of the code block

     ## Architecture Documentation
     [Current patterns, conventions, and design implementations found in the codebase]

     ## Historical Context (from thoughts/)
     [Relevant insights from thoughts/ directory with references]
     - `thoughts/shared/something.md` - Historical decision about X
     - `thoughts/local/notes.md` - Past exploration of Y
     Note: Paths exclude "searchable/" even if found there

     ## Related Research
     [Links to other research documents in thoughts/shared/research/]

     ## Open Questions
     [Any areas that need further investigation]
     ```

7. **Add GitHub permalinks (if applicable):**
   - Check if on main branch or if commit is pushed: `git branch --show-current` and `git status`
   - If on main/master or pushed, generate GitHub permalinks:
     - Get repo info: `gh repo view --json owner,name`
     - Create permalinks: `https://github.com/{owner}/{repo}/blob/{commit}/{file}#L{line}`
   - Replace local file references with permalinks in the document

8. **Sync (git only) and present findings:**
   - For `backend: git`, run `hyprlayer thoughts sync`. Skip this step for `obsidian`/`notion`/`anytype`.
   - Present a concise summary of findings to the user
   - Include key file references for easy navigation
   - Ask if they have follow-up questions or need clarification

9. **Handle follow-up questions:**
   - If the user has follow-up questions, append to the same research document (edit the file for `git`/`obsidian`; use `mcp__notion__update-page` / `mcp__anytype__API-update-object` for notion/anytype)
   - Update the frontmatter / properties field `last_updated` and `last_updated_by` to reflect the update
   - Add `last_updated_note: "Added follow-up research for [brief description]"` to frontmatter (git/obsidian) or a dedicated property (notion/anytype)
   - Add a new section: `## Follow-up Research [timestamp]`
   - Spawn new sub-agents as needed for additional investigation
   - For `backend: git`, sync again after updates

## Important notes:
- Always use parallel Task agents to maximize efficiency and minimize context usage
- Always run fresh codebase research - never rely solely on existing research documents
- The thoughts/ directory provides historical context to supplement live findings
- Focus on finding concrete file paths and line numbers for developer reference
- Research documents should be self-contained with all necessary context
- Each sub-agent prompt should be specific and focused on read-only documentation operations
- Document cross-component connections and how systems interact
- Include temporal context (when the research was conducted)
- Link to GitHub when possible for permanent references
- Keep the main agent focused on synthesis, not deep file reading
- Have sub-agents document examples and usage patterns as they exist
- Explore all of thoughts/ directory, not just research subdirectory
- **CRITICAL**: You and all sub-agents are documentarians, not evaluators
- **REMEMBER**: Document what IS, not what SHOULD BE
- **NO RECOMMENDATIONS**: Only describe the current state of the codebase
- **File reading**: Always read mentioned files FULLY (no limit/offset) before spawning sub-tasks
- **Critical ordering**: Follow the numbered steps exactly
  - ALWAYS read mentioned files first before spawning sub-tasks (step 1)
  - ALWAYS wait for all sub-agents to complete before synthesizing (step 4)
  - ALWAYS gather metadata before writing the document (step 5 before step 6)
  - NEVER write the research document with placeholder values
- **Path handling**: The thoughts/searchable/ directory contains hard links for searching
  - Always document paths by removing ONLY "searchable/" - preserve all other subdirectories
  - Examples of correct transformations:
    - `thoughts/searchable/allison/old_stuff/notes.md` → `thoughts/allison/old_stuff/notes.md`
    - `thoughts/searchable/shared/prs/123.md` → `thoughts/shared/prs/123.md`
    - `thoughts/searchable/global/shared/templates.md` → `thoughts/global/shared/templates.md`
  - NEVER change allison/ to shared/ or vice versa - preserve the exact directory structure
  - This ensures paths are correct for editing and navigation
- **Frontmatter consistency**:
  - Always include frontmatter at the beginning of research documents
  - Keep frontmatter fields consistent across all research documents
  - Update frontmatter when adding follow-up research
  - Use snake_case for multi-word field names (e.g., `last_updated`, `git_commit`)
  - Tags should be relevant to the research topic and components studied
