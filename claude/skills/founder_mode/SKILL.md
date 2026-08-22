---
name: founder_mode
description: Create a JIRA ticket and PR retroactively for an experimental feature that bypassed the standard ticketing flow. Side-effecting; user-only — never auto-invoke. Use when the user asks to "founder-mode" a commit or retroactively ticket and PR something already on the local branch.
allowed-tools: Bash, Read, Agent
disable-model-invocation: true
---

# Founder Mode

Retroactively ticket and PR an experimental feature that is already committed on
your local branch: create the JIRA ticket the work should have started from, then
replay the commit onto a properly named branch and open a PR against it.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything

follows:                       # skills read in full at the step that names them
  - ~/.claude/skills/commit/SKILL.md
  - ~/.claude/skills/describe_pr/SKILL.md

produces:
  - jira-ticket                # created, then transitioned to 'In Progress'
  - pull-request               # opened with `gh pr create --fill`, then described

orchestration:
  owns: [ticket-framing, branch-naming, git-history, pr-creation]

  steps:
    - id: commit-if-needed
      when: 'exit0(test -n "$(git status --porcelain)")'
      when-examples:
        match:
          - "the feature was written into new files that are still untracked"
          - "tracked files are modified but nothing has been committed"
        no-match: ["the working tree is clean — the experiment is already one commit"]
      inline: true
      follows: ~/.claude/skills/commit/SKILL.md
      judgment: >
        Is the uncommitted work the experiment, or something else that happens
        to be sitting in the tree? See "What `commit-if-needed` may commit"
        below.
      because: >
        The rest of this block assumes a commit already exists; everything
        downstream cherry-picks it. If the experiment is still sitting in the
        working tree, commit it here rather than discovering the gap at
        `git cherry-pick`, three irreversible branch switches later.
        The probe is `git status --porcelain`, not `git diff --quiet HEAD`:
        the ordinary founder-mode shape is a feature written into NEW files,
        which `git diff` cannot see at all. Under the diff probe the guard
        reads false on the exact case it exists to catch, this step skips, and
        `capture-sha` goes on to record a pre-existing HEAD — the failure its
        own `because:` describes.

    - id: capture-sha
      requires: [commit-if-needed]
      inline: true
      run: git rev-parse HEAD
      produces: commit-sha
      because: >
        Read before any branch switch. `git checkout <default-branch>` moves
        HEAD, so a SHA resolved after `base-branch` names a different commit
        entirely — and cherry-picking that one produces a PR that looks
        plausible and contains someone else's work.

    - id: read-change
      requires: [capture-sha]
      inline: true
      given:
        - { value: commit-diff,    src: "git show --stat HEAD" }
        - { value: commit-message, src: "git log -1 --format=%B HEAD" }
      judgment: >
        What problem does this commit actually solve, and what did you choose to
        build for it? See "Framing the retroactive ticket" below.

    - id: create-ticket
      requires: [read-change]
      agent: jira-ticket-reader
      given:
        - { value: change-summary, src: "your reading of git show --stat in read-change" }
        - { value: commit-sha,     src: "git rev-parse HEAD" }
      ask: [ticket-key, ticket-url]
      transition-to: In Progress
      description-headers: ["Problem to solve", "Proposed solution"]
      because: >
        The ticket is created in the state the work is really in — 'In
        Progress', not the default. A retroactive ticket left in 'To Do' while
        its PR is already open is worse than no ticket: it reports the wrong
        thing to everyone reading the board, and nobody re-checks a ticket that
        looks unstarted.

    - id: confirm-ticket
      requires: [create-ticket]
      agent: jira-ticket-reader
      ask: [ticket-key, status]
      reject: not matches(ticket-key, "^[A-Z][A-Z0-9]+-[0-9]+$")
      because: >
        Fetch the ticket back rather than trusting the creating agent's own
        report: a sub-agent's report is input, not truth, and the branch name,
        the PR title and every future link are all derived from this key. A key
        that is wrong here is wrong in three places that are expensive to
        rename once the PR exists.

    - id: base-branch
      requires: [confirm-ticket]
      inline: true
      given:
        - { value: default-branch, src: "git symbolic-ref --short refs/remotes/origin/HEAD | sed 's@^origin/@@'" }
        - { value: ticket-key,     src: "the confirm-ticket fetch" }
      run:
        - git checkout <default-branch>
        - git checkout -b <branch-name>
      produces: branch-name
      reject: not exists(default-branch) or matches(default-branch, "^origin/")
      because: >
        The prose this replaces said `git checkout main`. Resolve the default
        branch instead of assuming its name — this repository's is `master` —
        because branching an experiment off a stale or nonexistent `main` fails
        late and quietly, at the point where the PR shows a diff nobody wrote.
        Strip the remote prefix: bare `git symbolic-ref --short
        refs/remotes/origin/HEAD` prints `origin/master`, and `git checkout
        origin/master` detaches HEAD instead of checking out a branch, so the
        next `git checkout -b` cuts the ticket branch from a detached HEAD and
        nothing here complains. `origin/HEAD` is also unset in `--single-branch`
        and most CI clones, where that command exits non-zero and prints
        nothing; the fallback is `git remote show origin | sed -n 's/.*HEAD
        branch: //p'`, and the reject rule is what stops an empty or still
        remote-prefixed value from reaching `git checkout` at all.

    - id: cherry-pick
      requires: [base-branch]
      inline: true
      given:
        - { value: commit-sha, src: "the capture-sha step, read before the branch switch" }
      run: git cherry-pick <commit-sha>
      reject: matches(commit-sha, "^HEAD")
      because: >
        The value must be the literal hash captured in `capture-sha`, never the
        symbol `HEAD`. After `base-branch` ran, `HEAD` is the tip of the freshly
        cut branch, so cherry-picking it is a no-op that leaves the branch empty
        and every later step green.

    - id: push
      requires: [cherry-pick]
      inline: true
      run: git push -u origin <branch-name>

    - id: open-pr
      requires: [push]
      inline: true
      run: gh pr create --fill
      because: >
        `--fill` seeds the title and body from the commit message. That is a
        placeholder, not a description — `describe` replaces it.

    - id: describe
      requires: [open-pr]
      inline: true
      follows: ~/.claude/skills/describe_pr/SKILL.md
      because: >
        Read that skill in full and follow it. The commit message `--fill`
        copied explains one commit to someone who already has the context; the
        PR description has to explain the change to someone who does not.

conventions:
  branch-name:
    from: ticket-key
    must-contain: the ticket key verbatim, so the branch, the PR and the ticket all cross-link
    reject: not matches(branch-name, "^[A-Z][A-Z0-9]+-[0-9]+")
```

## Judgment

**What `commit-if-needed` may commit.** The guard answers "is anything
uncommitted", which is one question short of "is the *experiment*
uncommitted" — and no probe closes that gap. A tree dirtied by something
unrelated (a half-edited config, a scratch file) reads identically to one
holding the experiment, and the commit skill will sweep whatever is staged into
the commit that `capture-sha` then cherry-picks onto a branch named after the
ticket. Stage only the experiment. If the experiment is already committed and
the dirt is somebody else's business, leave it: a PR carrying an unrelated file
is reviewed as though that file were part of the feature, and the ticket you
just wrote describes a change the diff does not contain.

**Framing the retroactive ticket.** The commit already exists, so the temptation
is to describe the diff. Do not: "Problem to solve" describes the state of the
world *before* the commit and why it was worth changing, and "Proposed solution"
describes the approach you picked over the ones you did not. A ticket that
restates the diff tells the reviewer nothing they cannot read in the PR, and
tells whoever finds it in six months nothing at all — which defeats the only
reason to write the ticket after the fact.

**Which commit is "the experiment".** The block assumes exactly one — the prose
it came from says "assuming you just made a commit", and `capture-sha` reads a
single HEAD. Nothing checks that assumption, because nothing can: only you know
whether the experiment is that one commit or the four before it. Cherry-pick
only the last of a series and the branch builds locally off your cache and fails
in CI, on a PR that is already open and already tagged with a ticket.
