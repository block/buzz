# ORBIT Build-Ready Architecture Package

This package is organized so an AI coding agent can build ORBIT directly from the included specifications without needing external architecture documents.

## Start here

1. Read `AGENTS.md`.
2. Read `docs/INDEX.md`.
3. Read `features/FEATURE_ORDER.md`.
4. Start with Feature 01.
5. For the selected feature, read `INSTRUCTIONS.md`, `IMPLEMENTATION.md`, `CHECKLIST.md`, and its test case under `testing/feature_cases/`.
6. Build and test locally before moving to the next feature.

## Product modes

- **Local:** Rust + Tauri, SQLite, LanceDB, embedded graph, local-first processing, offline capable.
- **Cloud Sync subscription:** account-enabled replication of logical memory/workspace events across devices; local recall and processing remain primary.
- **Enterprise:** hosted control/data plane, tenant/workspace isolation, governance, retention, sync, and optional hosted processing controlled by policy.

## Repository map

```text
.
├── AGENTS.md
├── agents/                    AI implementation/review/test instructions
├── docs/
│   ├── architecture/         core architecture decisions, blueprint, and build overrides
│   ├── enterprise/           hosted, auth, sync, governance architecture
│   ├── history/              consolidated decision history
│   └── roadmap/              execution roadmaps
├── features/                 implementation specifications and checklists
├── reference/                source/reference material; not an implementation dependency
├── testing/                  local test strategy, cases, fixtures and validation log
└── ci/                       future CI/CD plan; local validation comes first
```

## Important implementation rule

Do not introduce an external dependency merely because it appears in research material. The repository's architecture decisions are authoritative for the current build.

## Completion philosophy

A feature is complete only when its implementation, checklist, local tests, manual validation, security checks, and documentation handoff all pass.
