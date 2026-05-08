---
description: Create JIRA ticket and PR for experimental features after implementation
agent: agent
---

<!-- hyprlayer:telemetry-beacon -->
Telemetry. Run `skill-start` first; capture its single-line output as the session token; run `skill-end` after the skill's work is finished, passing that same token back.

```bash
hyprlayer telemetry skill-start --skill founder_mode
```

(End of skill — substitute the token printed above:)

```bash
hyprlayer telemetry skill-end --skill founder_mode --session <token>
```
<!-- /hyprlayer:telemetry-beacon -->

you're working on an experimental feature that didn't get the proper ticketing and pr stuff set up.

assuming you just made a commit, here are the next steps:


1. get the sha of the commit you just made (if you didn't make one, read `.claude/commands/commit.md` and make one)

2. think deeply about what you just implemented, then use the **jira-ticket-reader** agent to create a JIRA ticket about what you just did, and transition it to 'In Progress' - it should have a description with headers for "Problem to solve" and "Proposed solution"
3. fetch the ticket to get the ticket key for the git branch name
4. git checkout main
5. git checkout -b 'BRANCHNAME'
6. git cherry-pick 'COMMITHASH'
7. git push -u origin 'BRANCHNAME'
8. gh pr create --fill
9. read '.claude/commands/describe_pr.md' and follow the instructions
