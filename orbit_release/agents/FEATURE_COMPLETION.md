# Agent Role — Feature Completion Gate

Before declaring a feature complete, verify:

- all checklist items are implemented;
- unit tests pass;
- integration tests pass;
- manual local checks pass where specified;
- failure/recovery behavior is covered where the feature persists state;
- security/privacy checks pass where applicable;
- no feature dependency was silently changed;
- docs/checklists/test evidence are updated;
- reviewer sign-off is recorded.

## Required handoff artifacts

1. updated feature `CHECKLIST.md`
2. feature test case status
3. `testing/LOCAL_VALIDATION_LOG.md` entry
4. any explicit decision note needed for deviations

Do not mark completion because the application merely compiles.
