# Validation report template

`validate_plan` produces a report in this shape. Populate sections that apply; omit sections that don't.

```markdown
## Validation Report: [Plan Name]

### Implementation Status
- Phase 1: [Name] — [Fully implemented | Partially implemented | Not implemented]
- Phase 2: [Name] — ...

### Automated Verification Results
- [pass/fail] Build: `make build`
- [pass/fail] Tests: `make test`
- [pass/fail] Linting: `make lint`

### Code Review Findings

#### Matches Plan:
- [Concrete matches with file:line references]

#### Deviations from Plan:
- [Differences with file:line references; note if improvement or regression]

#### Potential Issues:
- [Risks discovered during validation]

### Manual Testing Required:
1. UI functionality:
   - [ ] [Specific check]
2. Integration:
   - [ ] [Specific check]

### Recommendations:
- [Concrete next steps]
```

## Notes

- Run every command from the plan's "Automated Verification" section and record pass/fail.
- For checkboxes already in the plan, verify the actual code matches the claimed completion before trusting them.
- If the plan is fully implemented, surface that promoting the artifact's `status` from `active` to `implemented` is the follow-up — but do not perform the mutation in this skill.
