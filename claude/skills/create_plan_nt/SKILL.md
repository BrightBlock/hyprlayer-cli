---
name: create_plan_nt
description: Create implementation plans with thorough research, no-thoughts variant (omits Notion/Obsidian/thoughts-locator agents and follow-up sync prompts). Use when the user asks for a plan in a project that does not use the standard thoughts directory. Produces a plan artifact via the active storage backend.
model: opus
allowed-tools: Bash, Read, Grep, Glob, Agent, Write, Edit, Skill
---

# Implementation Plan

Turn a task or ticket into a phased implementation plan, by researching the tree
in parallel, aligning with the user on an approach, and having the plan drafted
and adversarially reviewed before it is saved. No-thoughts variant: no thoughts
directory agents, no `archivist`, and no sync you run yourself.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - storage-backend            # where the artifact is saved
  - required-metadata          # schema fields + backend-specific title format
  - subagent-guide             # the agent catalog and the spawning rules
  - templates/plan             # the artifact body structure

on-empty-invocation: |
  I'll help you create a detailed implementation plan. Let me start by understanding what we're building.

  Please provide:
  1. The task/ticket description (or reference to a ticket file)
  2. Any relevant context, constraints, or specific requirements
  3. Links to related research or previous implementations

  I'll analyze this information and work with you to create a comprehensive plan.

  Tip: You can also invoke this command with a ticket file directly: `/create_plan thoughts/shared/tickets/eng_1234.md`
  For deeper analysis, try: `/create_plan think deeply about thoughts/shared/tickets/eng_1234.md`

artifact:
  type: plan
  title-from: task

orchestration:
  owns: [decomposition, arbitration, synthesis, persistence, user-dialogue]

  excludes:
    agents: [archivist, thoughts-locator, thoughts-analyzer]
    because: this is the no-thoughts variant — there is no paper trail to search

  steps:
    - id: read-mentioned
      inline: true
      reject: matches(read-call, "limit|offset")
      because: >
        A parameter means skip the opener and start here. Ticket files,
        research documents, prior plans, any JSON mentioned — read them
        yourself, in the main context, before spawning anything. A sub-agent
        summary is not a substitute for having read the ticket, and a partial
        read is not a substitute for the whole file.

    - id: decompose
      requires: [read-mentioned]
      inline: true
      produces: areas
      track-with: TodoWrite
      judgment: >
        Which areas of the codebase does this task touch, and which exact
        directories does each one own? See "Decomposing the task" below.

    - id: context-map
      requires: [decompose]
      fanout: cartographer
      over: areas
      given:
        - { value: repo-root,           src: pwd }
        - { value: exact-directories,   src: "the area list decompose produced" }
        - { value: task-statement,      src: "the files read in read-mentioned, plus the user's request verbatim" }
      ask: [how-it-works, what-it-connects-to, conventions, gaps]
      reject: not matches(exact-directories, "/")
      because: >
        Research before questions. Each cartographer returns a document-ready
        section for its area with file:line references, so what you hand the
        user in present-understanding is grounded rather than guessed. The
        reject rule is the mechanical half of "never a generic term" — it
        catches "the backend", not a real path aimed at the wrong area.

    - id: context-ticket
      requires: [decompose]
      when: matches(request, "[A-Z]{2,}-\d+") and not matches(request, "(ADR|RFC|CVE|ISO|PR|SHA|UTF)-\d+")
      when-examples:
        match:    ["plan ENG-1478", "see PROJ-22 for the acceptance criteria"]
        no-match: ["per ADR-0002", "RFC-1234 says", "CVE-2021-44228", "add retries to the uploader"]
      agent: one-of [jira-ticket-reader, jira-searcher]
      because: >
        Only when a ticket is actually named. `jira-ticket-reader` for a known
        key, `jira-searcher` when you have a topic and not an id.

    - id: read-identified
      requires: [context-map, context-ticket]
      inline: true
      reject: matches(read-call, "limit|offset")
      because: >
        The maps name files; the files are the evidence. Read every file the
        research surfaced, fully, into the main context. You cannot
        cross-reference a ticket against code you have only read about.

    - id: verify-understanding
      requires: [read-identified]
      inline: true
      produces: [discrepancies, assumptions-needing-verification, true-scope]
      judgment: >
        Where does the ticket disagree with the tree, and what is the real
        scope once you account for the difference? See "Scope is what the code
        says" below.

    - id: present-understanding
      requires: [verify-understanding]
      inline: true
      presents:
        - accurate-summary-of-what-we-are-building
        - findings-with-file-line-references
        - questions-research-could-not-answer
      judgment: >
        Which of your open questions genuinely cannot be answered by reading
        more code? See "Which questions are actually for the human" below.
      because: >
        Questions come after the research, never before it. Asking the user
        something the tree already answers spends their attention on work you
        could have done yourself.

    - id: verify-corrections
      requires: [present-understanding]
      inline: true
      retry: { step: context-map, max: 1 }
      judgment: >
        Did the user's answer correct a fact you asserted, and have you
        verified the correction yourself? See "A correction is a claim, not a
        fact" below.

    - id: deeper-research
      requires: [verify-corrections]
      fanout: cartographer
      over: follow-up-areas
      track-with: TodoWrite
      given:
        - { value: repo-root,           src: pwd }
        - { value: exact-directories,   src: "the areas the first round surfaced" }
        - { value: what-round-one-left-open, src: "the gaps: sections of the context-map reports" }
      ask: [how-it-works, what-it-connects-to, conventions, gaps]
      reject: not matches(exact-directories, "/")
      because: >
        A second round aimed at what round one surfaced, not a repeat of round
        one. Everything ready spawns in one message; nothing downstream starts
        until all of them return.

    - id: deeper-targeted
      requires: [verify-corrections]
      agent: one-of [codebase-locator, codebase-analyzer, codebase-pattern-finder]
      judgment: >
        Is any remaining gap one narrow question rather than an area needing a
        map? A cartographer on a single lookup wastes a context; a narrow
        agent on a whole area returns search output instead of a section.

    - id: deeper-tickets
      requires: [verify-corrections]
      when: matches(request, "[A-Z]{2,}-\d+") and not matches(request, "(ADR|RFC|CVE|ISO|PR|SHA|UTF)-\d+")
      when-examples:
        match:    ["blocked on ENG-1479", "related: PROJ-22"]
        no-match: ["per ADR-0002", "RFC-1234 says", "no ticket here"]
      agent: one-of [jira-searcher, jira-ticket-reader]
      because: related tickets, not the one already read in context-ticket.

    - id: present-options
      requires: [deeper-research, deeper-targeted, deeper-tickets]
      inline: true
      presents: [current-state, design-options-with-tradeoffs, open-questions]
      judgment: >
        Which of these are genuinely different approaches rather than the same
        approach renamed? Two options that share a data model are one option,
        and offering them as two hides the decision instead of surfacing it.
      because: >
        The user picks the approach here, and everything downstream is written
        against that pick. A wrong one is cheap now and expensive after draft.

    - id: outline
      requires: [present-options]
      inline: true
      shapes: phase-shapes
      presents: [overview, numbered-phases-with-what-each-accomplishes]
      judgment: >
        What are the phases, in what order, at what granularity? See "Phasing
        granularity" below.

    - id: approve-outline
      requires: [outline]
      inline: true
      because: >
        Structure feedback before detail. This barrier is the whole reason the
        skill is interactive: restructuring after the body is written throws
        away the draughtsman's entire pass, and course corrections are free
        while the plan is still a list of phase names.

    - id: draft
      requires: [approve-outline]
      agent: draughtsman
      given:
        - { value: task,          src: "the files read in read-mentioned" }
        - { value: phase-outline, src: "the outline the user approved in approve-outline" }
        - { value: findings,      src: "the verified sub-agent maps and reports" }
        - { value: repo-root,     src: pwd }
        - { value: template,      src: "_thoughts/templates/plan.md" }
      ask: [phases-in-dependency-order, verified-file-paths, success-criteria-split]
      on:
        "BLOCKER:": resolve-it-before-moving-on-then-respawn
      judgment: >
        Is this plan small enough that delegating costs more than it saves?
        See "Drafting it yourself" below.
      because: >
        The draughtsman returns the plan body as markdown and writes no files.
        Persistence stays with you.

    - id: adjudicate
      requires: [draft]
      agent: adjudicator
      retry: { step: draft, max: 1 }
      given:
        - { value: draft-body, src: "the draughtsman's return in draft" }
        - { value: repo-root,  src: pwd }
      ask:
        - verdict
        - phantom-file-paths
        - unverifiable-success-criteria
        - phase-ordering-that-cannot-hold
        - unstated-open-questions
      on:
        "verdict: ship":   proceed-to-save
        "verdict: revise": fix-the-blocker-and-major-findings-then-re-run-it-once
        "verdict: reject": take-the-finding-back-to-the-user-rather-than-saving
      judgment: >
        Is each finding right? See "The arbiter, not a relay" below.
      because: >
        The review happens before the user ever sees the draft. A plan the
        user has already read is a plan they have already started trusting.

    - id: save
      requires: [adjudicate]
      inline: true
      given:
        - { value: date-iso,   src: "date -Iseconds" }
        - { value: git-commit, src: "git rev-parse HEAD" }
        - { value: branch,     src: "git branch --show-current" }
        - { value: author,     src: "hyprlayer thoughts config --json, else git config user.name" }
        - { value: backend,    src: "hyprlayer storage info --json" }
      apply: [template, frontmatter]
      reject: exists(open-question)
      title-format:
        git:      kebab-case-dated-slug     # 2025-01-08-ENG-1478-parent-child-tracking
        obsidian: kebab-case-dated-slug
        notion:   human-readable-heading
        anytype:  human-readable-heading
      destination:
        git:      thoughts/shared/plans/<title>.md
        obsidian: thoughts/shared/plans/<title>.md
        notion:   database-row (every required property populated, narrative as body)
        anytype:  object (every required property populated, narrative as body)
      because: >
        Every template placeholder populated, every required schema field
        present. The reject rule is the mechanical half of "no open questions
        in the final plan": if one is still open, stop and resolve it rather
        than writing the plan around it. Every decision has to be made before
        the file exists.

    - id: present
      requires: [save]
      inline: true
      presents: [artifact-location, review-prompts]
      review-prompts:
        - Are the phases properly scoped?
        - Are the success criteria specific enough?
        - Any technical details that need adjustment?
        - Missing edge cases or considerations?
      reminds-when:
        run-hyprlayer-thoughts-sync-yourself: backend == git
      because: >
        The no-thoughts variant does not push for you. On the git backend the
        sync is the user's to run, so say so with the location rather than
        leaving them with an unpushed path.

    - id: iterate
      requires: [present]
      inline: true
      accepts: [missing-phases, technical-approach, success-criteria, scope-items]
      because: continue refining until the user is satisfied.

conventions:

  template:
    source: _thoughts/templates/plan.md      # loads: templates/plan
    rule: every placeholder populated, nothing left as a placeholder

  frontmatter:
    source: _thoughts/required-metadata.md   # loads: required-metadata
    rule: every required schema field present
    title: per the step's own title-format map, which is backend-specific

  success-criteria:
    split: [automated, manual]
    automated:
      definition: runnable by an execution agent
      prefer: cargo                # cargo check, cargo test, cargo clippy, cargo fmt --check
      covers: [commands, files-that-must-exist, compilation, type-checking, test-suites]
    manual:
      definition: requires a human at a keyboard
      covers: [ui-ux, performance-under-real-conditions, hard-to-automate-edge-cases, user-acceptance]
    format: |
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

  plan-shape:
    changes: incremental-and-testable
    must-consider: [migration, rollback, edge-cases]
    must-include: what-we-are-NOT-doing
    open-questions: none-in-the-final-plan

  phase-shapes:
    database-change: [schema-migration, store-methods, business-logic, expose-via-api, update-clients]
    new-feature:     [research-existing-patterns, data-model, backend-logic, api-endpoints, ui-last]
    refactor:        [document-current-behavior, incremental-changes, backwards-compatibility, migration-strategy]

  subagent-prompts:
    spawn: every-ready-agent-in-one-message
    each-prompt-carries:
      - exactly-what-to-search-for
      - which-directories          # exact module or crate, never a generic term
      - what-information-to-extract
      - expected-output-format
      - read-only-tools
    returns: file:line
    on-unexpected-result: spawn-a-follow-up-and-cross-check-against-the-tree
    example-fanout:
      - 'cartographer  "Map how sessions are persisted — crates/hyprlayer-core/src/store/"'
      - 'cartographer  "Map the IPC command surface for sessions — src-tauri/src/commands/"'
      - 'cartographer  "Map how the session list renders — src/features/session/"'
      - 'codebase-pattern-finder  "Existing examples of a tauri-specta command + binding pair"'
```

## Judgment

**Decomposing the task.** Break the task into the areas of the codebase it
touches, looking past the literal ticket to the patterns and connections behind
it. Your area list bounds what the research can find, and the plan can only be as
correct as the research under it: miss a dimension and you get a thorough,
well-cited plan for the wrong change. With no `archivist` in this variant, the
live tree is your only source — nothing will surface a prior decision for you.

**Naming an area's directories.** "The CLI" is `src/`; "the daemon" is `hld/`.
Name the module or crate, never the concept. The reject rule catches a missing
path, not a plausible wrong one — hand a cartographer the wrong directory and you
get a confident map of the wrong code, which is harder to catch than an empty one.

**Scope is what the code says.** Cross-reference every ticket requirement against
the actual tree, and treat the difference as the finding. A ticket describes what
someone wanted; the code describes what is there. Where they disagree, the tree
decides the scope, and a plan sized to the ticket instead of the tree is the plan
that runs out of phases halfway through implementation.

**Which questions are actually for the human.** Ask only what code investigation
genuinely cannot settle: business logic, design preference, priorities between
tradeoffs. Everything else you look up. A question the tree already answers costs
the user a round trip and tells them the research was shallow — and buries the one
or two questions that really did need them.

**A correction is a claim, not a fact.** When the user corrects a misunderstanding,
do not just adopt it: spawn research against the files and directories they named
and verify it yourself. Users are usually right and sometimes remembering an older
version of the code. An unverified correction propagates straight into the phases,
where it is expensive; a verified one costs one extra round of agents.

**Phasing granularity.** Each phase should be independently testable and land in
one sitting, ordered so nothing depends on a later phase. Too coarse and the plan
cannot be verified until the end; too fine and it becomes a checklist nobody
follows. Consider migration and rollback at the boundaries, and say explicitly
what the plan is NOT doing — the unwritten exclusion is what gets argued about
later.

**Drafting it yourself.** Delegate to the `draughtsman` by default: it is the
step that turns findings into phases, and it does it with a fresh context. Draft
inline only when the plan is genuinely small — a single phase, a handful of files
— where handing over the full research costs more than writing the body. Getting
this wrong in the cheap direction wastes a spawn; getting it wrong in the
expensive direction means summarizing your research into a prompt and losing the
detail that made it worth gathering.

**The arbiter, not a relay.** The adjudicator's report is input, not a verdict you
execute. Fix what it got right, and where a finding is wrong say why and move on.
Churning the plan to satisfy a bad call degrades it while looking like diligence.
`verdict: reject` is different: that is the approach being wrong, and it goes back
to the user rather than into another revision.
