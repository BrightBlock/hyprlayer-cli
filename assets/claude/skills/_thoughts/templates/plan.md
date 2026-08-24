# Plan artifact template

Skills that produce implementation plans use this body structure. Populate every `[placeholder]`. Phase blocks repeat as needed.

For `git`/`obsidian` backends, prepend YAML frontmatter with every required schema field (see `_thoughts/required-metadata.md`). For `notion`/`anytype`, schema fields ride as typed properties — do NOT duplicate them in the body.

````markdown
# [Feature/Task Name] Implementation Plan

## Overview

[Brief description of what we're implementing and why]

## Current State Analysis

[What exists now, what's missing, key constraints discovered]

## Desired End State

[A specification of the desired end state after this plan is complete, and how to verify it]

### Key Discoveries:
- [Important finding with file:line reference]
- [Pattern to follow]
- [Constraint to work within]

## What We're NOT Doing

[Explicitly list out-of-scope items to prevent scope creep]

## Implementation Approach

[High-level strategy and reasoning]

## Phase 1: [Descriptive Name]

### Overview
[What this phase accomplishes]

### Changes Required:

#### 1. [Component/File Group]
**File**: `path/to/file.ext`
**Changes**: [Summary of changes]

```[language]
// Specific code to add/modify
```

### Success Criteria:

#### Automated Verification:
- [ ] Migration applies cleanly: `make migrate`
- [ ] Unit tests pass: `make test-component`
- [ ] Type checking passes: `npm run typecheck`
- [ ] Linting passes: `make lint`
- [ ] Integration tests pass: `make test-integration`

#### Manual Verification:
- [ ] Feature works as expected when tested via UI
- [ ] Performance is acceptable under load
- [ ] Edge case handling verified manually
- [ ] No regressions in related features

**Implementation Note**: After completing this phase and all automated verification passes, pause here for manual confirmation from the human that the manual testing was successful before proceeding to the next phase.

---

## Phase 2: [Descriptive Name]

[Similar structure with both automated and manual success criteria...]

---

## Testing Strategy

### Unit Tests:
- [What to test]
- [Key edge cases]

### Integration Tests:
- [End-to-end scenarios]

### Manual Testing Steps:
1. [Specific step to verify feature]
2. [Another verification step]
3. [Edge case to test manually]

## Performance Considerations

[Any performance implications or optimizations needed]

## Migration Notes

[If applicable, how to handle existing data/systems]

## References

- Original ticket: `[path or link to ticket]`
- Related research: `[path or link to research doc]`
- Similar implementation: `[file:line]`
````

## Success criteria rules

Always split criteria into two categories:

1. **Automated Verification** (runnable by execution agents): commands like `make test`, `cargo test`, `npm run lint`; file existence; compilation/type checking. Prefer `cargo` commands for Rust projects (`cargo check`, `cargo test`, `cargo clippy`, `cargo fmt --check`).
2. **Manual Verification** (requires human testing): UI/UX, performance under real load, edge cases that are hard to automate, user acceptance.

Do NOT leave open questions in the final plan. Every decision must be made before finalizing.
