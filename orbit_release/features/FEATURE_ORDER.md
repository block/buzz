# ORBIT Feature Execution Order

> Use this file to determine what to build next. Do not skip dependency gates unless the task explicitly documents why the dependency is intentionally stubbed.

## Build order

| Order | Feature | Depends on | Primary stage |
|---|---|---|---|
| 01 | Storage Foundation | — | Local V1 |
| 02 | Embedding & Reranker Engine | F01 | Local V1 |
| 03 | Ingestion Pipeline | F01, F02 | Local V1 |
| 04 | SuperRAG Retrieval | F01, F02, F03 | Local V1 |
| 05 | Knowledge Graph | F01, F03, F04 | Local V1 |
| 06 | Recall Agent | F01, F02, F03 | Local V1 |
| 07 | MCP Server Tools | F01, F02, F04, F05, F06 | Local V1 |
| 08 | Agent Auto-Wiring | F07 | Local V1 |
| 09 | Desktop Brain Panel | F04, F05, F07 | Local V1 |
| 10 | Packaging & Hardening | F01–F09 | Local V1 release gate |
| 11 | Hosted / Enterprise Foundation | F01–F10 | Post-V1 hosted |
| 12 | Identity / Subscription / Sync | F10, F11 | Post-V1 hosted |
| 13 | Auth Server & Data Governance | F10, F11, F12 | Post-V1 hosted |

## Boundary rules

- F11 owns hosted infrastructure, tenant data plane, workers, object storage, and local/hosted parity.
- F12 owns account lifecycle at the product level, entitlement gating, device registration, and logical multi-device synchronization.
- F13 owns auth-server implementation details and the policy/governance enforcement points applied to enterprise data.
- Do not duplicate F12 functionality inside F13 or F13 functionality inside F11. Cross-feature interfaces are allowed; duplicate implementations are not.

## Required gate

A feature is not complete when the code compiles. The feature must pass its checklist, its local test cases, its dependency gate, and its documentation handoff before the next dependent feature begins.
