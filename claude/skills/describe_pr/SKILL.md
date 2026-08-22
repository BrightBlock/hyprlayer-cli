---
name: describe_pr
description: Generate comprehensive PR descriptions following the repository's pr_description.md template. Use when the user asks to describe an existing PR (including running its automated verification commands and updating the PR via gh pr edit).
model: sonnet
allowed-tools: Bash, Read, Write, Agent, Skill, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# Generate PR Description

Fill the repository's PR description template from a pull request's diff and
commits, run whatever verification the template asks for, and push the finished
body to the PR itself.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - pr-description             # per-backend template, record and scratch-file locations
  - required-metadata          # the schema fields the record must carry
  - subagent-guide             # what the herald expects to be handed

artifact:
  type: pr
  title: "PR #{number}: {pr_title}"   # overrides the generic title-format rule

orchestration:
  owns: [pr-identification, verification-execution, persistence, status-promotion, pr-edit, cleanup]

  steps:
    - id: resolve-backend
      inline: true
      given: [{ value: backend, src: "hyprlayer storage info --json" }]
      produces: [template-path, record-path, scratch-path]
      because: >
        Every later location — template, record, scratch file — is a row of the
        dispatch table keyed on this one value. If the `hyprlayer` binary is
        unavailable or the project is not mapped, fall back to the `git` row.

    - id: read-template
      requires: [resolve-backend]
      inline: true
      reject: not exists(template)
      because: >
        This skill runs across different repositories, so the template is read
        locally on every run and never remembered. Read it carefully — its
        sections and its checklist are the whole contract for the body. On
        `notion`/`anytype` a template that cannot be located is a stop: tell
        the user to create a page or object named exactly
        `PR Description Template`. Never fall back to a hardcoded template:
        a body written to the wrong contract is worse than a stop the user
        can act on.

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
      requires: [identify-pr, choose-pr, resolve-backend]
      inline: true
      because: >
        The dispatch's "locate any prior record" step, keyed on the PR number.
        If a prior version is found, say you will update it rather than create
        a duplicate, and read it — what has changed since it was written is
        part of what this run has to describe.

    - id: gather
      requires: [identify-pr, choose-pr]
      inline: true
      run:
        - "gh pr diff {number}"
        - "gh pr view {number} --json url,number,title,state,baseRefName,commits"
      reject: matches(gh-output, "no default remote repository")
      because: >
        Two calls, never a fan-out of separate `gh pr view` invocations — one
        for the diff, one for every metadata field at once. The `state` fetched
        here is the source of truth for the whole status lifecycle. The reject
        reads the output of these two calls, because that is where the failure
        shows up: `gh repo set-default --view` exits 0 and prints "No default
        remote repository has been set", so an `exit0()` probe of it cannot see
        the condition at all. If either call errors with "no default remote
        repository", stop and tell the user to run `gh repo set-default` and
        pick the right one.

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
      produces: [architectural-implications]
      judgment: >
        Is this diff big enough — more than 10 files changed, or more than 300
        added-plus-deleted lines — to be worth extended reasoning about
        architectural implications?
      because: >
        `diff-lines` is added plus deleted. Above the threshold, think hard
        about architectural implications; below it, say you are skipping the
        extended reasoning and move on — it is wasted time on typo fixes and
        one-liners. Nothing counts the files or the lines for you, so this is
        a threshold you apply to the diff you just read, not a guard: written
        as `count(files-changed) > 10 or count(diff-lines) > 300` it resolves
        `unknown` and skips the step on every run, including the large PRs it
        was written to catch.

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
        Did `verify-plan` classify anything as automatable? With an empty list —
        or a template with no "How to verify it" section — there is nothing to
        ask about: say you are skipping the question and leave every box for
        the user.
      ask-user: >
        Run the automatable verification commands now, or skip and leave them
        for you to verify?
      on:
        run:  "execute each command; tick `- [x]` on pass, leave `- [ ]` plus a brief note of what failed"
        skip: 'leave the automatable boxes unchecked and add a one-line note in "How to verify it" that verification was deferred to the user'
      because: >
        Ask and wait for the answer before proceeding; never auto-run a
        verification command. Manual-only steps — UI interactions, external
        services — stay unchecked either way. `automatable-checks` exists only
        in `verify-plan`'s own reasoning, so whether the list is empty is a
        call you make here: as `count(automatable-checks) > 0` it resolves
        `unknown` and the step never runs, taking the user's choice with it.

    - id: generate
      requires: [analyze, deep-analysis, verify, prior-record]
      agent: herald
      given:
        - { value: pr-number,            src: "gh pr view --json number" }
        - { value: template,             src: "the file read in read-template — path or body" }
        - { value: repo-root,            src: pwd }
        - { value: commands-and-results, src: "the verification you ran in verify" }
      ask: [finished-body]
      judgment: >
        Is this PR trivial enough — one file, no ambiguity — to write inline
        instead of spawning a herald? See "Delegating the write-up" below.
      because: >
        The herald reads the diff and the commits itself and returns the
        finished body. Keeping a large diff out of this context is most of the
        value. It ticks a verification box only for commands you told it
        passed, so what you hand it about `verify` is what it is entitled to
        claim. `analyze` is required directly, not through `deep-analysis` or
        `verify`: routing the ordering through another step would let the
        herald spawn in the same wave as the analysis it is written from.

    - id: verify-body
      requires: [generate]
      inline: true
      retry: { step: generate, max: 1 }
      reject: matches(returned-body, "^BLOCKER:")
      judgment: >
        Does the returned checklist match what actually ran, and did anything
        caller-facing ride along with the body? See "Trusting the herald's
        checklist" below.
      because: >
        What the herald returns is the body, whole and unedited — it goes
        straight to `gh pr edit --body-file`, so whatever is in it is what
        reviewers read. A `BLOCKER:` line is not a body: resolve what it names
        and re-spawn. Strip anything caller-facing before persisting; never
        persist or publish it.

    - id: persist
      requires: [verify-body]
      inline: true
      given:
        - { value: scratch-path, src: "the dispatch table's scratch-file column for this backend" }
        - { value: record-path,  src: "the dispatch table's record column for this backend" }
        - { value: pr-state,     src: "gh pr view --json state, fetched in gather" }
      apply: [required-metadata, status-lifecycle]
      reject: matches(scratch-file, "^---")
      because: >
        Two writes, not one. The scratch file is body only — template sections,
        never YAML frontmatter or any metadata block — because GitHub renders
        frontmatter as literal text. The record carries the frontmatter on
        `git`/`obsidian`, or typed properties instead of a body header on
        `notion`/`anytype`. Show the user the generated description here.

    - id: sync
      requires: [persist]
      when: backend == git
      when-examples:
        match:    ["backend == git"]
        no-match: ["backend == obsidian", "backend == notion", "backend == anytype"]
      inline: true
      run: hyprlayer thoughts sync
      because: >
        On `git` the record is committed and then synced. `obsidian` writes the
        same record and skips the sync; `notion`/`anytype` write a row or
        object instead.

    - id: edit-pr
      requires: [persist]
      inline: true
      run: "gh pr edit {number} --body-file <scratch-path>"
      retry: { step: edit-pr, max: 2 }
      judgment: >
        Is the PR still open? The `state` field came back with the rest of the
        metadata in `gather`: on `MERGED` or `CLOSED`, skip the edit, say that
        you did, and leave `promote` to reconcile `status` alone.
      because: >
        Rewriting a merged or closed PR's body is unusual, which is why the
        skip exists — but only `backend` resolves as a comparison, so
        `pr-state != MERGED and pr-state != CLOSED` evaluates `unknown` and
        skips this step on every run, open PRs included. Pushing the body is
        the point of this skill, so the state check is yours to make from the
        `state` in hand, not a guard. Transient failures like TLS timeouts are
        common: retry twice with a short delay and confirm the command exited
        0. If it ultimately fails, halt — do not promote.

    - id: promote
      requires: [persist, edit-pr]
      when: backend == git or backend == obsidian or backend == notion or backend == anytype
      when-examples:
        match:    ["backend == git", "backend == notion"]
        no-match: ["no thoughts backend is configured for this repo"]
      inline: true
      updates: [status]
      deletes: [scratch-path]
      because: >
        With no thoughts backend, `persist` wrote only the scratch file and
        there is no record to promote — the guard resolves unknown and the
        step skips, which is the whole of a no-thoughts run here.
        Runs only after `persist` wrote the record and `edit-pr` succeeded, or
        was skipped because the PR had already resolved. Both edges are named
        here because a skipped step satisfies whatever required it: with
        `requires: [edit-pr]` alone, an `edit-pr` that skips leaves this step
        unconstrained and it runs in wave 1 — promoting a record that does not
        exist yet and deleting a scratch file nothing has written. Promoting
        without a successful edit leaves a
        `status: active` record advertising a synced PR whose body is still the
        placeholder. Reconcile `status` to the live PR `state` on **every**
        backend, not just `notion`/`anytype`: `draft` → `active` on a
        successful edit of an open PR, `merged`/`closed` when it has already
        resolved. This lookup happens every time this skill runs, not just the
        first time; it is what keeps records from sticking on `draft`. Delete
        the transient scratch file under `${TMPDIR:-${TEMP:-/tmp}}` on every
        backend — it was only ever the input to `gh pr edit`, so nothing of
        value is lost. On `notion`/`anytype`, set `merged`/`closed` only if
        that value is present in `schema.options`; otherwise leave `active` and
        add a one-line body note rather than inventing an unsupported option.

    - id: sync-promoted
      requires: [promote]
      when: backend == git
      when-examples:
        match:    ["backend == git"]
        no-match: ["backend == obsidian", "backend == notion", "backend == anytype"]
      inline: true
      run: hyprlayer thoughts sync
      because: >
        The promoted `status` is a second edit to the record file and has to be
        pushed too. Skipping it is how a promoted status stays local.

    - id: report
      requires: [promote]
      inline: true
      because: >
        Confirm the update was successful. Name every verification step still
        unchecked and say plainly that it needs manual testing before merge. If
        the PR had already merged or closed, say the record's `status` now
        reflects that — and if the user reports a merge later in this session,
        offer to re-run this skill to finalize `status: merged`.

conventions:

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
    never-add:  [thoughts, "thoughts/**"]
    never-run:  ["git add .", "git add -A"]
    instead:    hyprlayer thoughts sync
    because: >
      `thoughts/` holds symlinks into a separate repo managed by
      `hyprlayer thoughts sync`. If a commit is required elsewhere in this flow
      — the `git`-backend record write — stage explicit file paths only.
```

## Judgment

**Classifying a verification item.** An item in "How to verify it" is automatable
only if running it here is both possible and harmless. `make check test` is; a step
that posts to a live external service, or asks someone to click through a UI, is
not. Get it wrong in the permissive direction and this skill fires a real request at
a real system on the user's behalf; get it wrong in the conservative direction and
the PR ships with `- [ ] cargo test` next to a suite that passes in one second.
Deciding that the list is empty only settles whether there is anything to ask about —
it cannot tell you which items belong in the count.

**Delegating the write-up.** A herald is the default at any size: it reads the diff
and commits in its own context and hands back a finished body, and keeping a large
diff out of this context is most of what the delegation buys. Write inline only for
a genuinely trivial one-file PR, where the spawn costs more than the work. Misjudge
it upward and you pay a round trip to summarize a one-line change; misjudge it
downward and the diff you were avoiding is now in this context anyway, with the
description still unwritten.

**Trusting the herald's checklist.** The herald marks a box `- [x]` only for
commands you told it passed — but you have to check that what came back matches what
actually happened in `verify`, because the body goes to `gh pr edit --body-file`
whole and unedited. A checklist that over-claims produces a PR advertising a test
run that never happened, and the reviewer has no way to see that from the body. The
`^BLOCKER:` reject catches a herald that refused to write; it cannot catch one that
wrote confidently and wrong.

**Sorting user-facing from internal.** The template splits "What user-facing changes
did I ship?" from "How I implemented it", and the diff does not label which is
which — a refactor with a changed default is user-facing; a new public method nobody
calls yet is not. Put an implementation detail in the user-facing section and
reviewers weigh a change nobody will notice; bury a behavior change in the
implementation section and it ships unreviewed. When the PR touches several
components, organize within each section by component rather than by file.
