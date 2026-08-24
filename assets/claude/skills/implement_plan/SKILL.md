---
name: implement_plan
description: Implement an existing technical plan, checking off items and verifying success criteria as work progresses. Side-effecting; user-only — never auto-invoke. Use when the user asks to implement a previously approved plan.
model: sonnet
allowed-tools: Bash, Read, Edit, Write, Grep, Glob, Agent, Skill
disable-model-invocation: true
---

# Implement Plan

Execute an approved technical plan one phase at a time: delegate each substantial
phase to a `foreman`, verify what comes back, and own every checkbox in the plan
file throughout. It does not promote the plan's `status`: that transition belongs
to `validate_plan`, which audits the finished work before writing it.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - storage-backend            # "How to read existing artifacts" / "How to update existing artifacts"
  - required-metadata          # legal `select` values, `status` in particular
  - subagent-guide             # the catalog behind `foreman` and the debug agents

on-empty-invocation: >
  I need the path to the plan you'd like me to implement. Please provide one.

artifact:
  type: plan
  mode: update-in-place        # this skill mutates an existing plan; it never writes a new one
  source: plan-path
  mutates: [checkboxes]        # every `- [ ]` / `- [x]` in the body, and nothing else
  promotes: none               # `status` stays `active`; `validate_plan` owns `active → implemented`

orchestration:
  owns:
    - plan-artifact            # the foreman never touches the plan file
    - checkbox-updates         # the whole of this skill's write access to it — see `status-ownership`
    - promotion-handoff        # saying the plan is ready to validate; not the promotion itself
    - todo-list
    - phase-sequencing
    - mismatch-resolution
    - human-verification-pause

  steps:
    - id: read-plan
      inline: true
      reject: matches(read-call, "limit|offset")
      because: >
        Read the plan completely, including any existing `- [x]` marks. A partial
        read is not a substitute for the whole file, and those marks decide where
        this run starts.

    - id: read-referenced
      requires: [read-plan]
      inline: true
      reject: matches(read-call, "limit|offset")
      because: >
        The original ticket and every file the plan mentions, read fully. Think
        about how the pieces fit together before touching any of them.

    - id: resume-point
      requires: [read-plan]
      inline: true
      judgment: >
        Does the plan already carry `- [x]` marks, and do those checkmarks describe
        the tree in front of you? See "Resuming a partly-implemented plan" below.
      because: >
        Runs on every invocation, because the marks you read in `read-plan` are what
        decide where this run starts: pick up from the first unchecked item and take
        completed work as done. On a plan with no checked box the answer is one line
        and you move on. There is deliberately no guard: the plan path arrives as an
        invocation argument, not as a shell variable, so `exit0(grep ... "$PLAN")`
        probes an empty path, comes back false, and skips the step in exactly the
        run it exists to catch.

    - id: track
      requires: [read-referenced]
      inline: true
      track-with: TodoWrite
      produces: phase-queue
      because: >
        One todo per phase, kept current for the whole run — the todos and the
        plan's checkboxes are updated together, never one without the other.

    - id: delegation-call
      requires: [track]
      inline: true
      produces: [delegated-phases, inline-phases]
      judgment: >
        Which phases go to a foreman and which do you implement yourself? See
        "Delegating a phase, or doing it yourself" below.
      because: >
        The default for a substantial phase is to delegate it.

    - id: independence-call
      requires: [delegation-call]
      inline: true
      produces: phase-batch
      judgment: >
        Are the phases you are about to dispatch genuinely independent — no shared
        files, no ordering dependency? See "Whether phases are genuinely
        independent" below.
      because: >
        `phase-batch` is the list `delegate-phase` fans out over, so this call is
        the parallelism. It is one phase unless you can argue otherwise: plans are
        usually written so their phases are not independent, and each phase is
        implemented fully before the next begins.

    - id: delegate-phase
      requires: [independence-call]
      fanout: foreman
      over: phase-batch
      given:
        - { value: plan-path-or-body, src: "the invocation argument, or the file read in read-plan" }
        - { value: phase-number,      src: "the phase headings in the plan you read" }
        - { value: repo-root,         src: pwd }
      ask: [what-changed, what-passed, what-it-could-not-do]
      reject: not matches(phase-number, "\d")
      on: { MISMATCH: resolve-mismatch }
      because: >
        A foreman implements exactly one phase in a fresh context and runs that
        phase's automated verification. The reject rule is the mechanical half of
        "which phase is theirs": a foreman handed a plan and no phase number has
        nothing bounding it to one phase.

    - id: implement-inline
      requires: [delegation-call]
      inline: true
      for-each: inline-phases
      because: >
        The other half of delegation-call: a handful of lines, context you already
        hold, or code the user is iterating on with you right now. Exactly one of
        `implement-inline` and `delegate-phase` runs for any given phase.

    - id: review-report
      requires: [delegate-phase]
      inline: true
      reject: not exists(verification-output)
      judgment: >
        Does the report hold up when you read it against the plan? See "Reading a
        foreman's report" below.
      because: >
        A sub-agent's report is input, not truth. "Phase complete" without the
        verification results behind it is a claim, not a result.

    - id: resolve-mismatch
      requires: [review-report, implement-inline]
      inline: true
      produces: mismatch-report
      judgment: >
        Did a foreman's report carry a `MISMATCH` block, or did you hit the same
        wall inline — and if so, why can the plan not be followed here, and is this
        a divergence you adapt to or one you stop for? See "When the plan and the
        code disagree" below.
      because: >
        Entered by the `MISMATCH` token in a foreman's report, or by hitting the
        same wall inline. A MISMATCH block is yours to resolve, not the foreman's:
        stop, work out why, present it in the `mismatch-report` format below, and
        ask how to proceed. The step schedules on every run and the first clause of
        the judgment is its real entry test — on a clean run you answer "no
        mismatch" in one line and move on. It is a judgment rather than a
        `when: matches(report, "MISMATCH")` because at compile time the invocation's
        request text is bound to every `matches()` field regardless of its name, so
        that guard would test the user's request against `MISMATCH`, not the report.

    - id: verify-phase
      requires: [review-report, implement-inline]
      inline: true
      run: phase-automated-verification    # usually `make check test` covers everything
      because: >
        Run the phase's success criteria yourself even when a foreman reported them
        green — you own what the plan claims. Then check the change makes sense in
        the broader codebase, which no success-criteria command tests.

    - id: fix-failures
      requires: [verify-phase]
      inline: true
      retry: { step: verify-phase, max: 3 }
      because: >
        Fix what failed before proceeding; never carry a red check into the next
        phase. A no-op on a green run.

    - id: debug
      requires: [fix-failures]
      agent: one-of [cartographer, codebase-locator, codebase-analyzer, codebase-pattern-finder]
      given:
        - { value: repo-root,   src: pwd }
        - { value: the-failure, src: "the output of the verification you ran in verify-phase" }
      judgment: >
        Is anything genuinely blocked, and is the territory unfamiliar enough for a
        cartographer or is this one targeted question for a narrow `codebase-*`
        agent? See "Reaching for a sub-agent when stuck" below.
      because: >
        Sub-tasks are for debugging sparingly. First make sure you have read and
        understood the relevant code, and consider whether the codebase has evolved
        since the plan was written. There is deliberately no `retry:` back to
        `delegate-phase` here: a bounded re-spawn is a *permission*, and the rule is
        a prohibition — do not re-spawn a foreman on a phase that already failed the
        same way, work out why first.

    - id: check-off
      requires: [fix-failures, debug]
      inline: true
      apply: [checkbox-edit]
      updates: [plan-checkboxes, todos]
      reject: matches(item, "manual")
      because: >
        Automated items only, edited into the plan file itself. Manual-testing
        items wait for `check-off-manual`. It requires `fix-failures` rather than
        `verify-phase` because the order in the plan is run the checks, fix what
        failed, *then* check the boxes: required against `verify-phase` alone, this
        step would land in the same wave as `fix-failures` and mark a phase done
        while its red check was still being fixed. `debug` is in the barrier for the
        same reason one wave further out — no box gets marked, and no "automated
        verification passed" notice gets emitted downstream, while an agent is still
        out investigating that phase's failure. Both are unguarded, so both really
        do run and really do order this step.

    - id: pause-for-manual
      requires: [check-off]
      when: not matches(request, "all phases|every phase|all remaining phases|consecutively|straight through|phases \d+ *(-|to|through) *\d+")
      when-examples:
        match:    ["implement phase 2 of the plan", "implement thoughts/plans/eng-1478.md"]
        no-match: ["implement all phases", "run phases 1-4 consecutively", "do every phase straight through"]
      inline: true
      produces: phase-complete-notice
      because: >
        Automated verification passing is not the phase being done. Emit the
        `phase-complete-notice` format below and stop. Absent an instruction to
        run several phases, assume you are doing one; on a consecutive run the
        pause comes once, after the last phase, not after each.

    - id: check-off-manual
      requires: [check-off, pause-for-manual]
      inline: true
      updates: [plan-checkboxes]
      because: >
        Manual-testing items get checked off only once the user confirms the
        testing is done. Their barrier is the user's reply, which is why this is a
        step of its own rather than part of `check-off`. It requires `check-off`
        as well as the pause, so that skipping the pause on a consecutive run
        moves this step rather than freeing it.

    - id: hand-off-status
      requires: [check-off-manual]
      inline: true
      produces: promotion-handoff
      leaves: { status: active }
      defers: { status-promotion: validate_plan }
      because: >
        This skill does not promote `status`. Every checkbox in the file is
        this skill's; the `status` line is `validate_plan`'s — see
        `conventions.status-ownership`. Promoting here would be
        self-certification, since the only evidence is the marks this skill just
        wrote, and it would make `validate_plan`'s three-part gate unreachable:
        running second, it would find `status` already `implemented` and its
        promotion a no-op that still stamps `last_updated` and syncs. No guard,
        because it always runs — and because the guard it used to carry could
        not resolve anyway: nothing binds `count(unchecked-items)`, so it
        evaluated `unknown`, and `unknown` skips.

conventions:

  status-ownership:
    transition: active -> implemented
    owner: validate_plan             # the only skill that writes it — see its `promote` step
    this-skill: leaves `status` at `active`, and says the plan is ready to validate
    why: >
      Both skills used to declare this same mutation and neither named the other.
      It belongs to `validate_plan`: its frontmatter description promises it, and
      its gate measures the plan's checkboxes against an independent diff and an
      independent test run by an agent that did not write the code. The gate
      available here could only re-read the boxes this skill had just ticked, and
      promoting first made the stronger gate unreachable. One writer, and it is
      the one that audits. The cost is deliberate: a plan implemented and never
      validated stays `active`, because `implemented` is meant to mean audited.

  checkbox-edit:
    tool: Edit
    target: the plan file itself
    mark: "- [x]"
    manual-items: not until the user confirms

  mismatch-report:
    trigger: the plan cannot be followed as written
    stop-first: true
    format: |
      Issue in Phase [N]:
      Expected: [what the plan says]
      Found: [actual situation]
      Why this matters: [explanation]

      How should I proceed?

  phase-complete-notice:
    format: |
      Phase [N] Complete - Ready for Manual Verification

      Automated verification passed:
      - [List automated checks that passed]

      Please perform the manual verification steps listed in the plan:
      - [List manual verification items from the plan]

      Let me know when manual testing is complete so I can proceed to Phase [N+1].
```

## Judgment

**Delegating a phase, or doing it yourself.** Delegate when the phase touches several
files, spans a subsystem you would otherwise load into this context, or is one of many
you are working through in sequence. Do it inline when it is a handful of lines, when
you already hold the context, or when the user is iterating with you on that code.
Neither test is checkable from the plan: "substantial" is about how much context the
phase costs *you*, and only you can see how full this one is. Delegate a five-line
phase and you pay an agent to re-read the plan; hold a subsystem-wide phase inline and
you crowd out the phases after it.

**Whether phases are genuinely independent.** The size of `phase-batch` *is* the
parallelism decision, and nothing mechanical can make it: phases do not declare the
files they touch, and their ordering is usually implicit in the design. No command
returns "these two do not collide", which is why this is a `judgment:` and not a
`when:`. Default the batch to one. Two foremen in one file means last write wins, and
the plan will say both phases are done.

**Resuming a partly-implemented plan.** Trust completed work, pick up from the first
unchecked item, and verify earlier work only if something seems off. Re-verifying
everything burns the run; trusting a checkmark left by a session interrupted mid-phase
builds every later phase on half a phase.

**Reading a foreman's report.** Input, not truth — read it against the plan rather than
accepting its summary. The `reject` rule catches a report with no verification output,
not one whose output fails to support what it claims. Check off a phase that never ran
green and the failure surfaces later, inside code you did not write.

**When the plan and the code disagree.** Adapt to small divergences — the plan says
`foo.rs:42` and the function moved to `:58` — noting them as you go. Stop for anything
that changes behavior, contract, or scope: work out *why* the plan cannot be followed,
then present it and ask. Improvise a redesign instead and you have shipped an
unreviewed plan; the plan was reviewed, your workaround was not.

**Reaching for a sub-agent when stuck.** A `cartographer` for genuinely unfamiliar
territory, a narrow `codebase-*` agent for one targeted question. First make sure you
have read the relevant code, and consider whether the codebase moved since the plan was
written — a stale plan looks exactly like a broken implementation. Never re-spawn a
foreman on a phase that failed the same way; that is why there is no `retry:` back to
`delegate-phase`. A second identical spawn burns a context to reproduce a failure you
already have.
