---
name: implement_plan
description: Implement an existing technical plan, checking off items and verifying success criteria as work progresses. Side-effecting; user-only — never auto-invoke. Use when the user asks to implement a previously approved plan.
model: sonnet
allowed-tools: Bash, Read, Edit, Write, Grep, Glob, Agent
allowed-agents: foreman, cartographer, codebase-locator, codebase-analyzer, codebase-pattern-finder
allowed-skills: hyprlayer_doctor
disable-model-invocation: true
---

# Implement Plan

You are tasked with implementing an approved technical plan. These plans contain phases with specific changes and success criteria.

## Storage backend dispatch

Read `~/.claude/skills/_thoughts/storage-backend.md` for the per-backend mechanics — see the "How to read existing artifacts" and "How to update existing artifacts" sections. Read `~/.claude/skills/_thoughts/required-metadata.md` for legal `select` values (in particular for `status`). For this command: artifact type is `plan`; you read the plan, check off items as you go (`- [x]`), and once fully implemented promote `status` from `active` to `implemented` per the rules in those files.

## Getting Started

When given a plan path:
- Read the plan completely and check for any existing checkmarks (- [x])
- Read the original ticket and all files mentioned in the plan
- **Read files fully** - never use limit/offset parameters, you need complete context
- Think deeply about how the pieces fit together
- Create a todo list to track your progress
- Start implementing if you understand what needs to be done

If no plan path provided, ask for one.

## Delegating a phase to the `foreman`

Read `~/.claude/skills/_thoughts/subagent-guide.md` for the catalog. The default for a substantial phase is to delegate it: spawn a `foreman` with the plan path (or body), **which phase number is theirs**, and the repo root. It implements that phase in a fresh context, runs the phase's automated verification, and reports what changed, what passed, and what it could not do.

Delegate when the phase touches several files, spans a subsystem you'd otherwise have to load into this context, or is one of many phases you're working through in sequence. Implement inline when the phase is a handful of lines, when you already hold all the context the phase needs, or when the user is iterating with you interactively on that specific code.

Spawn foremen in parallel only when the phases are genuinely independent — no shared files, no ordering dependency. Plans are usually written so they aren't.

When a foreman returns:
- Read its report against the plan. Do not trust "phase complete" without the verification results to back it.
- A `MISMATCH` block is yours to resolve, not the foreman's — apply the "If you encounter a mismatch" flow below.
- You own the plan artifact: check off items (`- [x]`), keep the todos current, and promote `status` per the dispatch. The foreman never touches the plan file.

## Implementation Philosophy

Plans are carefully designed, but reality can be messy. Your job is to:
- Follow the plan's intent while adapting to what you find
- Implement each phase fully before moving to the next
- Verify your work makes sense in the broader codebase context
- Update checkboxes in the plan as you complete sections

When things don't match the plan exactly, think about why and communicate clearly. The plan is your guide, but your judgment matters too.

If you encounter a mismatch:
- STOP and think deeply about why the plan can't be followed
- Present the issue clearly:
  ```
  Issue in Phase [N]:
  Expected: [what the plan says]
  Found: [actual situation]
  Why this matters: [explanation]

  How should I proceed?
  ```

## Verification Approach

After implementing a phase:
- Run the success criteria checks (usually `make check test` covers everything)
- Fix any issues before proceeding
- Update your progress in both the plan and your todos
- Check off completed items in the plan file itself using Edit
- **Pause for human verification**: After completing all automated verification for a phase, pause and inform the human that the phase is ready for manual testing. Use this format:
  ```
  Phase [N] Complete - Ready for Manual Verification

  Automated verification passed:
  - [List automated checks that passed]

  Please perform the manual verification steps listed in the plan:
  - [List manual verification items from the plan]

  Let me know when manual testing is complete so I can proceed to Phase [N+1].
  ```

If instructed to execute multiple phases consecutively, skip the pause until the last phase. Otherwise, assume you are just doing one phase.

do not check off items in the manual testing steps until confirmed by the user.


## If You Get Stuck

When something isn't working as expected:
- First, make sure you've read and understood all the relevant code
- Consider if the codebase has evolved since the plan was written
- Present the mismatch clearly and ask for guidance

Use sub-tasks sparingly for debugging — a `cartographer` when the territory is genuinely unfamiliar, the narrow `codebase-*` agents for a single targeted question. Do not re-spawn a `foreman` on a phase that already failed the same way; work out why first.

## Resuming Work

If the plan has existing checkmarks:
- Trust that completed work is done
- Pick up from the first unchecked item
- Verify previous work only if something seems off

Remember: You're implementing a solution, not just checking boxes. Keep the end goal in mind and maintain forward momentum.
