---
name: hyprlayer_doctor
description: Verify the configured hyprlayer backend (git, obsidian, notion, or anytype) is ready before operations run against it. Use on first backend-touching call per session, on user request ("verify hyprlayer setup", "/hyprlayer_doctor"), and after any auth/404/schema error from a backend. Cheap (~1–15s depending on backend) and cached against the config-file content (auto-invalidates on any config change).
allowed-tools: Bash, Read, Write, mcp__claude_ai_Notion__*, mcp__anytype__*
---

# hyprlayer_doctor

Verify that the one backend configured for this repo is actually usable before
anything runs against it. Backend-agnostic work — config resolution, the
content-hash cache, per-repo mapping, dispatch — happens here; the per-backend
checks live in `./backends/<kind>.md` and run inline under this skill's tool
grants.

```yaml
loads:
  - orchestration-runtime      # how to execute this block — read before anything
  - storage-backend            # the schema the notion and anytype procedures check against

config:                        # `dirs::config_dir()` in Rust, per OS
  macos:   ~/Library/Application Support/hyprlayer/config.json
  linux:   ${XDG_CONFIG_HOME:-~/.config}/hyprlayer/config.json
  windows: '%APPDATA%\hyprlayer\config.json'

invocation:
  run-when:
    - "first backend-touching call in a session — `cache-check` decides whether that is a no-op"
    - "explicit user request: `/hyprlayer_doctor`, \"verify hyprlayer setup\", \"is my notion backend connected\""
    - "after any 401, 404, schema-mismatch, lock or permission error from a backend operation"
  never:
    - "as a SessionStart hook — the cost is wrong for the many sessions that never touch a backend"
    - "recursively, from inside a backend procedure"
    - "more than one backend's procedure in a single pass"
    - "in a loop — one pass per trigger, and a failure is a failure"

orchestration:
  owns: [config-resolution, backend-routing, aggregation, cache-lifetime]

  steps:
    - id: resolve-config
      inline: true
      run: read the config file at the OS path above, then parse it as JSON
      produces: config
      fails-with:
        missing:   "❌ Config not found at <path>. Run `hyprlayer thoughts init` to create one."
        malformed: "❌ Config at <path> is not valid JSON: <error>."
      because: >
        Two distinct failures with two distinct fixes. Report the one that
        actually happened — "config problem" sends the user to re-run init when
        the file is present and merely unparseable, and init will overwrite
        rather than repair it.

    - id: config-hash
      requires: [resolve-config]
      inline: true
      run: shasum -a 256 <config-path> | awk '{print substr($1,1,16)}'
      alternate: sha256sum on Linux
      produces: config-hash
      because: >
        The cache key is a hash of the config file's content, not a session
        identifier. `$SESSION_ID` is not reliably set in the Claude Code
        harness, and falling back to a constant (`default`) makes the cache
        global across sessions, which defeats the per-session intent and
        silently hides drift. Hashing the content buys the signal that actually
        matters: the cache invalidates itself the moment the user runs
        `hyprlayer thoughts init`, switches backends, changes a page id, or
        edits the file any other way.

    - id: resolve-backend
      requires: [resolve-config]
      inline: true
      given:
        - { value: repo-path, src: "$PWD, or the repo path passed explicitly" }
      reject: not exists(config.thoughts)
      resolution-order:
        - ".thoughts.repoMappings[$PWD] is an object with a non-null .profile → .thoughts.profiles[<profile>].backend"
        - "otherwise → .thoughts.backend"
      produces: [backend-kind, mapped-name, agent-tool]
      fails-with:
        no-thoughts: "❌ Thoughts not configured (AI may be configured but `hyprlayer thoughts init` was never run)."
        bad-kind:    "❌ with the actual value shown"
      because: >
        A repo mapping is either `"name"` or `{"repo": "name", "profile":
        "name-or-null"}`, and only the object form carries a profile — the
        string form has to fall through to the default backend rather than be
        read as a profile name. `kind` is one of git, obsidian, notion, anytype,
        lowercase per `BackendKind`'s serde rename; anything else is a ❌ that
        prints the value found, because the useful fact is what the file says,
        not merely that it is wrong. `mapped-name` is `repoMappings[$PWD].repo`
        or its string form, or null when the repo is unmapped. `agent-tool`
        comes from `.ai.agentTool` and only the anytype procedure needs it.

    - id: cache-check
      requires: [config-hash]
      inline: true
      given:
        - { value: tmp-dir, src: 'TMP="${TMPDIR:-${TEMP:-/tmp}}"' }
        - { value: sentinel, src: '$TMP/hyprlayer-doctor-$HASH.ok, with $HASH from config-hash' }
      run: find $TMP -maxdepth 1 -name hyprlayer-doctor-$HASH.ok -mmin -240 -print -quit
      produces: cache-hit
      judgment: >
        Did this invocation follow a backend error? Then a fresh sentinel has
        already been disproved, and saying so is not enough — the four steps
        below read the probe, not your opinion of it. Delete the sentinel here,
        so the probe answers false and the full procedure runs. See "Whether a
        cached pass still counts" below.
      because: >
        Resolve the temp directory portably instead of hard-coding `/tmp`:
        macOS exports `$TMPDIR` (`/var/folders/...`), Linux usually sets neither
        variable and falls through to `/tmp`, and Windows git-bash and WSL
        expose `$TEMP` (`/c/Users/<user>/AppData/Local/Temp`), where `/tmp` does
        not exist at all. The 4h TTL sits on top of the content hash because
        auth tokens and connector state drift externally even when the config
        file is untouched — hash-only would leave a disconnected Notion
        connector "verified" indefinitely. A sentinel older than 4h is not an
        error; it is simply overwritten by the next pass.

    - id: report-cached
      requires: [cache-check]
      when: exit0(find $TMP -maxdepth 1 -name hyprlayer-doctor-$HASH.ok -mmin -240 -print -quit | grep -q .)
      when-examples:
        match:    ["a sentinel for this config hash exists and is 20 minutes old"]
        no-match: ["no sentinel for this config hash", "the sentinel for this hash exists but is 9 hours old"]
      inline: true
      run: emit "✅ Already verified for this config (cached <human-readable-ago>)"
      ends-run: true
      because: >
        The terminal step on the cached path, and the only one that runs there:
        `check-backend`, `report` and `cache-write` each carry the negation of
        this same probe. That repetition is the early exit. The block has no
        early-exit vocabulary and `ends-run: true` is a note to the reader
        rather than a barrier — a step whose `requires` are already satisfied
        schedules in the same wave as this one, so without the negated guard the
        1–15s probe the cache exists to avoid would run beside the cached
        answer, and `cache-write` would then re-touch the sentinel it had just
        read, making the cache immortal. If the probe cannot resolve `$TMP` or
        `$HASH` the guard reads false and the full procedure runs, which is the
        safe direction to fail in.

    - id: check-backend
      requires: [cache-check, resolve-backend]
      when: not exit0(find $TMP -maxdepth 1 -name hyprlayer-doctor-$HASH.ok -mmin -240 -print -quit | grep -q .)
      when-examples:
        match:    ["no sentinel for this config hash", "the sentinel for this hash exists but is 9 hours old"]
        no-match: ["a sentinel for this config hash exists and is 20 minutes old"]
      inline: true
      run: backends/<backend-kind>.md — read from this skill's own directory and executed top-to-bottom
      given:
        - { value: config-path,    src: "the resolve-config step" }
        - { value: backend-kind,   src: "the resolve-backend step; it names the file" }
        - { value: backend-object, src: "the one matching variant — GitConfig, ObsidianConfig, NotionConfig or AnytypeConfig — never the whole config" }
        - { value: thoughts-user,  src: ".thoughts.user" }
        - { value: mapped-name,    src: "repoMappings[$PWD].repo or its string form; null when unmapped" }
        - { value: agent-tool,     src: ".ai.agentTool from the resolve-backend step; only the anytype procedure reads it" }
      because: >
        One generic dispatch rather than one guarded step per kind: the routing
        is a filename that `resolve-backend` already produced, so a fifth
        backend needs a new `backends/<kind>.md` and no change here. Hyprlayer
        has exactly one active backend per (repo, profile), so one pass reads
        exactly one procedure — resolve, then route, and never run two. What
        differs between the four lives in `conventions.backends`, including
        which ids each one is allowed to find missing. The guard is the
        cache-hit probe negated, because the decision at this point is not which
        backend (that is settled) but whether the expensive probe runs at all.

    - id: report
      requires: [cache-check, resolve-backend, check-backend]
      when: not exit0(find $TMP -maxdepth 1 -name hyprlayer-doctor-$HASH.ok -mmin -240 -print -quit | grep -q .)
      when-examples:
        match:    ["the cache was cold, so a procedure just ran and left ✅/⏭/❌ lines to fold"]
        no-match: ["this invocation was already answered from the sentinel by report-cached"]
      inline: true
      produces: [consolidated-report, failures]
      format: conventions.report-format
      because: >
        `requires` names `cache-check` and `resolve-backend` and not only
        `check-backend`, because a skipped step satisfies whatever required it:
        naming just the guarded procedure step would be the same as naming
        nothing, and this aggregation would schedule in wave 1 — before the
        config is parsed, before there is a backend to head the block with, and
        before any procedure has emitted a line to fold. Each procedure emits
        its own per-step ✅/⏭/❌ lines; this folds them into one block headed by
        the backend and repo, ending in a single Status line. `failures` is
        counted here and nowhere else — every ❌ line, plus every ⏭ the
        procedure did not explicitly mark optional or N/A — which is what
        `cache-write` reads. Do not re-run a write check that failed: report it.

    - id: cache-write
      requires: [cache-check, report]
      when: count(failures) == 0 and not exit0(find $TMP -maxdepth 1 -name hyprlayer-doctor-$HASH.ok -mmin -240 -print -quit | grep -q .)
      when-examples:
        match:    ["the cache was cold and the consolidated report contains no ❌ line and no unexplained ⏭"]
        no-match: ["the report contains one ❌ line", "the report contains a ⏭ the procedure did not mark optional or N/A", "this invocation was answered from the sentinel"]
      inline: true
      run: touch "${TMPDIR:-${TEMP:-/tmp}}/hyprlayer-doctor-<config-hash>.ok"
      judgment: >
        Which ⏭ lines still count as a pass? See "Which skips still count as a
        pass" below.
      because: >
        `failures` is the count `report` produces, which is why that step
        declares it. On any failure the sentinel is not written, so the next
        backend-touching call re-triggers the doctor instead of inheriting four
        hours of silence about a backend already known to be broken. The second
        conjunct is what keeps the cache mortal: a cached pass must not re-touch
        the sentinel it just read, or a config verified once stays "verified"
        forever and the 4h TTL never expires. `cache-check` is in `requires`
        because this step writes to the `$TMP` and `$HASH` that step resolved,
        and because naming only the guarded `report` would let this float into
        an early wave.

conventions:

  report-format: |
    hyprlayer_doctor (backend: notion, repo: brightblock/hyprlayer-cli):
      ✅ MCP connector: Claude.ai Notion connector reachable
      ✅ Auth: 2.8s
      ✅ Parent page: <Page Title>
      ❌ Schema: Tasks.Status expected select, found status
      ⏭  Write permission: skipped (prior step failed)

      Status: FAIL (schema). Halt before running hyprlayer commands.

  backends:
    git:
      file: backends/git.md
      sync: "commit / pull --rebase / push"
      notes: >
        Local filesystem plus a git checkout, and sync here is real, so that
        procedure's working-tree, remote and `gh` checks are the ones that carry
        weight.
    obsidian:
      file: backends/obsidian.md
      sync: no-op
      notes: >
        Local filesystem under an Obsidian vault, and sync is a no-op, so there
        is no remote to check and nothing to push. The vault-as-git collision
        check is the one that catches what people actually get wrong here.
    notion:
      file: backends/notion.md
      mcp: "none — hyprlayer registers no MCP server"
      optional-ids: [database_id]
      notes: >
        An agent-tool connector, so step 1 of that procedure infers reachability
        instead of probing with `mcp list`. `database_id` is auto-created on the
        first write — skip the checks that depend on it rather than failing
        them, or a correct first-run setup is reported broken when it is only
        new.
    anytype:
      file: backends/anytype.md
      mcp: "npx -y @any-org/anytype-mcp"
      optional-ids: [type_id]
      notes: >
        Hyprlayer registers that MCP server, so `mcp list` through the resolved
        agent tool is a real probe here — this is the only procedure that needs
        `agent-tool`. `type_id` is auto-created on first write. Do not check for
        a `title` Relation: in Anytype `title` maps to the built-in object
        `name` and is deliberately not created as a separate property, so
        flagging it would fail the exact setups the documented write flow
        produces.

  backend-procedure:
    is: a checklist, not a skill — no frontmatter, executed inline under this skill's tool grants
    read-from: this skill's own directory

  adding-a-backend:
    steps:
      - "add a `BackendKind` variant and its `*Config` struct in `src/config.rs`"
      - "create `backends/<newkind>.md` here"
      - "add one row to `conventions.backends`"
    note: >
      The dispatcher needs no change. `check-backend` reads
      `backends/<backend-kind>.md` for whatever kind `resolve-backend`
      returned, so there is no per-kind step to add.
```

## Judgment

**Whether this call is backend-touching.** Nothing fires this skill
automatically — that is the point of the SessionStart-hook prohibition — so on
the auto-trigger path you decide whether the operation you are about to run
actually reaches the backend. `hyprlayer thoughts sync` does; reading a file
that happens to live under `thoughts/` does not. Draw the line too wide and you
pay 1–15s on sessions that never touch a backend, which is the cost the hook ban
exists to avoid; draw it too narrow and the first real backend operation fails
with a raw 401 or a schema error instead of a report that says what to fix.

**Whether a cached pass still counts.** After a 401, a 404, a schema mismatch or
a lock error, the sentinel is fresh and wrong: the config file did not change,
so the hash matches, and the 4h TTL has not expired. Only you know that this
invocation followed a failure — no guard here can, because nothing in the
environment records it. Trust the sentinel in that case and you answer "✅
already verified" to the exact question the error just disproved, and the caller
walks straight into a second failure with a doctor report that says everything
is fine. Deciding it is stale is not enough, because `check-backend`, `report`
and `cache-write` are all guarded on the probe rather than on your reading of
it: delete the sentinel at `cache-check` (`rm -f
"$TMP/hyprlayer-doctor-$HASH.ok"`) so the probe agrees with you and the run
proceeds.

**Which skips still count as a pass.** `cache-write` fires only on a clean run,
and ⏭ is clean only when the procedure explicitly marked that step optional or
N/A — a missing `database_id` or `type_id` is auto-created on first write and is
genuinely fine to skip. A step that was skipped because a prior one failed, or
because you could not reach the thing it checks, is a failure wearing a ⏭. Write
the sentinel on one of those and you have suppressed the doctor for four hours
on a backend that is already broken, and the next four errors arrive with no
diagnosis attached.
