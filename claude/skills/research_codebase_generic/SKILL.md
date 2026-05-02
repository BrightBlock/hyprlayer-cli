---
name: research_codebase_generic
description: Research a codebase comprehensively using parallel sub-agents (generic variant — minimal opinions about thoughts directory structure or sub-agent specialization). Use when the user asks for code research in a project that does not assume the standard thoughts directory. Read-only; produces a thoughts artifact (research).
model: opus
allowed-tools: Bash, Read, Grep, Glob, Agent
---

# Research Codebase

You are tasked with conducting comprehensive research across the codebase to answer user questions by spawning parallel sub-agents and synthesizing their findings.

## Storage backend dispatch

Read `~/.claude/skills/_thoughts/storage-backend.md` and follow it for where to save the artifact. Read `~/.claude/skills/_thoughts/required-metadata.md` for the schema-required fields and the backend-specific title format. For this command: artifact type is `research`; the title is derived from the research question.

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
   - Read `~/.claude/skills/_thoughts/subagent-guide.md` for the catalog and spawning rules.
   - For this skill (generic variant), use whichever sections apply to the project at hand.

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
   - Collect: current date/time (ISO with timezone), git commit hash, branch name, repository name, researcher name (from `hyprlayer thoughts config --json` or `git config user.name`).
   - Determine the artifact title per the backend-specific rule in `~/.claude/skills/_thoughts/required-metadata.md`.
   - Destination is resolved by the storage backend dispatch:
     - For `git`/`obsidian`: `thoughts/shared/research/<title>.md`
     - For `notion`/`anytype`: a database row / object with `type: research`

6. **Generate research document:**
   - Read `~/.claude/skills/_thoughts/templates/research.md` for the body structure.
   - Populate every placeholder using the metadata from step 5. Include the `Historical Context (from thoughts/)` section only if the project at hand uses a thoughts directory; otherwise omit it.

7. **Add GitHub permalinks (if applicable):**
   - Read `~/.claude/skills/_thoughts/permalinks.md` and follow it.

8. **Sync (git only) and present findings:**
   - For `backend: git`, run `hyprlayer thoughts sync`. Skip for `obsidian`/`notion`/`anytype`.
   - Present a concise summary of findings to the user
   - Include key file references for easy navigation
   - Ask if they have follow-up questions or need clarification

9. **Handle follow-up questions:**
   - If the user has follow-up questions, append to the same research document (edit the file for `git`/`obsidian`; use `mcp__notion__update-page` / `mcp__anytype__API-update-object` for notion/anytype)
   - Update `last_updated` and `last_updated_by` (frontmatter or properties)
   - Add `last_updated_note: "Added follow-up research for [brief description]"`
   - Add a new section: `## Follow-up Research [timestamp]`
   - Spawn new sub-agents as needed for additional investigation
   - For `backend: git`, sync again after updates

## Important notes:
- Always use parallel Task agents to maximize efficiency and minimize context usage
- Always run fresh codebase research - never rely solely on existing research documents
- The thoughts/ directory provides historical context to supplement live findings
- Focus on finding concrete file paths and line numbers for developer reference
- Research documents should be self-contained with all necessary context
- Each sub-agent prompt should be specific and focused on read-only operations
- Consider cross-component connections and architectural patterns
- Include temporal context (when the research was conducted)
- Link to GitHub when possible for permanent references
- Keep the main agent focused on synthesis, not deep file reading
- Encourage sub-agents to find examples and usage patterns, not just definitions
- Explore all of thoughts/ directory, not just research subdirectory
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
