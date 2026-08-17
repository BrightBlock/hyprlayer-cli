# Sub-agent picker guide

Skills that delegate work to sub-agents pick from this catalog. Each agent already knows its job — do NOT write detailed prompts about HOW to work; tell it WHAT you need and hand it the context it cannot discover on its own (the plan, the diff range, the template, the repo root).

Two families:

- **Stage agents** carry one stage of the hyprlayer pipeline. They are the behavior a skill used to inline — delegate to them instead of re-describing the work.
- **Research agents** are the narrow read-only workers. Use them directly when you need one specific thing; use a stage agent when you need the whole stage done.

## Stage agents

| Stage | Agent | What you get back |
|---|---|---|
| Research | **cartographer** | One research area mapped end to end — where it lives, how it works, what it connects to — as a document-ready section with `file:line` refs. Spawn one per area, in parallel. Documentarian rules are built in. |
| Plan | **draughtsman** | A plan body drafted from an approved outline: phases in dependency order, verified file paths, success criteria split automated/manual. Returns markdown; you persist it. |
| Plan | **adjudicator** | An adversarial review of a draft plan — phantom paths, unverifiable criteria, phase order that can't hold, missing migrations — with `verdict: ship / revise / reject`. |
| Implement | **foreman** | One plan phase implemented in a fresh context, its automated checks run, and an honest report of what changed and what didn't. One agent per phase. |
| Validate | **inspector** | A validation report: every phase checked against the diff, every automated criterion run, manual items listed, ending in `verdict: promote / block`. |
| Commit | **quartermaster** | A proposed split of the working tree into atomic commits with drafted messages. Read-only — you get approval and run `git commit`. |
| Ship | **herald** | A filled-out PR description following the repo's template. Read-only — you run `gh pr edit`. |
| Revise | **marshal** | Raw review findings triaged: verified, deduplicated across reviewers, each ruled `fix now` / `defer` / `reject` with evidence. |
| Revise | **adversarial-reviewer** | An adversarial pass over a diff — edge cases, races, security holes, leaks, silent corruption — ordered by severity. Used by `/code_review` when codex is unavailable. |
| (memory) | **archivist** | The paper trail for a topic or ticket — what was decided, what shipped, what is still open — or a drafted handoff body. |

## Research agents

### Codebase

- **codebase-locator** — find WHERE files and components live.
- **codebase-analyzer** — understand HOW specific code works.
- **codebase-pattern-finder** — find concrete examples of existing patterns.

Prefer **cartographer** when you want an area mapped and written up; use these three when you want one targeted answer without the synthesis.

### Thoughts directory (only for skills that use the thoughts directory)

- **thoughts-locator** — discover what documents exist about the topic.
- **thoughts-analyzer** — extract key insights from specific documents.

Use these only on skills whose backend is `git`/`obsidian` and which assume the standard thoughts directory. The `_nt` and `_generic` skill variants omit them. Prefer **archivist** when you want the whole paper trail synthesized, or when the backend may be `notion`/`anytype`.

### Web research (only when the user explicitly asks)

- **web-search-researcher** — external documentation and resources. Instruct it to return LINKS, and include those links in the final report.

### JIRA tickets (when relevant)

- **jira-ticket-reader** — full details of a specific ticket.
- **jira-searcher** — related tickets or historical context.

## Delegation allowlists

Every agent and skill declares what it may reach for, in frontmatter:

- `allowed-agents` — sub-agents it may spawn.
- `allowed-skills` — skills it may invoke.

Both are comma-separated, or the literal `none`. **Absent means deny**, so a file that omits the key can spawn nothing — add the key when you add a file. hyprlayer-desktop reads these to enforce the graph.

All agents are leaves: `allowed-agents: none` on every one of them. Fan-out belongs to the skill, which holds the user's context and owns persistence. If a stage agent starts needing sub-agents, that is a sign the stage should be split, not that the leaf rule should bend.

`allowed-agents` must be a subset of what the skill's body actually sanctions, and a skill listing any agent needs `Agent` in `allowed-tools`.

## Spawning rules

- Run multiple agents in parallel when they work on different things — several cartographers over different areas, several foremen only when the phases are genuinely independent.
- Start with locator and finder agents to find what exists, then use analyzer agents on the most promising findings.
- Be EXTREMELY specific about directories in your prompts. If the task mentions "CLI", say `src/`; if it mentions "daemon", say `hld/`. Never use generic terms.
- Hand over context the agent cannot find on its own: the plan path or body, the diff range, the template path, which phase is theirs, the repo root.
- Wait for ALL sub-agent tasks to complete before synthesizing.
- For skills under documentarian rules (`_thoughts/documentarian-rules.md`), remind agents they are documenting, not evaluating or improving. The cartographer already knows.
- Verify sub-task results: if something seems off, spawn follow-up tasks rather than accepting the result. A `verdict:` line from an adjudicator, inspector, or marshal is advice — you and the user still decide.
- **The caller owns persistence.** Stage agents return bodies and verdicts; writing artifacts, promoting `status`, committing, and editing PRs stay in the skill, where the storage backend dispatch lives.
