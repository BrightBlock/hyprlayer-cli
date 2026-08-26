# Sub-agent picker guide

Skills that delegate work to sub-agents pick from this catalog. Each agent already knows its job — do NOT write detailed prompts about HOW to work; tell it WHAT you need.

> **The mechanics of delegation live in the block, not here.** `given:` (with its `src:`) is the context you hand over, `ask:` is what you want back, `requires:` is the ordering and the barrier, `constraints:` binds every agent you spawn, and the wave schedule decides what runs in parallel. See `orchestration-runtime.md`; it is the authority on all of that, and this file does not restate it.
>
> What this file holds is what a block cannot say: **what each agent gives you back**, and **what a sub-agent can never be**.

Two families:

- **Stage agents** carry one stage of the hyprlayer pipeline. They are the behavior a skill used to inline — delegate to them instead of re-describing the work.
- **Research agents** are the narrow read-only workers. Use them directly when you need one specific thing; use a stage agent when you need the whole stage done.

The catalog matters most at an `agent: one-of [a, b, c]` step. `orchestrate compile` deliberately declines to pick — it records the choice in `unresolved[]` and leaves it to you. These tables are what you decide from.

On native Codex, these names have two resolution modes: normally, spawn the matching custom agent installed at `~/.codex/agents/`; if custom agents are gated off or the bundle is not installed, fall back to the matching persona in `./.claude/agents/` and then `~/.claude/agents/`.

OpenCode always uses a persona-file fallback. Hyprlayer exposes the shared skills through `~/.claude/skills`, but OpenCode does not discover either custom-agent registry. With a Claude model, read `./.claude/agents/<name>.md` and then `~/.claude/agents/<name>.md`; with every other model, read `./.codex/agents/<name>.toml` and then `~/.codex/agents/<name>.toml`, carrying its `developer_instructions`. In either case, put those instructions into a generic OpenCode sub-agent prompt. The model family also selects the return transport: Claude models follow Claude behavior, while every other model follows Codex behavior. Follow `orchestration-runtime.md` for the corresponding in-band or file-mediated fan-out prompt contract.

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

## Delegation limits

The `tools:` list IS the restriction — there is no separate allowlist key. Claude drops unknown frontmatter keys silently, so a made-up key restricts nothing.

- No `Agent` entry — cannot delegate. (`Task` is a legacy alias that still resolves.)
- No `Skill` entry — cannot invoke skills.
- `Agent(cartographer, foreman)` narrows delegation to the named agents. Names are agent `name`s, not paths.
- `skills: name-a, name-b` preloads skills into an agent.

All agents are leaves: none carry `Agent` or `Skill`. Fan-out belongs to the skill, which holds the user's context and owns persistence. If a stage agent starts needing sub-agents, that is a sign the stage should be split, not that the leaf rule should bend.

Because a skill's `allowed-tools` takes a bare `Skill` entry, a skill that can invoke one skill can invoke any of them. Keep the intended set named in the body prose — that is what actually gets read.

**Not every file in `claude/agents/` is spawnable.** `ship` is a *session* agent — it is what a whole Ship turn runs as (`claude --agent ship`), never something a skill spawns; for the read-only "draft me a PR body" job the answer is **herald**. `orchestrate check` resolves agent names from the filesystem and does not model this distinction, so `agent: ship` would pass validation and fail at spawn time. It is absent from the catalog above deliberately.

## Picking well

- Be EXTREMELY specific about directories in your prompts. If the task mentions "CLI", say `src/`; if it mentions "daemon", say `hld/`. Never use generic terms. A cartographer handed the wrong directory returns a confident, well-cited map of the wrong code, which is harder to catch than an empty one.
- Start with locator and finder agents to find what exists, then use analyzer agents on the most promising findings — when the block has not already fixed that order with `requires:`.
