---
name: validate_plan
description: Validate that an implementation plan was correctly executed, run its automated success criteria, report deviations and remaining manual steps, and promote the plan's status frontmatter to `implemented` once fully verified. Use when the user asks to validate or audit an implementation against an existing plan.
allowed-tools: Bash, Read, Edit, Grep, Glob, Agent, Skill
---

# Validate Plan

Audit a finished implementation against the plan that specified it: check every
claimed-complete phase against the actual diff, run every automated success
criterion, and promote the plan's `status` to `implemented` only when nothing is
left outstanding. This skill is the sole writer of that transition — `implement_plan`
finishes with the plan still `active` and hands it here.

```yaml
loads:
  - orchestration-runtime        # how to execute this block — read before anything
  - storage-backend              # both "How to read existing artifacts" and "How to update existing artifacts"
  - required-metadata            # legal `select` values, in particular for `status`
  - subagent-guide               # the inspector's catalog entry and spawning rules
  - templates/validation-report  # the report structure

artifact:
  type: plan
  reads: existing                # located, never created
  mutates: [status]              # the one write in an otherwise read-only skill
  status-transition: active -> implemented
  transition-owner: validate_plan  # sole writer; `implement_plan` owns the checkboxes and leaves `status` alone

orchestration:
  owns: [promotion-decision, artifact-mutation, sync, report]

  steps:
    - id: context
      inline: true
      produces: [session-role, evidence-mode]
      because: >
        Fresh session, or the one that did the work? Fresh means discovering
        what was done from git and the codebase. Part of this session means
        you already know it — review the conversation and your todo list,
        focus on the work done here, and be honest about shortcuts taken and
        items left incomplete. Which it is changes how you gather evidence,
        never whether you verify it.

    - id: locate-plan
      requires: [context]
      inline: true
      produces: plan-ref
      because: >
        A path in the invocation wins. Otherwise search recent commits for a
        plan reference; if that turns up nothing, ask. Validating against a
        plan you guessed at reports confidently on the wrong work.

    - id: read-plan
      requires: [locate-plan]
      inline: true
      reject: matches(read-call, "limit|offset")
      because: >
        Completely, in your own context, before anything is delegated: every
        checkbox and its current state, every success criterion, every file
        the plan says should have changed. A `- [x]` is a claim, not
        evidence, and you cannot weigh a claim you only skimmed.

    - id: inventory
      requires: [read-plan]
      inline: true
      produces: [files-expected, automated-criteria, manual-criteria, phase-checkboxes, key-functionality]
      checkbox-source:
        - { backend: "git, obsidian", from: "`- [ ]` / `- [x]` lines in the plan body" }
        - { backend: notion,          from: "toggle / to-do block children — enumerate them to count done vs pending" }
        - { backend: anytype,         from: "the body markdown — enumerate it the same way" }
      because: >
        This inventory is what the audit gets measured against: which files
        should have changed, which criteria are automated and which are
        manual, and which phases claim to be done. `key-functionality` is the
        behaviour half of that — what the change is supposed to do, as
        distinct from the criteria written down about it — and it is gathered
        here rather than later because it bounds what the inspectors are told
        to look for. It is also the phase list the `audit` fan-out is cut
        from.

    - id: evidence
      requires: [locate-plan]
      inline: true
      produces: [diff-range, baseline-check-result]
      run:
        - git log --oneline -n 20
        - git diff HEAD~N..HEAD    # N covers the implementation commits
        - cd $(git rev-parse --show-toplevel) && make check test
      because: >
        Validation works best after commits are made, because the git history
        is what says which change belongs to this plan. The baseline run is
        also the number the inspector's automated results get checked
        against, and it tells you whether a red check is this diff's fault or
        was already red.

    - id: audit
      requires: [inventory, evidence]
      fanout: inspector
      over: phase-groups
      given:
        - { value: plan-ref,          src: "locate-plan — the path, or the body for notion/anytype" }
        - { value: diff-range,        src: "the commits identified in evidence" }
        - { value: repo-root,         src: "git rev-parse --show-toplevel" }
        - { value: report-template,   src: "_thoughts/templates/validation-report.md" }
        - { value: phase-group,       src: "the phase split you chose in inventory" }
        - { value: key-functionality, src: "the behaviour list inventory produced" }
      ask: [phase-results, automated-results, manual-candidates, deviations, gaps-and-risks, verdict]
      reject: not exists(plan-ref) or not exists(diff-range)
      judgment: >
        Do you delegate this audit at all, and as one inspector or one per
        phase group? See "Delegating the audit" below.
      because: >
        A fresh context is the point — an agent that did not write the code
        will not assume its own work is correct. Each inspector reads the
        plan, checks its phases' claims against the diff, runs every
        automated criterion in them, and returns the report plus
        `verdict: promote` or `verdict: block`. One group is the normal
        case; `over: phase-groups` exists so a large plan can be split
        without changing the shape of the step.

    - id: verify-report
      requires: [audit]
      inline: true
      retry: { step: audit, max: 1 }
      judgment: >
        Which of the inspector's claims do you doubt enough to check
        yourself? See "Verifying the audit, not repeating it" below.
      because: >
        A sub-agent's report is input, not truth. Spot-check the phases it
        marked complete against the diff, re-run any automated command whose
        reported result you have reason to doubt, and where more than one
        inspector ran, reconcile their reports here. A check reported red is
        investigated, not merely counted: find the root cause, so the report
        names what broke rather than only that something did. Its `verdict:`
        is advice; the gate on `promote` is yours.

    - id: edge-cases
      requires: [verify-report]
      inline: true
      judgment: >
        Beyond the plan's own criteria — were error conditions handled, are
        validations missing, could this break existing functionality, and
        does it actually solve the problem it was written for? See "Looking
        past the checklist" below.

    - id: manual-items
      requires: [verify-report]
      inline: true
      produces: manual-outstanding
      because: >
        Every manual verification item still unconfirmed, quoted from the
        plan, with steps clear enough for a human to follow. Neither you nor
        the inspector may confirm one on a human's behalf, and an unconfirmed
        item blocks promotion — so this list is a gate input, not a
        courtesy section. The inspector's `manual-candidates` is named apart
        from it deliberately: `manual-outstanding` is the leaf the `promote`
        gate counts, and this step is its only producer.

    - id: report
      requires: [edge-cases, manual-items]
      inline: true
      given:
        - { value: template, src: "_thoughts/templates/validation-report.md" }
        - { value: findings, src: "the verified inspector reports plus your own spot-checks" }
      sections: [implementation-status, automated-verification-results, code-review-findings, manual-testing-required, recommendations]
      reject: exists(unresolved-placeholder)
      because: >
        Every applicable section populated with concrete findings from the
        audit and your verification — successes and issues both, each claim
        carrying a `file:line`. The template ships bracketed placeholders; one
        still in the report is a section nobody filled in.

    - id: promote
      requires: [report]
      inline: true
      updates: [status]
      writes-transition: active -> implemented   # the one place in the skill set this is written
      gate: count(phases-not-complete) == 0 and count(failed-checks) == 0 and count(manual-outstanding) == 0
      counts:
        phases-not-complete: plan phases either unchecked or not backed by the diff in the verified report
        failed-checks:       automated verification commands from any phase that did not pass
        manual-outstanding:  manual items still unconfirmed — see "Resolving the promotion gate" below
      judgment: >
        Are all three counts in `gate:` zero — and did you resolve each one, as
        opposed to failing to find it? See "Resolving the promotion gate" below.
      on-promote:
        - { edit: status, from: active, to: implemented, per: "storage-backend.md, How to update existing artifacts" }
        - { backend: "git, obsidian", edit: last_updated, to: today, only-if: "the artifact already tracks the field — never add it" }
        - { backend: git, run: hyprlayer thoughts sync }
      on-block: >
        Leave `status` exactly as it is and change nothing else — no
        `last_updated` bump, no sync — then name what is blocking.
      because: >
        This skill owns the promotion outright — `implement_plan` ticks the
        checkboxes and leaves `status` `active` — so the audit is what the word
        `implemented` rests on. `gate:` is the whole decision and has no taste in
        it, but it is skill data rather than a `when:` because all three counts
        come from the report *this run* produces: as a guard it evaluated
        `unknown`, and `unknown` skips, so the one step this skill exists to
        reach never ran. Fail closed by hand — a count you could not resolve is
        not zero. The bump and the sync stay inside this step so a run that does
        not promote cannot stamp `last_updated` or sync an unmoved plan.

    - id: summary
      requires: [report, promote]
      inline: true
      because: >
        Call the outcome out explicitly, in the report summary and in the
        message: "Plan status promoted: `active` → `implemented`", or exactly
        what is blocking — which check failed, which manual item is
        unconfirmed — so a future run can pick it up once resolved. `promote`
        carries no guard, so it always runs and this step genuinely lands after
        it; `report` stays in the barrier because the summary is written into
        that report, and because requiring a step that could skip is the same as
        requiring nothing. A blocked promotion nobody mentions reads as a
        successful one — and since `implement_plan` no longer promotes, a
        blocked plan stays `active` until someone re-runs this skill.

conventions:

  always-verify:
    - phases marked complete are actually done
    - automated tests pass
    - code follows existing patterns
    - no regressions introduced
    - error handling is robust
    - documentation updated if needed
    - manual test steps are clear
    - plan `status` reflects the actual completion state

  citations:
    every-claim: file:line
    primary-source: the diff and the working tree
    secondary-source: the plan's own claims

  workflow:
    recommended-order: [create_plan, implement_plan, commit, validate_plan, describe_pr]
    this-skill: validate_plan          # after the commits, because validation reads git history
    promotes-status: validate_plan     # `active -> implemented`; every other skill in the order leaves it alone
    describe_pr: has its own status lifecycle — _thoughts/pr-description.md
    note: >
      A recommended order, not a declared graph edge. This used to be written as
      `before:`/`after:` here and nowhere else, which is half of a relationship the
      other four skills had never heard of — five copies would be five things to
      keep in sync and nothing in the runtime reads any of them. So the order is
      documentation, stated once, in the skill whose prose already documented it.
      The one part of it that binds is who writes `status`, and that is not carried
      by this list: it is written into the `promote` step here and into
      `implement_plan`'s `hand-off-status` step and `status-ownership` convention,
      so a reader of either file lands on the same answer.
```

## Judgment

**Delegating the audit.** Delegate by default. The one exception takes both halves:
you implemented this work yourself, in this session, *and* the diff is small enough to
re-read in full (~one phase, under 100 lines). "It looks small" alone is not a reason —
a small diff is cheap for the inspector too. Audit inline and you are asking the context
that produced the code to find what it missed, which is what that context is worst at.
On a large plan, one inspector per phase group: split too fine and none of them sees a
regression crossing phases; hand one everything and you get a shallow pass.

**Verifying the audit, not repeating it.** Re-run everything and the fresh context was
wasted. Re-run nothing and the `verdict:` has quietly become the decision. Spot-check
the phases marked complete, and re-run the specific command you have reason to doubt — a
suspiciously fast pass, output summarized instead of pasted, evidence citing a file the
diff never touched. `retry: {step: audit, max: 1}` is for when the report is simply
wrong: spawn a fresh inspector rather than averaging two you trust less than either.

**Looking past the checklist.** The plan's criteria are a floor. Were error conditions
handled or only the happy path? Validations the plan never thought to ask for? Did the
diff break something it touched incidentally? An implementation can satisfy every
criterion and still leave the feature broken — a test added that never runs, a flag
wired but never read — and none of that shows up as a red check.

**Resolving the promotion gate.** Three counts, no taste; the taste is in getting them
right and in admitting when you could not. Fail closed: a count you could not resolve is
not zero. `manual-outstanding` is the sharp edge — a `- [x]` beside a manual item means
a human said they ran it, and nothing else does. Neither the inspector nor you may check
one off; if nobody in this conversation confirmed it, it is outstanding. Promote on an
unperformed manual item and the plan reads `implemented` while its user-facing half has
never been tried, and the next run trusts that status instead of re-checking. Blocking
costs something real: this is the only skill that writes `active -> implemented`, so a
blocked plan stays `active` until someone re-runs this skill. That is intended —
`implemented` means audited, not merely finished.
