---
name: research_codebase_declared
description: Document codebase as-is with thoughts directory for historical context — declarative-orchestration variant of research_codebase. Use when the user asks to research, document, or map an existing codebase area. Read-only; produces a thoughts artifact.
model: sonnet
allowed-tools: Bash, Read, Grep, Glob, Agent, Write, Edit, Skill, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# Research Codebase (declared)

Answer a research question about the codebase by mapping its areas in parallel and
synthesizing the maps into a thoughts artifact.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - storage-backend            # where the artifact is saved
  - required-metadata          # schema fields + backend-specific title format
  - documentarian-rules
  - templates/research         # the artifact body structure

constraints: [documentarian]   # binds you and every agent you spawn

on-empty-invocation: >
  I'm ready to research the codebase. Please provide your research question or
  area of interest, and I'll analyze it thoroughly by exploring relevant
  components and connections.

artifact:
  type: research
  title-from: research-question

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
      track-with: TodoWrite
      judgment: >
        What are the composable areas of this question, and which exact
        directories does each one own? See "Decomposing" below.

    - id: map
      requires: [decompose]
      fanout: cartographer
      over: areas
      given:
        - { value: repo-root,           src: pwd }
        - { value: exact-directories,   src: "the survey you ran in decompose" }
        - { value: boundary-vs-siblings, src: "the area list decompose produced" }
      ask: [how-it-works, what-it-connects-to, conventions, gaps]
      reject: not matches(exact-directories, "/")
      because: >
        Each cartographer returns a document-ready section with file:line refs.
        That is the body of `write`, so you synthesize maps, not raw search
        output. The reject rule is the mechanical half of "never a generic
        area" — it catches "the backend", not a real path aimed at the wrong
        area, which stays your judgment.

    - id: history
      requires: [decompose]
      agent: archivist
      given: [{ value: topic, src: "the user's request, verbatim" }]
      ask: [what-was-decided, what-shipped, what-is-open, what-superseded-what]

    - id: targeted
      requires: [decompose]
      agent: one-of [codebase-locator, codebase-analyzer, codebase-pattern-finder]
      judgment: >
        Is any part of this one narrow question rather than an area needing a
        map? A cartographer on a single lookup wastes a context; a narrow
        agent on a whole area returns search output instead of a section.

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

    - id: metadata
      requires: [verify-results]
      inline: true
      given:
        - { value: date-iso,   src: "date -Iseconds" }
        - { value: git-commit, src: "git rev-parse HEAD" }
        - { value: branch,     src: "git branch --show-current" }
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
        historical-context: backend != notion and backend != anytype
      judgment: >
        How do the maps connect? See "Synthesizing" below — this step is the
        one the sub-agents cannot do for you.

    - id: permalinks
      requires: [write]
      when: exit0(git merge-base --is-ancestor HEAD @{u})
      when-examples:
        match:    ["HEAD is reachable from the upstream ref"]
        no-match: ["HEAD has unpushed commits"]
      inline: true
      because: >
        An unpushed commit's permalink 404s. A passing guard is not optional:
        skip it only if the user said to, and say that you did.

    - id: sync
      requires: [write]
      when: backend == git
      when-examples:
        match:    ["backend == git"]
        no-match: ["backend == notion"]
      inline: true
      run: hyprlayer thoughts sync

    - id: present
      requires: [write]
      inline: true
      because: >
        A concise summary with key file references. The document is
        self-contained; the message is the way in.

    - id: follow-up
      requires: [present]
      judgment: >
        Is this a clarifying question about the doc just written, or a
        brand-new research topic? Only the former extends this artifact.
      inline: true
      appends-to: same-artifact
      updates: [last_updated, last_updated_by, last_updated_note]
      adds-section: "## Follow-up Research [timestamp]"
      because: follow-up extends the artifact; it does not start a new one.

conventions:

  path-rewrite:
    applies-to: thoughts-paths
    strip: "searchable/"
    preserve: all-other-segments
    reject: matches(before, "/allison/") and not matches(after, "/allison/")
    examples:
      - thoughts/searchable/allison/old_stuff/notes.md → thoughts/allison/old_stuff/notes.md
      - thoughts/searchable/shared/prs/123.md         → thoughts/shared/prs/123.md

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

**Decomposing.** Break the query into composable areas. Look past the literal
question to the patterns and connections behind it. Your area list bounds what the
research can find; miss a dimension and the document comes back thorough about the
wrong things.

**Naming an area's directories.** "The CLI" is `src/`; "the daemon" is `hld/`. The
reject rule catches a missing path, not a plausible wrong one. Hand a cartographer
the wrong directory and you get a confident, well-cited map of the wrong code, which
is harder to catch than an empty one.

**Synthesizing.** Live codebase findings are the primary source of truth; thoughts
findings are supplementary historical context. When they disagree, the tree wins and
the disagreement is itself a finding. Weight how systems interact over any single
component's internals: no sub-agent can see the connections between maps, which is
why you do this step yourself.

**Where documentarian discipline bends.** `constraints: [documentarian]` binds you
to describing what is rather than what should be. One override: if the user asks for
recommendations, that ask wins. Answer it in a clearly separated section and say why
the section exists.
