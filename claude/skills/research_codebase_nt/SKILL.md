---
name: research_codebase_nt
description: Document codebase as-is without evaluation or recommendations, no-thoughts variant (omits thoughts-locator/thoughts-analyzer agents). Use when the user asks to map a codebase that does not use the standard thoughts directory. Read-only; produces a research artifact via the active storage backend.
model: opus
allowed-tools: Bash, Read, Grep, Glob, Agent, Write, Edit, Skill
---

# Research Codebase

Answer a research question about the codebase by mapping its areas in parallel and
synthesizing the maps into a research artifact. No-thoughts variant: there is no
paper trail to consult, so the live tree is the only source.

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
  destination:                 # resolved by storage-backend; restated for legibility
    git-or-obsidian:   thoughts/shared/research/<title>.md
    notion-or-anytype: database row / object with type=research

orchestration:
  owns: [decomposition, synthesis, persistence, sync]

  forbids: [archivist, thoughts-locator, thoughts-analyzer]
  # No-thoughts variant. There is no paper trail here, so the reference block's
  # `history` step has no counterpart and these three agents are off the table.

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
        - { value: repo-root,            src: pwd }
        - { value: exact-directories,    src: "the survey you ran in decompose" }
        - { value: boundary-vs-siblings, src: "the area list decompose produced" }
      ask: [how-it-works, what-it-connects-to, conventions, usage-examples, gaps]
      reject: not matches(exact-directories, "/")
      because: >
        Each cartographer returns a document-ready section with file:line refs.
        That is the body of `write`, so you synthesize maps, not raw search
        output. Ask for examples and usage as they exist, not just definitions.
        The reject rule is the mechanical half of "never a generic area" — it
        catches "the backend", not a real path aimed at the wrong area, which
        stays your judgment.

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
      requires: [map, targeted, web, tickets]
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
        - { value: repo-name,  src: "basename of git rev-parse --show-toplevel" }
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
      apply: [frontmatter, citations]
      sections-omit: [historical-context]
      judgment: >
        How do the maps connect? See "Synthesizing" below — this step is the
        one the sub-agents cannot do for you.
      because: >
        No thoughts directory means no `Historical Context (from thoughts/)`
        section and no `searchable/` path rewriting to do. The document is
        self-contained: a reader should need nothing but this file.

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
        After `permalinks`, never beside it: that step rewrites the artifact
        body, so syncing first pushes a document whose references are still
        local paths. `write` is named too, because a skipped `permalinks`
        satisfies this requirement and the ordering must survive that.

    - id: present
      requires: [write, permalinks, sync]
      inline: true
      because: >
        Last, as in the original's step 8: the summary describes the artifact
        as it now stands — permalinks rewritten, and pushed on a git backend.
        `write` is named alongside the two guarded steps so the ordering holds
        when either is skipped. A concise summary with key file references,
        then ask for follow-up questions. The document is self-contained; the
        message is the way in.

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

  frontmatter:
    field-case: matches(field, "^[a-z0-9_]+$")
    tags-from: [topic, components-studied]
    typed-properties-instead-of-body: backend == notion or backend == anytype

  citations:
    every-claim: file:line
    primary-source: live-codebase
```

## Judgment

**Decomposing.** Break the query into composable areas. Look past the literal
question to the patterns and connections behind it. Your area list bounds what the
research can find; miss a dimension and the document comes back thorough about the
wrong things — and here there is no paper trail to catch the omission later.

**Naming an area's directories.** "The CLI" is `src/`; "the daemon" is `hld/`. The
reject rule catches a missing path, not a plausible wrong one. Hand a cartographer
the wrong directory and you get a confident, well-cited map of the wrong code, which
is harder to catch than an empty one.

**Synthesizing.** The live codebase is the only source of truth this variant has, so
run the research fresh every time and never lean on an existing research document.
Weight how systems interact over any single component's internals: no sub-agent can
see the connections between maps, which is why you do this step yourself.

**Where documentarian discipline bends.** `constraints: [documentarian]` binds you
to describing what is rather than what should be. One override: if the user asks for
recommendations, that ask wins. Answer it in a clearly separated section and say why
the section exists.
