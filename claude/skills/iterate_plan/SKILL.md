---
name: iterate_plan
description: Iterate on an existing implementation plan with thorough research and surgical updates. Use when the user asks to update, refine, or rewrite parts of an existing plan. Mutates an existing thoughts artifact (a plan).
model: opus
allowed-tools: Bash, Read, Grep, Glob, Agent, Write, Edit, Skill, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# Iterate Implementation Plan

Update an existing implementation plan against user feedback: read the plan whole,
research only what the change opens up, confirm the approach, then edit surgically.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - storage-backend            # "How to read existing artifacts" + "How to update existing artifacts"
  - required-metadata          # schema-required fields and legal `select` values
  - subagent-guide             # the sub-agent catalog and the spawning rules

on-empty-invocation: |
  I'll help you iterate on an existing implementation plan.

  Which plan would you like to update? Please provide the path to the plan file (e.g., `thoughts/shared/plans/2025-10-16-feature.md`).

  Tip: You can list recent plans with `ls -lt thoughts/shared/plans/ | head`

artifact:
  type: plan
  mutates: existing            # this skill never creates a plan
  title: unchanged             # unless the user asks to rename it
  updates: [last_updated, last_updated_by]   # where those fields are populated
  preserve: required-schema-fields

orchestration:
  owns: [input-parsing, scope, surgical-edits, user-confirmation, persistence, sync]

  steps:
    - id: parse-input
      inline: true
      produces: [plan-ref, requested-changes]
      reject: not exists(plan-ref) or not exists(requested-changes)
      because: >
        Two inputs, both required: which plan, and what to change about it.
        See `conventions.input-scenarios` for what to emit when one is
        missing. Nothing downstream guesses a plan from a directory listing
        or infers feedback from the plan's own contents.

    - id: read-plan
      requires: [parse-input]
      inline: true
      reject: matches(read-call, "limit|offset")
      because: >
        The whole plan before any edit — structure, phases, scope, success
        criteria, implementation approach. A partial read is how a surgical
        edit ends up contradicting a phase you never saw.

    - id: scope-research
      requires: [read-plan]
      inline: true
      produces: research-areas
      track-with: TodoWrite
      judgment: >
        Does this change need new technical understanding, and if so what are
        the areas and the exact directories each one owns? An empty list is a
        legitimate answer. See "Deciding what needs research" below.

    - id: map-areas
      requires: [scope-research]
      fanout: cartographer
      over: research-areas
      given:
        - { value: repo-root,         src: pwd }
        - { value: exact-directories, src: "the areas you named in scope-research" }
        - { value: requested-change,  src: "the user's feedback, verbatim" }
      ask: [how-it-works, what-it-connects-to, conventions, gaps]
      reject: not matches(exact-directories, "/")
      because: >
        One cartographer per area the feedback opens up, all spawned in one
        message. The reject rule is the mechanical half of "never a generic
        area": it catches "the backend", not a real path aimed at the wrong
        area, which stays your judgment.

    - id: prior-art
      requires: [scope-research]
      agent: archivist
      given:
        - { value: change-topic, src: "the user's feedback, verbatim" }
        - { value: plan-title,   src: "the plan you read in read-plan" }
      ask: [what-was-decided, what-shipped, what-is-open, what-superseded-what]
      judgment: >
        Does this iteration want the prior trail? See "Prior context" below.
      because: >
        The prior artifacts that bear on this change. The archivist covers
        every backend and returns one synthesized briefing. This and
        `thoughts-lookup` are the only steps reading prior context.

    - id: thoughts-lookup
      requires: [scope-research]
      when: backend == git or backend == obsidian
      when-examples:
        match:    ["backend == git", "backend == obsidian"]
        no-match: ["backend == notion", "backend == anytype"]
      agent: one-of [thoughts-locator, thoughts-analyzer]
      because: >
        Narrow lookups against a filesystem thoughts directory, for one
        specific fact out of a document you can already name. Both agents read
        files; on notion/anytype there are no files to read.

    - id: targeted
      requires: [scope-research]
      agent: one-of [codebase-locator, codebase-analyzer, codebase-pattern-finder]
      judgment: >
        Is any part of this one narrow question rather than an area needing a
        map? A cartographer on a single lookup wastes a context; a narrow
        agent on a whole area returns search output instead of a section.

    - id: read-new-files
      requires: [map-areas, prior-art, thoughts-lookup, targeted]
      inline: true
      reject: matches(read-call, "limit|offset")
      because: >
        Every file the research named, read fully into the main context and
        cross-referenced against what the plan already claims. `requires` is
        the barrier — no editing while an agent is still out.

    - id: verify-research
      requires: [read-new-files]
      inline: true
      retry: { step: map-areas, max: 1 }
      judgment: >
        Does any report contradict another, contradict the plan, or contradict
        something you checked yourself? A sub-agent's report is input, not
        truth: spawn a follow-up rather than averaging two of them.

    - id: confirm-approach
      requires: [verify-research]
      inline: true
      emits: templates.understanding
      blocks-on: user-confirmation
      judgment: >
        Is any part of this request wrong for this plan — vague, technically
        infeasible, or in conflict with a phase already there? See "Pushing
        back" below.
      because: >
        Say what you understood, what the research found, and what you intend
        to change; then stop and wait. This is the last cheap moment to
        course-correct, and it is why you do not disappear into research
        without reporting.

    - id: rewrite-phases
      requires: [confirm-approach]
      agent: draughtsman
      given:
        - { value: current-plan,     src: "the file you read in read-plan" }
        - { value: requested-change, src: "the user's feedback, verbatim" }
        - { value: research,         src: "the verified agent reports" }
      ask: [rewritten-phases]
      judgment: >
        Is this a substantial rewrite of one or more phases, or a surgical
        edit? Surgical edits stay yours. See "Delegating a rewrite" below.

    - id: edit-plan
      requires: [confirm-approach, rewrite-phases]
      inline: true
      apply: [surgical-edits, structure-consistency, quality-standards]
      updates: [last_updated, last_updated_by]
      reject: exists(open-question)
      because: >
        Edit, never rewrite wholesale: preserve content that does not need
        changing, keep every file:line reference accurate, and follow the
        existing phase pattern when adding one. A scope change updates "What
        We're NOT Doing"; an approach change updates "Implementation
        Approach". New content carries specific paths and line numbers, and
        success criteria stay measurable and split per
        `conventions.success-criteria`. A question the change raises gets
        asked and answered now — never parked in the plan as an open question.

    - id: adjudicate
      requires: [edit-plan]
      agent: adjudicator
      given:
        - { value: revised-plan, src: "the file you just edited" }
        - { value: repo-root,    src: pwd }
      ask: [phantom-paths, criteria-nobody-can-run, phase-consistency]
      judgment: >
        Did this iteration add or reorder a phase, change the approach, or
        rewrite success criteria? See "When adjudication earns its context"
        below.

    - id: sync
      requires: [edit-plan, adjudicate]
      when: backend == git
      when-examples:
        match:    ["backend == git"]
        no-match: ["backend == obsidian", "backend == notion", "backend == anytype"]
      inline: true
      run: hyprlayer thoughts sync

    - id: present-changes
      requires: [edit-plan, adjudicate, sync]
      inline: true
      emits: templates.change-summary
      because: >
        What changed, what the plan now does, and an invitation to adjust
        further. A skipped `sync` still satisfies this step's requirement, so
        this runs on every backend.

    - id: iterate-again
      requires: [present-changes]
      inline: true
      re-enters-at: read-plan
      judgment: >
        Is the next message another round on this same plan, or a different
        request? See "Re-entering the loop" below.
      because: >
        Another round re-reads the plan as it now stands. The copy in your
        context went stale the moment `edit-plan` returned.

conventions:

  input-scenarios:
    both-present: proceed straight to read-plan; no preliminary questions
    plan-only:    emit prompts.missing-feedback, wait
    neither:      emit on-empty-invocation, wait, then re-check for feedback

  prompts:
    missing-feedback: |
      I've found the plan at [path]. What changes would you like to make?

      For example:
      - "Add a phase for migration handling"
      - "Update the success criteria to include performance tests"
      - "Adjust the scope to exclude feature X"
      - "Split Phase 2 into two separate phases"

  templates:
    understanding: |
      Based on your feedback, I understand you want to:
      - [Change 1 with specific detail]
      - [Change 2 with specific detail]

      My research found:
      - [Relevant code pattern or constraint]
      - [Important discovery that affects the change]

      I plan to update the plan by:
      1. [Specific modification to make]
      2. [Another modification]

      Does this align with your intent?

    change-summary: |
      I've updated the plan at `thoughts/shared/plans/[filename].md`

      Changes made:
      - [Specific change 1]
      - [Specific change 2]

      The updated plan now:
      - [Key improvement]
      - [Another improvement]

      Would you like any further adjustments?

  surgical-edits:
    tool: Edit
    preserve: every part of the plan the change does not touch
    keep-accurate: file:line references
    update-if-needed: success-criteria

  structure-consistency:
    new-phase:       follows the pattern the existing phases use
    scope-change:    update "What We're NOT Doing"
    approach-change: update "Implementation Approach"
    keep-distinct:   automated vs manual success criteria

  quality-standards:
    new-content: specific file paths and line numbers
    criteria:    measurable, split per conventions.success-criteria
    language:    clear and actionable

  success-criteria:
    keep-two-categories: always
    automated:
      run-by: execution-agents
      prefer: [cargo check, cargo test, cargo clippy, cargo fmt --check]
      also:   [make test, npm run lint, files-that-should-exist, compilation, type-checking]
    manual:
      needs:  human-testing
      covers: [ui-ux, performance-under-real-conditions, hard-to-automate-edge-cases, user-acceptance]
    measurable: every criterion names what to run or what to look at

  spawning:
    only-when:  the change actually needs new technical understanding
    parallel:   every ready agent in one message
    focus:      one specific area per task
    each-prompt-carries: [what-to-search-for, which-directories, what-to-extract, expected-output-format]
    ask-for: "file:line references"
```

## Judgment

**Prior context.** Only the request says whether this iteration wants the prior trail.
Skip `prior-art` and `thoughts-lookup` when the user asks to work from the plan and the
code alone — "ignore the old research", "fresh look". Silent request: read it.

The error is asymmetric. An unwanted trail argues for the plan as written, which is the
thing being iterated on; an excluded one costs a re-run.

**Deciding what needs research.** Research only what the feedback actually opens
up — a change that touches code you have not read needs an area; a wording or
scope-note tweak needs none. Over-researching stalls a two-line edit behind four
contexts; under-researching produces a phase grounded in what you assumed the code
does, and the plan reads perfectly right up until someone tries to execute it.

**Naming an area's directories.** "The CLI" is `src/`; "the daemon" is `hld/`. The
reject rule catches a missing path, not a plausible wrong one. Hand a cartographer
the wrong directory and you get a confident, well-cited map of the wrong code — and
you will edit the plan to match it.

**Pushing back.** A change request can be wrong for the plan: vague, technically
infeasible, or in conflict with a phase already there. Question it before
`confirm-approach`, not after, and ask for the specific thing you need rather than
guessing at intent. Feedback edited in faithfully but understood loosely produces a
phase nobody can execute, and it now carries the authority of being written down.

**Delegating a rewrite.** A substantial rewrite of one or more phases goes to the
`draughtsman` with the current plan, the requested change, and the research;
surgical edits stay yours. Delegating a two-line edit costs a context and loses the
surrounding structure you had in view; keeping a whole-phase rewrite yourself
produces a phase that reads like a patch of the old one rather than a coherent
replacement.

**When adjudication earns its context.** Adjudicate whenever the iteration added or
reordered a phase, changed the approach, or rewrote success criteria — those are
exactly the edits that introduce phantom paths and criteria nobody can run. Skip it
for wording and scope-note tweaks. The cost of skipping it wrongly is a plan that
looks executable and is not; the cost of running it wrongly is one context spent
confirming a typo fix.

**Re-entering the loop.** Further feedback on this plan re-enters at `read-plan`
against the plan as it now stands — not at `edit-plan` against the copy in your
context, which is one iteration stale. A brand-new request is a new invocation.
Editing from the stale copy is how a second round silently reverts the first.
