# ORBIT Feature Test Matrix

| Feature | Automated focus | Manual focus | Exit evidence |
|---|---|---|---|
| F01 | store consistency, reopen, deletion | fresh machine / offline | all F01 tests pass |
| F02 | embedding/reranker provider traits | local zero-network mode | model outputs validated |
| F03 | chunking, dedup, watcher events | edit file and observe incremental update | no duplicate unchanged chunks |
| F04 | hybrid retrieval, RRF, reranking, packing | exact + semantic + temporal recall | relevance + latency baseline |
| F05 | entity/edge consistency, temporal invalidation | inspect graph neighborhood | graph evidence traceable |
| F06 | parser fixtures, incremental transcript sync | ingest real local sample sessions | no duplicate events |
| F07 | MCP tool contracts and fencing | connect agent and retrieve context | safe tool behavior |
| F08 | config generation and idempotency | connect/disconnect supported harness | repeatable onboarding |
| F09 | graph query/UI integration | inspect filters, node drawer, time slider | UI smoke test |
| F10 | packaging, recovery, benchmark harness | clean install / restart / offline | release artifacts validated |
| F11 | tenant/data-plane API boundaries | hosted test tenant and workspace | parity/security evidence |
| F12 | auth, entitlement, sync replay/conflicts | signup/login/deeplink/two-device | sync correctness |
| F13 | policy evaluation, audit, retention hooks | personal vs enterprise policy flows | governance evidence |
