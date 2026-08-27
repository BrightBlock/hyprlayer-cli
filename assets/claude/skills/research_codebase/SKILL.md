---
name: research_codebase
description: Document codebase as-is with thoughts directory for historical context. Use when the user asks to research, document, or map an existing codebase area. Read-only; produces a thoughts artifact.
model: sonnet
allowed-tools: Bash, Read, Grep, Glob, Agent, Write, Edit, Skill, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# Research Codebase

Answer a research question about the codebase by mapping its areas in parallel and
synthesizing the maps into a thoughts artifact. You and every agent you spawn are
documentarians: describe what IS, never what SHOULD BE.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - storage-backend            # where the artifact is saved
  - required-metadata          # schema fields + backend-specific title format
  - documentarian-rules        # binds you and every agent you spawn
  - templates/research         # the artifact body structure

constraints: [documentarian]   # binds you and every agent you spawn

on-empty-invocation: >
  I'm ready to research the codebase. Please provide your research question or
  area of interest, and I'll analyze it thoroughly by exploring relevant
  components and connections.

artifact:
  type: research
  title-from: research-question
  destination:                 # resolved by storage-backend; restated for legibility
    git-or-obsidian:   thoughts/shared/research/<title>.md
    notion-or-anytype: database row / object with type=research

delegation:                    # binds every sub-agent prompt this skill writes
  read-only: true              # no spawned agent writes, edits, or runs a mutating command
  scope: one specific, focused question per prompt — never "look into X"
# This variant sets no `constraints:` file, so the read-only rule lives here
# instead: say it in every prompt you spawn, the same way a constraint would
# be said. Persistence is `owns:`, never delegated.

orchestration:
  owns: [decomposition, synthesis, persistence, sync]

  steps:
    - id: read-mentioned
      inline: true
      reject: matches(read-call, "limit|offset")
      because: >
        Full context before decomposing. A sub-agent summary is not a
        substitute for having read the ticket yourself, and a partial read is
        not a substitute for the whole file.

    - id: decompose
      requires: [read-mentioned]
      inline: true
      produces: areas
      track-with: task-list
      judgment: >
        What are the composable areas of this question, and which exact
        directories does each one own? See "Decomposing" below.

    - id: map
      requires: [decompose]
      fanout: cartographer
      over: areas
      given:
        - { value: repo-root,            src: pwd }
        - { value: exact-directories,    src: "the survey you ran in decompose" }
        - { value: boundary-vs-siblings, src: "the area list decompose produced" }
      ask: [how-it-works, what-it-connects-to, conventions, usage-examples, gaps]
      reject: not matches(exact-directories, "/")
      because: >
        Each cartographer returns a document-ready section with file:line refs.
        That is the body of `write`, so you synthesize maps, not raw search
        output. Ask for examples and usage patterns, not just definitions. The
        reject rule is the mechanical half of "never a generic area" — it
        catches "the backend", not a real path aimed at the wrong area, which
        stays your judgment.

    - id: history
      requires: [decompose]
      when: exit0(test -d thoughts) or backend == notion or backend == anytype
      when-examples:
        match:
          - "the project has a thoughts/ directory at the repo root"
          - "backend == notion, where the trail lives in the database"
        no-match: ["backend == git and the tree keeps no thoughts directory"]
      agent: archivist
      given: [{ value: topic, src: "the user's request, verbatim" }]
      ask: [what-was-decided, what-shipped, what-is-open, what-superseded-what]
      judgment: >
        Does this research want the prior trail? See "Prior context" below.
      because: >
        The guard only proves a trail exists. `notion`/`anytype` have no
        thoughts/ directory to test, so a bare `test -d thoughts` would skip
        the users with the richest trail; when the tree is the store, cover
        all of thoughts/, not just research/. With `thoughts-lookup` these are
        the only steps reading prior context; skipping both is the whole of a
        no-thoughts run.

    - id: thoughts-lookup
      requires: [decompose]
      when: backend == git or backend == obsidian
      when-examples:
        match:    ["backend == git", "backend == obsidian"]
        no-match: ["backend == notion", "backend == anytype"]
      agent: one-of [thoughts-locator, thoughts-analyzer]
      judgment: >
        Same call as `history`, plus: locator to find what exists, analyzer to
        pull facts from a document you can already name. See "Prior context".
      because: >
        Narrow lookups against a filesystem thoughts directory, for one
        specific fact rather than a synthesized trail — that is what the
        archivist is for. Both agents read files, so notion/anytype have
        nothing for them.

    - id: targeted
      requires: [decompose]
      agent: one-of [codebase-locator, codebase-analyzer, codebase-pattern-finder]
      judgment: >
        Is any part of this one narrow question rather than an area needing a
        map, and which of the three narrow agents fits it? See "Choosing the
        narrow agent" below.

    - id: web
      requires: [decompose]
      when: matches(request, "search the web|look online|latest docs|upstream")
      when-examples:
        match:    ["search the web for the spec", "check the latest docs"]
        no-match: ["how does our indexer work", "map the PTY stack"]
      agent: web-search-researcher
      given: [{ value: question, src: "the user's request, verbatim" }]
      ask: [answer-with-links]
      because: external research is opt-in — never inferred from the topic.

    - id: tickets
      requires: [decompose]
      when: matches(request, "[A-Z]{2,}-\d+") and not matches(request, "(ADR|RFC|CVE|ISO|PR|SHA|UTF)-\d+")
      when-examples:
        match:    ["fix ENG-1478", "see PROJ-22 for context"]
        no-match: ["per ADR-0002", "RFC-1234 says", "CVE-2021-44228", "no ticket here"]
      agent: one-of [jira-ticket-reader, jira-searcher]

    - id: verify-results
      requires: [map, history, targeted, web, tickets]
      inline: true
      retry: { step: map, max: 1 }
      judgment: >
        Does any report contradict another, or contradict something you
        checked yourself? Cross-check claims two agents both touch, and
        spot-check load-bearing constants against the tree. If one is wrong,
        spawn a follow-up rather than averaging them.
      because: >
        Every agent step above is a barrier for this one: nothing is
        synthesized until all of them have returned.

    - id: metadata
      requires: [verify-results]
      inline: true
      given:
        - { value: date-iso,   src: "date -Iseconds" }
        - { value: git-commit, src: "git rev-parse HEAD" }
        - { value: branch,     src: "git branch --show-current" }
        - { value: repo-name,  src: 'mappedName from hyprlayer storage info --json (already fetched below for backend); if null, d=$(git rev-parse --path-format=absolute --git-common-dir) || exit 1; d=${d%/.git}; basename "${d%.git}" — the SOURCE repo; --show-toplevel returns the worktree' }
        - { value: researcher, src: "hyprlayer thoughts config --json, else git config user.name" }
        - { value: backend,    src: "hyprlayer storage info --json" }
      reject: exists(unresolved-placeholder)
      because: >
        Gathered before writing, never after. A document written with
        placeholder metadata ships with placeholders.

    - id: write
      requires: [metadata]
      inline: true
      given:
        - { value: template, src: "_thoughts/templates/research.md" }
        - { value: findings, src: "the verified agent reports" }
      apply: [path-rewrite, frontmatter, citations]
      sections-when:
        # The `history` STEP, not its guard: no trail read, no section. A
        # heading over an empty body claims the trail was searched and bare.
        historical-context: history-ran
      judgment: >
        How do the maps connect? See "Synthesizing" below — this step is the
        one the sub-agents cannot do for you.
      because: >
        The document is self-contained either way — one with no
        historical-context section is not incomplete, just sourced entirely
        from the live tree.

    - id: permalinks
      requires: [write]
      when: exit0(git merge-base --is-ancestor HEAD @{u})
      when-examples:
        match:    ["HEAD is reachable from the upstream ref"]
        no-match: ["HEAD has unpushed commits"]
      inline: true
      follows: permalinks
      because: >
        An unpushed commit's permalink 404s. A passing guard is not optional:
        skip it only if the user said to, and say that you did.

    - id: sync
      requires: [write, permalinks]
      when: backend == git
      when-examples:
        match:    ["backend == git"]
        no-match: ["backend == notion"]
      inline: true
      run: hyprlayer thoughts sync
      because: >
        Original step 8 comes after step 7 for a reason: `permalinks` rewrites
        the artifact body, and syncing first pushes the pre-permalink version.
        The ordering has to be in `requires:` — requiring only `write` puts
        both in one wave. If `permalinks` skips, this still runs: a skipped
        step satisfies whatever required it.

    - id: present
      requires: [write, permalinks]
      inline: true
      because: >
        A concise summary with key file references, then ask for follow-up
        questions. The document is self-contained; the message is the way in.
        `permalinks` is required so the summary is not handed over while the
        links are still being written into the document it points at — the
        original orders permalinks before this, and a skipped `permalinks`
        satisfies this edge anyway.

    - id: follow-up
      requires: [present]
      judgment: >
        Is this a clarifying question about the doc just written, or a
        brand-new research topic? Only the former extends this artifact.
      inline: true
      appends-to: same-artifact
      updates: [last_updated, last_updated_by, last_updated_note]
      adds-section: "## Follow-up Research [timestamp]"
      re-runs: [map, targeted, sync]
      because: >
        Follow-up extends the artifact; it does not start a new one. Spawn
        fresh agents for whatever the follow-up actually needs, and re-run
        `sync` afterwards when the backend is git.

conventions:

  path-rewrite:
    applies-to: thoughts-paths
    strip: "searchable/"
    preserve: all-other-segments
    reject: matches(before, "/allison/") and not matches(after, "/allison/")
    examples:
      - thoughts/searchable/allison/old_stuff/notes.md → thoughts/allison/old_stuff/notes.md
      - thoughts/searchable/shared/prs/123.md         → thoughts/shared/prs/123.md
      - thoughts/searchable/global/shared/templates.md → thoughts/global/shared/templates.md

  frontmatter:
    field-case: matches(field, "^[a-z0-9_]+$")
    tags-from: [topic, components-studied]
    typed-properties-instead-of-body: backend == notion or backend == anytype

  citations:
    every-claim: file:line
    primary-source: live-codebase
    secondary-source: thoughts
```

## Judgment

**Prior context.** The guard proves a trail exists; only the request says whether this
research wants one. Skip `history` and `thoughts-lookup` when the user asks for the
codebase on its own terms — "fresh eyes", "just the code", "without the prior
research". Silent request and a trail exists: read it, and say so in the summary.
Reach for `thoughts-lookup` on top of `history` when you need one fact out of a named
document rather than the whole trail.

The error is asymmetric. An unwanted trail contaminates the finding — the research
comes back agreeing with the last document rather than the tree, and that reads as
corroboration. An excluded one costs a re-run.

**Decomposing.** Break the query into composable areas. Look past the literal
question to the patterns and connections behind it. Your area list bounds what the
research can find; miss a dimension and the document comes back thorough about the
wrong things.

**Naming an area's directories.** "The CLI" is `src/`; "the daemon" is `hld/`. The
reject rule catches a missing path, not a plausible wrong one. Hand a cartographer
the wrong directory and you get a confident, well-cited map of the wrong code, which
is harder to catch than an empty one.

**Choosing the narrow agent.** Past the per-area maps the block offers exactly
three: `codebase-locator` to find where a thing lives, `codebase-analyzer` to read
how one already-located thing works, `codebase-pattern-finder` to collect existing
examples of a shape. Pick one only for a question narrow enough that a whole mapped
section would be waste. A cartographer on a single lookup wastes a context; a narrow
agent on a whole area returns search output instead of a section; and a narrow agent
aimed at something this project does not have comes back empty and costs you a
wave.

**Synthesizing.** Live codebase findings are the primary source of truth; thoughts
findings, where the project keeps any, are supplementary historical context. Run the
research fresh every time rather than leaning on an existing research document. When
sources disagree, the tree wins and the disagreement is itself a finding. Weight how
systems interact over any single component's internals: no sub-agent can see the
connections between maps, which is why you do this step yourself.
