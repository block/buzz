# Feature XX — <Feature Name>

## Implementation agent must do

1. Read the feature `IMPLEMENTATION.md` completely.
2. Read the feature `CHECKLIST.md` completely.
3. Read the root `AGENTS.md` and `agents/FEATURE_EXECUTOR.md`.
4. Read the test cases under `testing/feature_cases/` for this feature.
5. Implement one logical task at a time; keep changes scoped to the feature and its documented interfaces.
6. Run the smallest relevant local test immediately after each task.
7. Update the feature checklist only after the behavior is actually verified.
8. Record any deviation from the specification in the feature implementation document or an explicit decision note.

## Completion gate

Before marking the feature complete:

- All feature checklist items are verified.
- Feature unit/integration tests pass locally.
- Manual smoke tests required by the feature pass.
- No forbidden external service is required for local V1 features.
- Storage/API contracts remain compatible with dependent features.
- Test evidence is recorded in the local test log.
- Reviewer agent has inspected the diff and checklist.

## Handoff

After completion, update:

- `features/CHECKLIST.md`
- feature `CHECKLIST.md`
- the relevant test case file
- `testing/LOCAL_VALIDATION_LOG.md`

Then move to the next feature in `features/FEATURE_ORDER.md`.
