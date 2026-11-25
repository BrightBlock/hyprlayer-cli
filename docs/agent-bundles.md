# Agent bundles

The skills, agents, and hooks hyprlayer installs for Claude and Codex ship as
two matching assets attached to each GitHub release:
`hyprlayer-assets-{claude,codex}-<version>.tar.gz`.

From **1.6.0** the bundle is pinned to a release version rather than tracking
`master`. Hyprlayer provisions the matching Claude/Codex pair automatically;
`hyprlayer ai reinstall` repairs it on demand. Both assets are downloaded and
their published SHA256 digests verified before either live integration is
changed. A release carrying only one half is not installable. Once the
installed version and symlinks match the wanted generation, startup performs
no network I/O at all.

On every supported OS, bundle bytes live in
`~/.config/hyprlayer/agents/<version>/{claude,codex}`. Claude and Codex agent
and skill locations are per-entry link farms into that central generation,
matching Omarchy's mixed-directory convention: Hyprlayer creates the native
directory and links each named entry into it without replacing unrelated
skills or agents. Claude skills go to `~/.claude/skills`; Codex-compatible
skills go to both `~/.agents/skills` and `~/.codex/skills`, as Omarchy does;
custom agents go to each harness's `agents` directory. Windows uses directory
junctions where a directory symlink is unavailable, while file entries remain
symlinks. If Windows does not permit file symlinks, setup stops with Developer
Mode/elevation guidance instead of silently creating divergent copies.

## Choosing a version

```bash
hyprlayer ai versions                      # releases carrying both supported bundles
hyprlayer ai versions --json --limit 20
hyprlayer ai reinstall --version 1.6.0     # pin to a version and install it
hyprlayer ai reinstall --unpin             # back to the binary's own bundle
hyprlayer ai reinstall --force             # bypass the local store and download again
```

A pin survives binary upgrades, so a bundle that regresses can be held back
until it is fixed. `hyprlayer ai status` shows one compact version and calls out
a pin or CLI skew only when relevant; `--json` exposes `assetsVersion`,
`pinnedVersion` and `binaryVersion` separately. A pin is refused if its bundle
needs a newer CLI than the one running.

`ai versions` is the only new API call, it runs on demand rather than at
startup, and its result is cached for an hour — repeated calls do not spend the
unauthenticated rate-limit budget.

## Your edits are kept

Installs never replace a personal file or an external link that occupies a
managed name. They leave the collision in place, report the incomplete pair,
and wait for you to resolve it. On the first store-mode install, exact
digest-matched copies from older Hyprlayer releases are migrated to links;
anything modified is treated as personal and kept. The desktop presents
store-backed agents as read-only System entries, so customizations are made by
creating a separate personal agent rather than editing shared release bytes.

`~/.claude/settings.json` remains mutable configuration rather than an agent
asset: an existing file is preserved, and only a missing file receives the
starter settings.

Skills retired before 1.6.0 are cleaned up too. `ci_commit`, `create_plan_nt`
and the six others dropped at that release are still on disk on every machine
that ran an older hyprlayer — pre-1.6.0 installs kept no record, so nothing
could prove they were ours to delete, and the harness kept finding them next to
the skills that replaced them. The binary now carries a digest list of exactly
what those installs wrote, so an install removes them, and removes only files
that still match it byte for byte. Edit one and it is yours; it stays.

1.6.0 handled that first install differently: it copied each file it replaced to
`<name>.hyprlayer-backup`, which left one inside nearly every skill directory —
extra files sitting where the harness scans for skills. Installs from 1.6.1 on
clear them out, on the daily refresh as well as an explicit reinstall. Only
copies sitting beside a file hyprlayer ships are removed — that is exactly the
set that install could have written, so anything of yours wearing the same
suffix stays. `settings.json.hyprlayer-backup` stays too: it may be the only
copy left of the settings that install replaced, so merge it back and delete it
yourself.

## Frozen legacy trees

The root `claude/` directory is **frozen** and exists only to serve Claude
installs made by CLIs older than 1.6.0, whose download path is compiled in and
cannot be retargeted. It is not a live installation source. Current Claude
and Codex work happens in `assets/`, which is where release bundles are cut
from. See
[assets/FROZEN.md](assets/FROZEN.md).

Once the paired Claude + Codex setup is healthy, an upgrade also retires the
Copilot and OpenCode files installed by pre-1.6.0 Hyprlayer clients. Cleanup is
per-file and digest-guarded: known unmodified Hyprlayer files are removed,
while edited files, personal files, and the shared harness roots remain.

