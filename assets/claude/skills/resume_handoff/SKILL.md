---
name: resume_handoff
description: Resume work from a handoff document with context analysis, validation against current state, and a generated action plan. Side-effecting; user-only — never auto-invoke. Use when the user asks to resume from a prior handoff (by path or by ticket number).
allowed-tools: Bash, Read, Edit, Write, Grep, Glob, Agent, Skill, mcp__claude_ai_Notion__*, mcp__anytype__*
disable-model-invocation: true
---

# Resume work from a handoff document

Pick up work a previous session left behind: read its handoff in full, check every
claim it makes against the tree as it stands now, and turn what survives into an
agreed action plan.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - storage-backend            # "How to read existing artifacts" — the lookup dispatch
  - subagent-guide             # the catalog, and the rule that the caller reads first

reads-artifact:
  type: handoff
  addressed-by: [path, ticket]

on-empty-invocation: |
  I'll help you resume work from a handoff document. Let me find the available handoffs.

  Which handoff would you like to resume from?

  Tip: You can invoke this command directly with a handoff path: `/resume_handoff `thoughts/shared/handoffs/ENG-XXXX/YYYY-MM-DD_HH-MM-SS_ENG-XXXX_description.md`

  or using a ticket number to resume from the most recent handoff for that ticket: `/resume_handoff ENG-XXXX`

orchestration:
  owns: [handoff-reading, linked-artifact-reading, verification, confirmation-gates, task-list, implementation]

  steps:
    - id: sync-thoughts
      when: backend == git and matches(request, "[A-Z]{2,}-\d+") and not matches(request, "\.md")
      when-examples:
        match:    ["ENG-2124", "resume ENG-2124 please"]
        no-match: ["thoughts/shared/handoffs/ENG-2124/2026-01-14_09-30-00_ENG-2124_pty.md", "resume the handoff we wrote yesterday"]
      inline: true
      run: hyprlayer thoughts sync
      because: >
        Only the ticket-lookup path needs this — it searches the tree, and a
        stale tree finds a stale handoff or none at all. A path the user typed
        is already resolved, and obsidian/notion/anytype have no sync step.

    - id: locate-handoff
      requires: [sync-thoughts]
      when: matches(request, "[A-Z]{2,}-\d+") and not matches(request, "\.md")
      when-examples:
        match:    ["ENG-2124", "resume ENG-2124 please"]
        no-match: ["thoughts/shared/handoffs/ENG-2124/2026-01-14_09-30-00_ENG-2124_pty.md", "resume the handoff we wrote yesterday"]
      inline: true
      produces: [handoff-path, handoff-matches]
      apply: [handoff-lookup]
      because: >
        A ticket names a directory, not a document; there may be zero, one or
        many handoffs under it. Picking among them is mechanical, so the rule
        lives in `conventions.handoff-lookup`. A path skips this step.

    - id: ask-for-path
      requires: [locate-handoff]
      when: exists(handoff-matches) and count(handoff-matches) == 0
      when-examples:
        match:    ["the ticket's handoff directory is empty"]
        no-match:
          - "one handoff matched the ticket"
          - "four handoffs matched the ticket"
          - "the user gave a path, so no lookup ran and there is no match list"
      inline: true
      emits: >
        I'm sorry, I can't seem to find that handoff document. Can you please
        provide me with a path to it?
      because: >
        A miss stops the lookup: `read-handoff` requires this step, so nothing
        is read until the user has supplied a path. Do not widen to a
        neighbouring ticket or to the newest handoff in the repo — resuming
        the wrong session's work is worse than resuming none. The `exists(...)`
        conjunct keeps the apology off the path invocation, where no lookup
        ran and `handoff-matches` was never produced at all.

    - id: read-handoff
      requires: [locate-handoff, ask-for-path]
      inline: true
      given:
        - { value: handoff-path, src: "the invocation argument, or the file locate-handoff selected" }
      reject: not exists(handoff-path) or matches(read-call, "limit|offset")
      extract: [tasks-and-statuses, recent-changes, learnings, artifacts, action-items, other-notes]
      because: >
        Read FULLY, in the main context, before anything else happens — never
        through a sub-agent, never with limit/offset. Everything downstream is
        a claim this document makes, and you cannot weigh a claim you have
        only a summary of.

    - id: read-linked
      requires: [read-handoff]
      inline: true
      given:
        - { value: linked-artifacts, src: "the links in the handoff's Artifacts section" }
        - { value: backend,          src: "hyprlayer storage info --json" }
      extract: [key-requirements, decisions-already-made, phases-still-open]
      because: >
        The plan and research documents the handoff links to are read the same
        way, by the same backend dispatch that found the handoff. These are the
        critical files: a sub-agent's summary of the plan you are resuming is
        not the plan. Nothing is delegated until this step returns.

    - id: paper-trail
      requires: [name-areas]
      agent: archivist
      given:
        - { value: ticket-or-topic, src: "the handoff's frontmatter and the user's invocation" }
        - { value: handoff-date,    src: "the handoff's `date` field" }
      ask: [what-else-exists, what-superseded-what, which-plans-are-still-active, what-shipped-since]
      because: >
        The handoff is history, not truth. Someone may have replaced its plan,
        shipped half its action items, or closed the ticket since it was
        written, and none of that is visible from the document itself. It
        requires `name-areas` rather than `read-linked` so that it and `remap`
        are ready in the same wave and spawn in one message, which is what
        "parallel research tasks" means.

    - id: name-areas
      requires: [read-linked]
      inline: true
      produces: in-flight-areas
      judgment: >
        Which code areas does the handoff say were in flight, and which exact
        directories does each one own? See "Naming the in-flight areas'
        directories" below.

    - id: remap
      requires: [name-areas]
      fanout: cartographer
      over: in-flight-areas
      given:
        - { value: repo-root,         src: pwd }
        - { value: exact-directories, src: "the area list name-areas produced" }
        - { value: handoff-claim,     src: "that area's entries under Recent changes and Learnings" }
      ask: [how-it-works-now, what-moved-since, which-claims-still-hold, conventions]
      reject: not matches(exact-directories, "/")
      because: >
        The tree has moved since the handoff was written, so each area is
        mapped as it stands rather than as it was described. Hand every
        cartographer the handoff's claim about its own area so the report
        comes back as a comparison rather than a fresh survey.

    - id: read-critical-files
      requires: [paper-trail, remap]
      inline: true
      because: >
        Every sub-agent returns before this starts. Then read completely: the
        files named in Learnings, the files named in Recent changes, and the
        new related files the research surfaced — that last group is why this
        comes after the fan-out and not straight after read-handoff.

    - id: verify-references
      requires: [read-critical-files]
      inline: true
      checks: [file-references-still-resolve, changes-still-present, learnings-still-apply, no-new-conflicts-or-regressions]
      retry: { step: remap, max: 1 }
      because: >
        Never assume handoff state matches current state: verify ALL mentioned
        changes still exist and check for breaking changes since. A sub-agent's
        report is input, not truth — where one contradicts another or
        contradicts something you checked yourself, re-run the fan-out rather
        than averaging the two.

    - id: classify-divergence
      requires: [verify-references]
      inline: true
      scenarios: [clean-continuation, diverged-codebase, incomplete-handoff-work, stale-handoff]
      judgment: >
        Which scenario is this, and where the handoff and the tree disagree,
        which of them is describing reality? See "Reading the divergence"
        below.

    - id: present-analysis
      requires: [classify-divergence]
      inline: true
      apply: [analysis-report]
      because: >
        Findings before work. The report pairs every handoff claim with its
        current verification, so the user can see what has rotted before
        agreeing to anything.

    - id: confirm-direction
      requires: [present-analysis]
      inline: true
      because: >
        A gate, not a formality: nothing is edited until the user has agreed to
        the direction or corrected it. Course corrections are cheapest here.

    - id: action-plan
      requires: [confirm-direction]
      inline: true
      track-with: TodoWrite
      produces: task-list
      judgment: >
        In what order does the surviving work go, given what verification
        turned up? See "Ordering the resumed work" below.

    - id: present-plan
      requires: [action-plan]
      inline: true
      apply: [plan-presentation]
      because: >
        The todo list is shown and the first task named before it starts — the
        second gate, and the last cheap moment to reorder.

    - id: implement
      requires: [present-plan]
      inline: true
      apply: [handoff-learnings, documented-patterns, known-dead-ends]
      updates: [todo-status]
      because: >
        Start with the first approved task. Learnings is the section that earns
        the handoff its keep: the patterns it documents and the mistakes it
        names are how this session avoids paying for them twice.

    - id: continuity
      requires: [implement]
      inline: true
      cite-in-commits: handoff-path
      records: [deviations-from-the-original-plan]
      judgment: >
        Does this session end somewhere the next one would need its own handoff
        to reach? See "Whether to hand off again" below.

conventions:

  handoff-lookup:
    artifact-type: handoff
    by-path: read it directly — no lookup, and no default message
    by-ticket:
      git-obsidian: >
        list thoughts/shared/handoffs/<TICKET>/ and take the most recent by the
        YYYY-MM-DD_HH-MM-SS filename prefix
      notion-anytype: >
        query the database/type filtered by type = handoff, project =
        <mappedName>, ticket = <TICKET>, sorted by `date` descending
    linked-artifacts: >
      same backend, same dispatch — find a linked plan or research document by
      its `type` schema field

  handoff-sections: [tasks-and-statuses, recent-changes, learnings, artifacts, action-items, other-notes]

  scenarios:
    clean-continuation:
      looks-like: every change the handoff describes is present; no conflicts, no regressions; the action items name clear next steps
      response: proceed with the handoff's recommended actions
    diverged-codebase:
      looks-like: some changes missing or modified; new related code landed since
      response: reconcile the differences and adapt the plan to current state
    incomplete-handoff-work:
      looks-like: tasks left `in_progress`; partial implementations to re-understand
      response: finish the unfinished work before starting anything new
    stale-handoff:
      looks-like: significant time has passed; major refactoring since
      response: re-evaluate the strategy — the original approach may no longer apply

  analysis-report:
    opens-with: "I've analyzed the handoff from <date> by <researcher>. Here's the current situation:"
    sections:
      original-tasks:     "<task>: <status from handoff> → <current verification>"
      key-learnings:      "<learning, with file:line> — still valid / changed"
      recent-changes:     "<change> — verified present / missing / modified"
      artifacts-reviewed: "<document>: <key takeaway>"
      recommended-next:   "ordered, from the handoff's action items plus what verification turned up"
      potential-issues:   "conflicts, regressions, missing dependencies, broken code"
    closes-with: "Shall I proceed with <recommended action 1>, or would you like to adjust the approach?"

  plan-presentation:
    opens-with: "I've created a task list based on the handoff and current analysis:"
    body: the todo list, in order
    closes-with: "Ready to begin with the first task: <task>?"
```

## Judgment

**Naming the in-flight areas' directories.** The handoff names its areas in prose —
"the PTY stack", "the sync path" — and a cartographer needs `src/pty/`, not the
phrase. The reject rule catches a missing path, not a plausible wrong one, and here
there is a second way to be wrong that the reference case does not have: the paths
the handoff itself cites may no longer exist, so a name that was accurate when it was
written can now point at nothing. Hand a cartographer the wrong directory and you get
a confident, well-cited map of the wrong code, which is harder to catch than an empty
one.

**Reading the divergence.** `conventions.scenarios` lists the four shapes a resume
takes and what each one asks for; telling them apart is the call. Weigh how much time
has passed, how much of Recent changes survived verification, and whether the plan
the handoff was executing is still `active`. The handoff's Learnings are the last
thing to discard — a learning survives a refactor more often than a `file:line` does
— but where the tree contradicts the document the tree wins, and the contradiction is
itself a finding worth showing the user. Read a stale handoff as a clean continuation
and you implement against an architecture that is gone; read a clean continuation as
a stale handoff and you re-research work that was already done and paid for.

**Ordering the resumed work.** The handoff proposes an order, and verification may
have invalidated it: a task whose precondition shipped is now free, a task whose
precondition was reverted now has to go last. Prioritize on dependencies as they are
now, using the handoff's guidance as evidence rather than as the answer. Follow a
stale order and you start the task whose precondition no longer holds, and find out
three tasks in.

**Whether to hand off again.** Reference this handoff in commits and record any
deviation from the original plan as you go; then decide, at the end, whether what
this session learned needs its own handoff. The test is not how much you did but how
much of what you know is only in this context window. Skip it when it was needed and
the next session restarts from where this one started rather than from where it
ended.
