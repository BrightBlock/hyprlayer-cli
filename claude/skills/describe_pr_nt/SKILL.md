---
name: describe_pr_nt
description: Generate comprehensive PR descriptions using a built-in template, no-thoughts variant (writes to the OS temp directory instead of thoughts/shared/prs). Use when the user asks to describe a PR in a repo without a thoughts/shared/pr_description.md template.
allowed-tools: Bash, Read
---

# Generate PR Description

Fill a built-in PR description template from a pull request's diff and commits,
run whatever verification the template asks for, and push the finished body to
the PR. The no-thoughts variant: no backend, no record, one temp file.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything

template: |                    # built-in; this variant reads no repository template
  ## What problem(s) was I solving?

  ## What user-facing changes did I ship?

  ## How I implemented it

  ## How to verify it

  ### Manual Testing

  ## Description for the changelog

orchestration:
  owns: [pr-identification, diff-analysis, verification-execution, the-write-up, persistence, pr-edit]

  steps:
    - id: read-template
      inline: true
      given: [{ value: template, src: "the `template:` key above, verbatim" }]
      because: >
        Read it carefully to understand all sections and requirements. This
        variant carries its own template rather than resolving one per backend;
        `describe_pr` is the variant that reads
        `thoughts/shared/pr_description.md`. There is nothing to locate here,
        so there is nothing that can fail to be located.

    - id: identify-pr
      inline: true
      run: "gh pr view --json url,number,title,state 2>/dev/null"
      produces: [pr-number, pr-title, pr-state]
      because: >
        The current branch's PR is the default subject. The probe is silenced
        because a branch with no PR is an ordinary case, not an error.

    - id: choose-pr
      requires: [identify-pr]
      when: not exit0(gh pr view --json number)
      when-examples:
        match:    ["the current branch has no associated PR", "HEAD is on main"]
        no-match: ["the current branch's PR is #412"]
      inline: true
      run: "gh pr list --limit 10 --json number,title,headRefName,author"
      because: >
        No PR on this branch — or sitting on main/master — means the subject is
        ambiguous. List the open PRs and ask the user which one they want
        described. Never pick for them.

    - id: prior-record
      requires: [identify-pr, choose-pr]
      inline: true
      given: [{ value: record-path, src: "the `paths.record` convention below" }]
      because: >
        If a prior description already exists at that path, read it, tell the
        user you will be updating it, and consider what has changed since it
        was written.

    - id: gather
      requires: [identify-pr, choose-pr]
      inline: true
      run:
        - "gh pr diff {number}"
        - "gh pr view {number} --json url,number,title,state,baseRefName,commits"
      reject: not exit0(gh repo set-default --view)
      because: >
        Two calls, never a fan-out of separate `gh pr view` invocations — one
        for the diff, one for every metadata field at once. If either call
        errors with "no default remote repository", tell the user to run
        `gh repo set-default` and pick the right one.

    - id: analyze
      requires: [gather]
      inline: true
      judgment: >
        Which of these changes is user-facing and which is implementation
        detail? See "Sorting user-facing from internal" below.
      because: >
        Read the diff itself; open an adjacent file only when a hunk is
        genuinely ambiguous without it. Pre-emptively loading surrounding
        context is how a one-line PR costs a large one's budget. Breaking
        changes and migration requirements are what you hunt for first — they
        belong in the description prominently, not in a footnote.

    - id: deep-analysis
      requires: [analyze]
      inline: true
      judgment: >
        Is this diff big enough that its architectural implications are worth
        thinking through — more than 10 files changed, or more than 300 added
        plus deleted lines? Below that, spend nothing here.
      because: >
        `diff-lines` is added plus deleted. Above the threshold, think hard
        about architectural implications; below it, skip the extended
        reasoning — it is wasted time on typo fixes and one-liners. The
        threshold is not a `when:` because nothing binds `count(files-changed)`
        or `count(diff-lines)` for the compiler; such a guard evaluates unknown
        and would skip this step on every PR, the 900-line ones included. So
        the step schedules every time and the size question is yours, answered
        once `analyze` has the diff in hand.

    - id: verify-plan
      requires: [read-template, analyze]
      inline: true
      produces: automatable-checks
      judgment: >
        Which items in the template's "How to verify it" section can this
        session actually run, and which are manual-only? See "Classifying a
        verification item" below.

    - id: verify
      requires: [verify-plan]
      inline: true
      judgment: >
        Did `verify-plan` classify anything as automatable? If nothing did,
        say so and run nothing; if something did, ask before running a single
        one. See "Classifying a verification item" below.
      ask-user: >
        Run the automatable verification commands now, or skip and leave them
        for you to verify?
      on:
        run:  "execute each command; tick `- [x]` on pass, leave `- [ ]` plus a brief note of what failed"
        skip: 'leave the automatable boxes unchecked and add a one-line note in "How to verify it" that verification was deferred to the user'
      because: >
        Ask and wait for the answer before proceeding; never auto-run a
        verification command. Manual-only steps — UI interactions, external
        services — stay unchecked either way. The built-in template ships "How
        to verify it" empty, so the checks that exist are the ones you added
        while filling the section in, not ones the template handed you — which
        is why the count cannot be a guard. `count(automatable-checks)`
        resolves for nobody but this run, so as a `when:` it would evaluate
        unknown and skip the ask even with commands waiting to be run.

    - id: generate
      requires: [verify, prior-record, deep-analysis]
      inline: true
      because: >
        Written here, never delegated — `allowed-tools` for this variant is
        `Bash, Read`, so there is no Agent tool to spawn a herald with. Fill
        out each section from the template thoroughly against the `body:`
        conventions below.

    - id: verify-body
      requires: [generate]
      inline: true
      reject: exists(unfilled-template-section)
      because: >
        Every section from the template is answered and every checklist item is
        either checked or explained before this goes anywhere. Nothing else
        reviews this draft — the herald and its `BLOCKER:` protocol belong to
        `describe_pr`, so this self-check is the only gate the body passes
        through.

    - id: persist
      requires: [verify-body]
      inline: true
      given: [{ value: record-path, src: "the `paths.record` convention below" }]
      because: >
        One file, doing double duty: it is both the record and the
        `--body-file` input to `gh pr edit`, so it is body only — never YAML
        frontmatter, which GitHub renders as literal text. Create the parent
        directories first. Show the user the generated description.

    - id: edit-pr
      requires: [persist]
      inline: true
      run: "gh pr edit {number} --body-file <record-path>"
      because: >
        Update the PR description directly from the file just written, then
        confirm the update was successful.

    - id: report
      requires: [edit-pr]
      inline: true
      because: >
        If any verification steps remain unchecked, name them and remind the
        user to complete them before merging. Say clearly which ones need
        manual testing.

conventions:

  paths:
    record: "${TMPDIR:-${TEMP:-/tmp}}/{repo_name}/prs/{number}_description.md"
    doubles-as: "the --body-file argument to gh pr edit"
    frontmatter: never

  body:
    every-section:      answered-from-the-analysis
    problems:           specific
    changes:            specific
    user-impact:        called-out-where-relevant
    technical-detail:   in-the-section-that-fits-it
    changelog:          one-concise-entry
    checklist:          every-item-checked-or-explained
    breaking-changes:   prominent
    multi-component-pr: organize-by-component
    tone:               thorough-but-scannable
    weight:             the-why-as-much-as-the-what

  staging:
    never-add: [thoughts, "thoughts/**"]
    never-run: ["git add .", "git add -A"]
    because: >
      `thoughts/` may hold symlinks into a separate repo. If a commit is needed
      in the project repo, stage explicit file paths only. Nothing this skill
      writes lives in the repo anyway — the description goes to a temp path.
```

## Judgment

**Classifying a verification item.** An item under "How to verify it" is automatable
only if running it here is both possible and harmless. `npm test` is; a step that
posts to a live external service, or asks someone to click through a UI, is not. Get
it wrong in the permissive direction and this skill fires a real request at a real
system on the user's behalf; get it wrong in the conservative direction and the PR
ships with `- [ ] npm test` next to a suite that passes in one second. Nothing
outside this run can count that list, so `verify` carries no guard at all: it
schedules every time, and deciding there is nothing worth asking about is part of
the call. This variant's template arrives with the section empty, so every item in
it is one you wrote and then have to judge.

**Sorting user-facing from internal.** The template splits "What user-facing changes
did I ship?" from "How I implemented it", and the diff does not label which is
which — a refactor with a changed default is user-facing; a new public method nobody
calls yet is not. Put an implementation detail in the user-facing section and
reviewers weigh a change nobody will notice; bury a behavior change in the
implementation section and it ships unreviewed. When the PR touches several
components, organize within each section by component rather than by file.

**Writing without a second reader.** `describe_pr` gets a herald's draft and then
audits it; here the draft and the audit are the same pass, by the same context that
just read the diff and is primed to think it understood it. Read `verify-body`
against the template, section by section, as if someone else had written the body —
an unanswered section here reaches the PR, because `gh pr edit` in the next step
publishes the file exactly as it stands.
