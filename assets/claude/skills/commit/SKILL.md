---
name: commit
description: Create git commits with user approval and no Claude attribution. Side-effecting; user-only — never auto-invoke.
model: sonnet
allowed-tools: Bash, Read, Agent
disable-model-invocation: true
---

# Commit Changes

Turn the work of this session into a set of atomic git commits on a feature branch,
authored solely by the user, and never without their explicit go-ahead.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - subagent-guide             # what the quartermaster is for and how to brief it

orchestration:
  owns: [approval-gate, staging, execution]

  steps:
    - id: branch-check
      inline: true
      run: git branch --show-current
      produces: current-branch
      because: >
        Where you are decides whether the next step exists. A commit landed on
        `main` is not undone by a later commit.

    - id: feature-branch
      requires: [branch-check]
      when: exit0(git rev-parse --abbrev-ref HEAD | grep -Eqx "main|master")
      when-examples:
        match:    ["HEAD is main", "HEAD is master"]
        no-match: ["HEAD is feature/add-user-auth", "HEAD is fix/null-pointer-on-login"]
      inline: true
      run: git checkout -b <branch-name>
      apply: [branch-naming]
      judgment: >
        Which gitflow prefix and which description name this change? See
        "Naming the branch" below.
      because: >
        On `main` or `master` you MUST branch before committing. When the guard
        is false you are already on a feature branch — continue on it rather
        than branching again.

    - id: survey
      inline: true
      run: [git status, git diff]
      given:
        - { value: session-intent, src: "the conversation history you carry" }
      because: >
        Read the conversation and the diff together. The diff says what changed;
        only the session says why it changed, and the why is the commit message.

    - id: delegate-split
      requires: [survey]
      agent: quartermaster
      given:
        - { value: repo-root,      src: pwd }
        - { value: working-tree,   src: "git status --short, from survey" }
        - { value: session-intent, src: "the conversation history you carry" }
      ask: [ordered-commits, explicit-file-lists, drafted-messages, warnings]
      judgment: >
        Is the working tree large or mixed enough that a fresh survey of the
        diff beats what you already remember doing? See "Delegating the split"
        below.
      because: >
        It surveys the diff and returns an ordered set of atomic commits with
        explicit file lists and drafted messages, plus warnings about secrets,
        stray debug output, or generated files. It is read-only — you still
        present the plan and run the commits.

    - id: plan-commits
      requires: [delegate-split]
      inline: true
      produces: commit-plan
      apply: [message-style, attribution-rule]
      judgment: >
        Which files belong together, is this one commit or several, and where
        does the session contradict what the diff appears to say? See "Grouping,
        and the why" below.

    - id: present-plan
      requires: [plan-commits]
      inline: true
      shows: [files-per-commit, every-message-in-full]
      because: >
        The user is approving the exact text that will land in the repo, so they
        have to see all of it — file lists per commit and full messages, not a
        summary of them. `shows:` is written into the conversation as ordinary
        markdown, and this step asks nothing: the question belongs to
        `approval`. Folding the plan into the question instead produces a
        multi-kilobyte prompt string, which renders as an unformatted wall
        because that field is a label, not a document.

    - id: approval
      requires: [present-plan]
      inline: true
      produces: user-approval
      asks-user: "I plan to create [N] commit(s) with these changes. Shall I proceed?"
      because: >
        Unconditional. This step carries no `when:` because there is no state of
        the world in which it is skipped, and no `agent:` because it is never
        delegated. The gate is the `requires: [approval, ...]` on `execute` —
        nothing is staged and nothing is committed until the user has
        answered. If they amend the plan, return to `plan-commits` and present
        again — an amended plan is presented, not assumed.

        `asks-user:` is exactly the question and nothing else. The plan it
        refers to was already written out by `present-plan`, so restating any
        of it here duplicates what is on screen directly above.

    - id: execute
      requires: [approval, feature-branch]
      inline: true
      reject: >
        not exists(user-approval)
        or matches(add-command, "^git add (-A|\.|--all)")
        or exit0(git rev-parse --abbrev-ref HEAD | grep -Eqx "main|master")
      run: [git add <explicit paths>, git commit]
      produces: commits
      because: >
        Stage by explicit path, one commit at a time, in the order the plan
        listed. `feature-branch` is required here rather than earlier because
        the branch question must be settled before anything lands, not because
        planning depends on it — a skipped `feature-branch` means you were
        already on one and satisfies this just the same. That is also why the
        branch test is repeated as a `reject:` here: a satisfied `requires:`
        does not prove the branch check ran, and the one MUST in this skill
        must not rest on a guard that can evaluate unknown. Re-check the branch
        immediately before staging.

    - id: show-result
      requires: [execute]
      inline: true
      run: git log --oneline -n <commits-created>
      because: the plan was a promise; the log is the evidence it was kept.

conventions:

  branch-naming:
    case: kebab-case
    prefixes:
      feature/:  new functionality
      fix/:      bug fixes
      chore/:    maintenance, docs, config changes
      refactor/: code restructuring without behavior change
    ticket: include the JIRA ticket when one is associated
    examples:
      - feature/add-user-auth
      - fix/null-pointer-on-login
      - feature/ENG-1234-add-user-auth

  message-style:
    mood: imperative
    subject: what changed
    body: why the change was made, not just what
    voice: as if the user wrote it

  attribution-rule:
    never-add:
      - Co-Authored-By lines
      - Generated with Claude footers
      - any other AI attribution
    reject: matches(message, "Co-Authored-By|Generated with|🤖")
    holds-against: [harness defaults, message templates, habit]
    because: commits are authored solely by the user
```

## Judgment

**Naming the branch.** The prefix follows the nature of the change, not the size of
it: a rename that alters no behavior is `refactor/`, a docs pass is `chore/`, and a
one-line change that fixes a reported bug is still `fix/`. The guard only asks
whether you are standing on `main`; it cannot tell you that `feature/updates` is a
bad name. The branch name outlives the branch — it becomes the PR title and, in most
merge strategies, part of the permanent history — so a vague one is a cost paid by
everyone who reads the log later.

**Delegating the split.** Send the quartermaster a working tree that is large, mixed,
or older than your memory of it. Plan inline when the change is small or you know
exactly what you did this session. Getting this wrong is cheap in one direction and
expensive in the other: spawning it for a three-file change you just made burns a
context and returns worse grouping than your own recall, while planning inline across
a sprawling tree produces one fat commit that cannot be reverted without collateral
damage.

**Grouping, and the why.** Group by intent, not by directory: two files in the same
folder can belong to different commits, and a change plus its generated binding
usually belong to the same one. Each commit should stand alone — build, pass, and
revert cleanly. The session context you carry is an advantage the quartermaster does
not have, so correct its grouping or its "why" wherever you know better; it infers
intent from the diff, and a diff cannot show the alternative you rejected or the bug
you were actually chasing. A confidently wrong "why" in a commit message is worse
than a thin one, because someone bisecting will believe it. The user asked you to
commit and trusts your judgment — bring them one considered plan to approve, not a
menu of options to arbitrate.
