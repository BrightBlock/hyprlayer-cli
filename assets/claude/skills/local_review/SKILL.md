---
name: local_review
description: Set up a git worktree for reviewing a colleague's branch, with dependency setup and thoughts init. Side-effecting; user-only — never auto-invoke. Use when the user asks to review a branch or PR locally in a fresh worktree.
allowed-tools: Bash
disable-model-invocation: true
---

# Local Review

Set up a throwaway git worktree for reviewing someone else's branch: resolve the
branch, cut the worktree under `~/hyprlayer/worktrees/<repo>/<short-name>`, install whatever this
repo needs, initialize thoughts inside it, and hand back the path so a new
Claude Code session can be started there.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything

on-empty-invocation: >
  Which branch or PR should I set up for review? Give me a branch name
  (`eng-1696-add-feature`) or a PR number (`42`).

orchestration:
  owns: [branch-resolution, worktree-naming, worktree-creation]

  steps:
    - id: resolve-pr
      when: matches(input, "^[0-9]+$")
      when-examples:
        match:    ["123", "42"]
        no-match: ["eng-1696-add-feature", "review/eng-1696", "feature/parser-rewrite"]
      inline: true
      run: gh pr view <input> --json headRefName -q .headRefName
      produces: branch
      because: >
        A bare integer is a PR number and nothing else — no branch is named
        `42`. Anything else is already a branch name and is used verbatim, so
        this step skipping is the ordinary path rather than a failure.

    - id: short-name
      requires: [resolve-pr]
      inline: true
      given:
        - { value: branch, src: "gh pr view -q .headRefName, or the argument verbatim when it was not a PR number" }
      produces: short-name
      judgment: >
        Is the derived name unique under `~/hyprlayer/worktrees/<repo-name>/`, and
        will it still
        be legible in a week? See "The short worktree name" below.

    - id: fetch
      requires: [resolve-pr]
      inline: true
      run: git fetch origin
      because: >
        Before `git worktree add`, not alongside it. `origin/<branch>` does not
        resolve until this has run, so without it "that branch does not exist"
        and "you have not fetched yet" are the same error message.

    - id: worktree
      requires: [fetch, short-name]
      inline: true
      given:
        - { value: repo-name,  src: 'mappedName from hyprlayer storage info --json; if null, d=$(git rev-parse --path-format=absolute --git-common-dir) || exit 1; d=${d%/.git}; basename "${d%.git}" — the SOURCE repo, never basename "$PWD"' }
        - { value: short-name, src: "the short-name step" }
      run: git worktree add -b review/<branch> ~/hyprlayer/worktrees/<repo-name>/<short-name> origin/<branch>
      reject: exit0(test -e ~/hyprlayer/worktrees/<repo-name>/<short-name>) or not exit0(git rev-parse --verify origin/<branch>)
      because: >
        Two preconditions with two different remedies, which is why they are
        checked before the command rather than read out of its exit code. A path
        that already exists means the user has to remove that worktree first —
        say so, rather than quietly inventing a suffixed name they will never
        find again. A branch that does not resolve on origin after a fetch is a
        typo: list the near misses with `git branch -r --list '*<fragment>*'`
        and offer them. Review happens on a new local branch, `review/<branch>`,
        so nothing you do in the worktree can reach the colleague's branch.

    - id: copy-settings
      requires: [worktree]
      when: exit0(test -f .claude/settings.local.json)
      when-examples:
        match:    ["the source repo has a .claude/settings.local.json"]
        no-match: ["the source repo has no .claude directory at all"]
      inline: true
      run:
        - mkdir -p <worktree>/.claude
        - cp .claude/settings.local.json <worktree>/.claude/
      because: >
        Local settings are the reason the review session does not re-prompt for
        every permission the user already granted in this repo. They are
        gitignored, so a fresh worktree never inherits them on its own.

    - id: deps
      requires: [worktree]
      inline: true
      run: the first matching entry in `conventions.setup-commands`, executed inside the worktree
      judgment: >
        Is the first match actually this repo's setup command? See "Which setup
        command the repo wants" below.
      because: >
        If setup fails, report the error and carry on — `thoughts-init` still
        runs and `report` still hands the tree over. A worktree with unbuilt
        dependencies is still readable and still reviewable; a run that halts
        here leaves a half-configured tree the user has to finish or delete by
        hand.

    - id: thoughts-init
      requires: [worktree, deps]
      inline: true
      given:
        - { value: repo-name, src: 'mappedName from hyprlayer storage info --json; if null, d=$(git rev-parse --path-format=absolute --git-common-dir) || exit 1; d=${d%/.git}; basename "${d%.git}" — the SOURCE repo, read before any cd' }
        - { value: worktree,  src: "the worktree step: ~/hyprlayer/worktrees/<repo-name>/<short-name>" }
      run: cd <worktree> && hyprlayer thoughts init --directory <repo-name> --yes
      because: >
        The `cd` is load-bearing, not decoration: this initializes thoughts in
        the *worktree*. Run from the source repo it rewrites the thoughts
        config and repoMappings of the repo the user is working in — a
        destructive edit to the wrong tree, and one no later step undoes.
        `--directory` takes the *source* repo's name, not the worktree's short
        name. Thoughts are keyed to the repo, so a worktree that registers
        itself as `eng-1696` gets its own empty thoughts tree instead of the
        repo's history — which looks like a working setup right up until you
        search it. That is why `repo-name` comes from `--git-common-dir` and
        not from `basename "$PWD"`: this skill is normally invoked from
        inside a worktree, where the cwd's basename IS the worktree's short
        name, so the obvious spelling produces exactly the failure this
        paragraph warns about. It cannot be left to the CLI's own default
        either — `get_current_repo_path()` is `current_dir()` and
        `get_repo_name_from_path()` is `file_name()`, so omitting
        `--directory` under `--yes` picks the same wrong name silently.
        `mappedName` is preferred over any path arithmetic because it is the
        thoughts directory the source repo actually joined, and the two
        diverge in practice: `sanitize_directory_name` turns `brightblock.ai`
        into `brightblock_ai`, so a basename would name a directory that does
        not exist. The git expression is the fallback for a source repo not
        yet initialized. In it, `--path-format=absolute` is required and must
        precede `--git-common-dir`, since the flag only affects options after
        it: bare, it returns a relative `.git` in a non-worktree checkout. The
        `|| exit 1` is equally load-bearing — `basename` swallows git's
        non-zero exit, so outside a repo the older spelling returned the
        string `.` with status 0, and `.` flows straight into
        `--directory .`. Stripping the suffix rather than taking `dirname`
        keeps bare repos and submodules right, where the common dir IS the
        repo and `dirname` climbs one level too far. Ordered after `deps` because that is the order the setup
        runs in, and it runs even when `deps` failed.

    - id: report
      requires: [copy-settings, deps, thoughts-init]
      inline: true
      because: >
        Hand back the worktree path, the branch it tracks, and which setup
        command ran or that none matched, so the user can start a new Claude
        Code session in that tree — this handoff is the launch the skill sets
        up. It is owed even when `deps` failed: report what broke alongside the
        path, never instead of it.

conventions:

  short-name:
    from: matches(branch, "[A-Za-z]{2,}-[0-9]+")   # eng-1696, ENG-1696
    case: lower
    fallback: a sanitized form of the whole branch name
    examples:
      - eng-1696-add-feature   → eng-1696
      - ENG-1696/parser-rework → eng-1696
      - feature/parser-rewrite → feature-parser-rewrite

  setup-commands:              # ordered; first match wins, all run inside the worktree
    - { if: "Makefile with a `setup` target", run: make -C <worktree> setup }
    - { if: package.json,        run: npm install }
    - { if: Cargo.toml,          run: cargo build }
    - { if: pyproject.toml,      run: pip install -e . }
    - { if: requirements.txt,    run: pip install -r requirements.txt }
    - { if: go.mod,              run: go mod download }
    - { if: "a *.sln or *.csproj file", run: dotnet restore }
    - { if: none of the above,   run: skip dependency setup }

  worktree-path:   ~/hyprlayer/worktrees/<repo-name>/<short-name>
  worktree-branch: review/<branch>

usage:
  - /local_review eng-1696-add-feature
  - /local_review 42
```

## Judgment

**The short worktree name.** The regex catches `eng-1696` in
`eng-1696-add-feature`; what it cannot catch is a branch that carries its ticket
in another shape (`1696-add-feature`, `fix/ENG_1696`) or a second branch for a
ticket that already has a worktree. Both are yours to notice. A collision does
not surface until `git worktree add` fails, after the fetch and after you have
told the user what you are about to do; a name with no ticket in it makes
`~/hyprlayer/worktrees/<repo>/` unreadable within a week, which is exactly
when a reviewer needs
to find the tree again.

**Which setup command the repo wants.** The ordered list is a heuristic about
file presence, and file presence is not intent. A repo with both a `Makefile`
and a `package.json` takes `make setup`, which may or may not install the node
dependencies; a `Makefile` can carry a `setup` target that provisions something
else entirely. Deciding the detected command is the wrong one for *this* repo is
not something the list can do. Get it wrong and you report a green setup on a
worktree that cannot build, and the reviewer finds out twenty minutes later,
inside a session they have already started.
