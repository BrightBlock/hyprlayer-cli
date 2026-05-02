# Sub-agent picker guide

Skills that delegate research to sub-agents pick from this catalog. Spawn agents in parallel when their queries are independent. Each agent already knows its job — do NOT write detailed prompts about HOW to search; tell it WHAT you are looking for.

## Codebase research

- **codebase-locator** — find WHERE files and components live.
- **codebase-analyzer** — understand HOW specific code works.
- **codebase-pattern-finder** — find concrete examples of existing patterns.

## Thoughts directory (only for skills that use the thoughts directory)

- **thoughts-locator** — discover what documents exist about the topic.
- **thoughts-analyzer** — extract key insights from specific documents.

Use these only on skills whose backend is `git`/`obsidian` and which assume the standard thoughts directory. The `_nt` and `_generic` skill variants omit them.

## Web research (only when the user explicitly asks)

- **web-search-researcher** — external documentation and resources. Instruct it to return LINKS, and include those links in the final report.

## JIRA tickets (when relevant)

- **jira-ticket-reader** — full details of a specific ticket.
- **jira-searcher** — related tickets or historical context.

## Spawning rules

- Run multiple agents in parallel when they search for different things.
- Start with locator and finder agents to find what exists, then use analyzer agents on the most promising findings.
- Be EXTREMELY specific about directories in your prompts. If the task mentions "CLI", say `src/`; if it mentions "daemon", say `hld/`. Never use generic terms.
- Wait for ALL sub-agent tasks to complete before synthesizing.
- For skills under documentarian rules (`_thoughts/documentarian-rules.md`), remind agents they are documenting, not evaluating or improving.
- Verify sub-task results: if something seems off, spawn follow-up tasks rather than accepting the result.
