---
name: create_handoff
description: Create a handoff document for transferring work to another session. Use when the user asks to create a handoff, summarize the session for a future agent, or capture context for a clean restart. Produces a thoughts artifact (a handoff).
allowed-tools: Bash, Read, Write, Edit, Agent, Skill, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# Create Handoff

Compact this session into a document another agent can start cold from: what the
work is, where it stands, and what only this context knows. Thorough, and
concise — losing a key detail defeats it, and so does a wall nobody reads.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - storage-backend            # where the artifact is saved
  - required-metadata          # schema fields + backend-specific title format
  - subagent-guide             # the archivist's two jobs, and the spawning rules
  - templates/handoff          # the artifact body structure

artifact:
  type: handoff
  status: active               # a handoff is actionable the moment it is written
  title: "<ENG-XXXX|general>: <short description>"
  body-type: implementation_strategy   # domain-specific; supplements the schema-level `type: handoff`

on-completion:
  emit: verbatim                # the XML tags in the source are markers — never part of your response
  path-field: "local filepath for git/obsidian; page/object ID for notion/anytype"
  example-path: thoughts/shared/handoffs/ENG-2166/2025-01-08_13-44-55_ENG-2166_create-context-compaction.md
  text: |
    Handoff created! You can resume from this handoff in a new session with the following command:

    ```bash
    /resume_handoff <path or id>
    ```

orchestration:
  owns: [narrative, learnings, persistence, sync, completion-message]

  steps:
    - id: scope-session
      inline: true
      produces: [ticket, task-in-flight]
      judgment: >
        Which ticket does this work hang off — or is it `general` — and what is
        the task in flight? See "Naming the ticket" below.
      because: >
        The ticket fixes the destination directory and the title; the task in
        flight fixes what everything downstream is about. Decided once, here,
        before anything else reads them.

    - id: metadata
      requires: [scope-session]
      inline: true
      given:
        - { value: date-iso,   src: "date -Iseconds" }
        - { value: time-24h,   src: "date +%H-%M-%S" }
        - { value: git-commit, src: "git rev-parse HEAD" }
        - { value: branch,     src: "git branch --show-current" }
        - { value: repository, src: 'mappedName from hyprlayer storage info --json (already fetched below for backend); if null, d=$(git rev-parse --path-format=absolute --git-common-dir) || exit 1; d=${d%/.git}; basename "${d%.git}" — the SOURCE repo; --show-toplevel returns the worktree' }
        - { value: researcher, src: "hyprlayer thoughts config --json (user field) or git config user.name" }
        - { value: backend,    src: "hyprlayer storage info --json" }
      produces: destination
      reject: exists(unresolved-placeholder)
      because: >
        Collected directly, and before writing rather than after. The template
        has a placeholder for every one of these; a document written with
        placeholder metadata ships with placeholders.

    - id: paper-trail
      requires: [scope-session]
      agent: archivist
      job: retrieve-paper-trail
      given:
        - { value: repo-root,       src: pwd }
        - { value: ticket-or-topic, src: "the ticket scope-session resolved" }
      ask: [plans, research-docs, prior-handoffs, pr-records, status-of-each, paths-or-ids]
      reject: not exists(ticket-or-topic)
      because: >
        The Artifacts section is the part of a handoff that lives in the store
        rather than in your head. Delegated, it is accurate and current;
        remembered, it is a list of paths that were true earlier in the session.

    - id: raw-dump
      requires: [scope-session]
      inline: true
      produces: raw-notes
      judgment: >
        What does only this session know — which dead ends, which approach was
        abandoned and why, what "done" actually means for the task in flight?
        See "Recovering what only this session holds" below.
      because: >
        This is the half of a handoff no sub-agent can recover, so it is never
        delegated. Get it out of context as raw state, changes, and learnings
        first; shaping it is a separate and cheaper problem.

    - id: shape
      requires: [raw-dump]
      agent: archivist
      job: draft-handoff
      given:
        - { value: raw-notes, src: "the dump you just wrote in raw-dump" }
        - { value: template,  src: "_thoughts/templates/handoff.md" }
      ask: [structured-body]
      reject: not exists(raw-notes)
      judgment: >
        Is this session long enough that a delegated shaping pass earns its
        context? See "Delegating the shaping pass" below.

    - id: reconcile
      requires: [paper-trail, shape]
      inline: true
      retry: { step: shape, max: 1 }
      judgment: >
        Does the paper trail match what this session actually produced, and did
        the shaping pass drop a learning you consider load-bearing? See "What
        the archivist may not drop" below.
      because: >
        A sub-agent's report is input, not truth. Put back what is missing
        before it becomes the artifact; a second miss means writing the body
        yourself rather than spawning a third time.

    - id: write
      requires: [metadata, reconcile]
      inline: true
      given:
        - { value: template,    src: "_thoughts/templates/handoff.md" }
        - { value: body,        src: "the reconciled narrative" }
        - { value: destination, src: "the path metadata resolved" }
      apply: [filename, metadata-rendering, body-rules]
      destination-when:
        filename-pattern:    backend == git or backend == obsidian
        backend-assigned-id: backend == notion or backend == anytype
      judgment: >
        Thorough but concise: what compresses to a `file:line` reference, and
        what has to stay in prose? See "Thorough but concise" below.
      because: >
        Populate every placeholder in the template, then save to the destination
        the storage backend dispatch resolved. Persistence is yours.

    - id: sync
      requires: [write]
      when: backend == git
      when-examples:
        match:    ["backend == git"]
        no-match: ["backend == obsidian", "backend == notion", "backend == anytype"]
      inline: true
      run: hyprlayer thoughts sync
      because: >
        Only `git` has a push step. Obsidian writes straight into the vault, and
        notion/anytype already wrote through their API.

    - id: present
      requires: [write, sync]
      inline: true
      emits: on-completion.text
      reject: not exists(destination)
      because: >
        Emitted verbatim, with the path field filled per `on-completion`. A
        skipped sync still satisfies this barrier — say that you skipped it.

conventions:

  filename:
    applies-to: [git, obsidian]
    pattern: thoughts/shared/handoffs/ENG-XXXX/YYYY-MM-DD_HH-MM-SS_ENG-ZZZZ_description.md
    fields:
      YYYY-MM-DD:  "today's date"
      HH-MM-SS:    "current time, 24-hour (13-00-00 for 1:00 pm)"
      ENG-XXXX:    "ticket number; `general` when there is none"
      ENG-ZZZZ:    "ticket number; omitted entirely when there is none"
      description: brief-kebab-case
    examples:
      - with-ticket:    2025-01-08_13-55-22_ENG-2166_create-context-compaction.md
      - without-ticket: 2025-01-08_13-55-22_create-context-compaction.md
    delegated-to-backend: [notion, anytype]   # human-readable title per required-metadata; the backend assigns the ID

  metadata-rendering:
    git-obsidian: yaml-frontmatter
    notion-anytype: typed-properties          # schema fields ride as properties — do not repeat them in the body
    supplementary-in-body: [git_commit, branch, topic, "type: implementation_strategy"]
                                              # the domain-specific body type only —
                                              # never the schema-level `type: handoff`

  body-rules:
    floor: more-information-not-less          # the template is the minimum, never the ceiling
    covers: [top-level-objectives, lower-level-details]
    code-snippets: brief-only                 # one that describes a key change, or an error you are debugging
    never: [large-blocks, pasted-diffs]
    prefer: "path/to/file.ext:line"           # e.g. packages/dashboard/src/app/dashboard/page.tsx:12-24
```

## Judgment

**Naming the ticket.** The ticket may be in the branch name, in the plan you were
working from, in the user's opening message, or nowhere at all — `general` is a
legitimate answer, not a fallback for not having looked. Nothing mechanical checks
this, and it decides both the destination directory and the title: a correct handoff
filed under the wrong ticket is invisible to the next session, which lists
`handoffs/ENG-XXXX/` and finds nothing.

**Recovering what only this session holds.** git still has the diff and the store
still has the artifacts. What dies with this context is why an approach was
abandoned, which lead was already chased and came back empty, and what "done"
actually means for the task in flight. Those lines cannot be re-derived at any
price, so they are the ones that must survive compaction — if you are cutting, cut
the recoverable half.

**Delegating the shaping pass.** Hand the archivist your raw dump plus the template
when the session has run long enough that shaping it yourself would cost more
context than you have left to spend. There is no counter to consult here; the
question is whether the dump has outgrown one pass of your own attention. On a short
session the round trip costs more than it saves, and a trip through a sub-agent can
only lose detail, never add it.

**What the archivist may not drop.** The returned body reads as internally
consistent whether or not it kept your learnings — that is exactly what makes the
loss hard to spot. Read it against your raw dump rather than on its own merits, and
put back anything missing. Read the paper trail the same way: it reports the store's
view, so when it lists a plan this session superseded, or misses an artifact you
wrote an hour ago, your context is ahead of the store and wins. A handoff that reads
well and omits the dead end sends the next session straight down it.

**Thorough but concise.** The handoff has to survive without the context that
produced it and still be readable in one pass. More information rather than less —
but a change that compresses to `src/foo.rs:120-148` should not arrive as a pasted
diff, and a snippet earns its place only when it *is* the point, like the error you
are debugging. Err one way and the next session skims a wall; err the other and it
re-researches what you already knew.
