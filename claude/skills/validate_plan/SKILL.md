---
name: validate_plan
description: Validate that an implementation plan was correctly executed, run its automated success criteria, report deviations and remaining manual steps, and promote the plan's status frontmatter to `implemented` once fully verified. Use when the user asks to validate or audit an implementation against an existing plan.
allowed-tools: Bash, Read, Edit, Grep, Glob, Agent
allowed-agents: inspector
allowed-skills: hyprlayer_doctor
---

# Validate Plan

You are tasked with validating that an implementation plan was correctly executed, verifying all success criteria and identifying any deviations or issues.

## Storage backend dispatch

Read `~/.claude/skills/_thoughts/storage-backend.md` for the per-backend mechanics — see both the "How to read existing artifacts" and "How to update existing artifacts" sections. Read `~/.claude/skills/_thoughts/required-metadata.md` for legal `select` values (in particular for `status`). For this command: artifact type is `plan`. Reading and analysis are read-only, but the status promotion described in Step 4 below is an explicit, narrow exception: if validation concludes the plan is fully implemented — every phase checked off, every automated check passing, and no manual verification items still outstanding — promote `status` from `active` to `implemented` as the final step of this command, per the update rules in `storage-backend.md`. If any manual item is unconfirmed or any automated check fails, leave `status` untouched and say exactly what is blocking promotion.

For `notion`/`anytype`, checkboxes inside the plan body come back as toggle/to-do block children (Notion) or the body markdown (Anytype) — enumerate them to count done vs. pending.

## Initial Setup

When invoked:
1. **Determine context** - Are you in an existing conversation or starting fresh?
   - If existing: Review what was implemented in this session
   - If fresh: Need to discover what was done through git and codebase analysis

2. **Locate the plan**:
   - If plan path provided, use it
   - Otherwise, search recent commits for plan references or ask user

3. **Gather implementation evidence**:
   ```bash
   # Check recent commits
   git log --oneline -n 20
   git diff HEAD~N..HEAD  # Where N covers implementation commits

   # Run comprehensive checks
   cd $(git rev-parse --show-toplevel) && make check test
   ```

## Validation Process

### Step 1: Context Discovery

If starting fresh or need more context:

1. **Read the implementation plan** completely
2. **Identify what should have changed**:
   - List all files that should be modified
   - Note all success criteria (automated and manual)
   - Identify key functionality to verify

3. **Delegate the audit to the `inspector` agent** (see `~/.claude/skills/_thoughts/subagent-guide.md`). Hand it: the plan (path or body), the diff range or commits that implemented it, the repo root, and the report template path `~/.claude/skills/_thoughts/templates/validation-report.md`.

   It reads the plan, checks each claimed-complete phase against the actual diff, runs every automated success criterion, and returns the report plus `verdict: promote` or `verdict: block`. A fresh context is the point — an agent that did not write the code will not assume its own work is correct.

   **Delegate by default.** There is exactly one exception: you implemented this work yourself, in this same session, *and* the diff is small enough to re-read in full (roughly one phase, under ~100 changed lines). Both conditions, not either. "The diff looks small" on its own is not a reason to skip the fresh context — a small diff is cheap for the inspector too. If you do audit inline, hold yourself to the same bar: run every command, cite every claim with `file:line`.

   For a large plan, spawn one inspector per phase group rather than one for everything, and reconcile their reports yourself.

### Step 2: Systematic Validation

If an `inspector` ran, this step is verification of its report, not a repeat of it: spot-check the phases it marked complete against the diff yourself, and re-run any automated command whose reported result you have reason to doubt. Its `verdict:` is advice — the promotion decision in step 4 is yours.

For each phase in the plan:

1. **Check completion status**:
   - Look for checkmarks in the plan (- [x])
   - Verify the actual code matches claimed completion

2. **Run automated verification**:
   - Execute each command from "Automated Verification"
   - Document pass/fail status
   - If failures, investigate root cause

3. **Assess manual criteria**:
   - List what needs manual testing
   - Provide clear steps for user verification

4. **Think deeply about edge cases**:
   - Were error conditions handled?
   - Are there missing validations?
   - Could the implementation break existing functionality?

### Step 3: Generate Validation Report

Read `~/.claude/skills/_thoughts/templates/validation-report.md` for the report structure. Populate every section with concrete findings from steps 1 and 2.

### Step 4: Promote Plan Status (only when fully implemented)

Check the report you just produced against this bar:
- Every phase in the plan is checked off (`- [x]`) and the code backs that up.
- Every automated verification command passed.
- No manual verification item is still outstanding/unconfirmed.

If **all** of those hold:
1. Edit the plan artifact's `status` field from `active` to `implemented`, following the "How to update existing artifacts" rules in `storage-backend.md` for the active backend.
2. For `git`/`obsidian`, also bump `last_updated` (today's date) if the artifact already tracks that field — don't add it if the plan never had it.
3. For `git`, run `hyprlayer thoughts sync` so the promoted status is pushed.
4. Call this out explicitly in the report summary, e.g. "Plan status promoted: `active` → `implemented`."

If any of those don't hold, leave `status` exactly as it is — do not promote — and state plainly in the report what's blocking promotion (which check is failing, which manual item is unconfirmed) so a future run can pick it up once resolved.

## Working with Existing Context

If you were part of the implementation:
- Review the conversation history
- Check your todo list for what was completed
- Focus validation on work done in this session
- Be honest about any shortcuts or incomplete items

## Important Guidelines

1. **Be thorough but practical** - Focus on what matters
2. **Run all automated checks** - Don't skip verification commands
3. **Document everything** - Both successes and issues
4. **Think critically** - Question if the implementation truly solves the problem
5. **Consider maintenance** - Will this be maintainable long-term?

## Validation Checklist

Always verify:
- [ ] All phases marked complete are actually done
- [ ] Automated tests pass
- [ ] Code follows existing patterns
- [ ] No regressions introduced
- [ ] Error handling is robust
- [ ] Documentation updated if needed
- [ ] Manual test steps are clear
- [ ] Plan `status` frontmatter reflects the actual completion state (promoted to `implemented` only when fully done)

## Relationship to Other Commands

Recommended workflow:
1. `/implement_plan` - Execute the implementation
2. `/commit` - Create atomic commits for changes
3. `/validate_plan` - Verify implementation correctness and promote plan `status` to `implemented`
4. `/describe_pr` - Generate PR description (see its own status lifecycle in `_thoughts/pr-description.md`)

The validation works best after commits are made, as it can analyze the git history to understand what was implemented.

Remember: Good validation catches issues before they reach production. Be constructive but thorough in identifying gaps or improvements.
