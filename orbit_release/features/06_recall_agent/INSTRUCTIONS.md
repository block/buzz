# Feature 06 — Recall Agent — AI Execution Instructions

> These instructions are additive. The existing `IMPLEMENTATION.md` and `CHECKLIST.md` remain the detailed specification for this feature.

## Read first

1. `../../AGENTS.md`
2. `../../agents/FEATURE_EXECUTOR.md`
3. `../../docs/architecture/BUILD_OVERRIDES.md`
4. `./IMPLEMENTATION.md`
5. `./CHECKLIST.md`
6. `../../testing/feature_cases/F06_RECALL_AGENT.md`
7. `../../features/FEATURE_ORDER.md`

## Dependency gate

Required dependencies: **F01, F02, F03**.

Do not begin implementation until the dependency features are either complete or explicitly stubbed in a documented local test harness.

## Feature focus

IDE transcript ingestion and durable recall.

## Execution protocol

- Work in small, reviewable tasks.
- After each task, run the narrowest relevant local test.
- Do not mark checklist items complete based on code presence alone.
- Keep the public contracts in `orbit-core` / storage / retrieval interfaces stable unless the implementation document explicitly requires a contract change.
- Preserve local-first behavior for desktop features.
- Do not introduce a server, cloud dependency, or CI-only workaround into a local feature.

## Done gate

The feature is complete only after:

- implementation checklist is fully verified;
- feature tests pass locally;
- manual checks required by the feature are recorded;
- dependency compatibility is confirmed;
- security/privacy requirements are tested where applicable;
- the reviewer has signed off;
- the master checklist and local validation log are updated.
