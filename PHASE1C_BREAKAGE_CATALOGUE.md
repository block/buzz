# Phase 1c single-node breakage catalogue

Branch: `local-mode/phase-1`, based on committed SQLite core slice `88b9644` and the Phase 0 seam stack through `a680665`. This catalogue describes the production-router single-node slice, not the retired Phase 0 alternate router.

## Result

`BUZZ_PROFILE=single-node` enters the normal production router through `crates/buzz-relay/src/main.rs:164,1208-1305`, rejects non-loopback binding (`main.rs:1209-1214`), and constructs SQLite, in-process pub/sub and replay fencing, filesystem media, unsupported search, and permissive admission without opening PostgreSQL, Redis, or S3 connections (`main.rs:1231-1278`). The readiness log enumerates intentionally disabled background/service families (`main.rs:1295-1296`).

The fresh WP-B matrix passed profiles, public relay-member admission (kind 9030), channel creation/membership, owner↔agent chat, thread/reply/mention, reaction add/get/remove/get, DM open and sends in both directions, feeds, explicit search denial, canonical filesystem media byte round-trip, NIP-42/history/live fan-out, and restart durability. Evidence is recorded below.

## Worked as-is

| Surface | File:line evidence | Result |
|---|---|---|
| Production protocol/router and NIP-42 | `crates/buzz-relay/src/main.rs:164,1284-1294`; `crates/buzz-test-client/src/bin/local_mode_smoke.rs:7-54` | Normal router and client protocol; no local-only wire protocol. |
| PostgreSQL production profile | `crates/buzz-db/src/lib.rs:172-221`; backend matches such as `lib.rs:1730-1734` | Existing PostgreSQL construction and paths remain the default backend arms. |
| Search failure shape | `crates/buzz-relay/src/api/bridge.rs:1732-1741` | Local unsupported search returns explicit HTTP 501 `unsupported_feature`, not 500 or false success. |
| Loopback safety fence | `crates/buzz-relay/src/main.rs:1209-1214` | Single-node startup fails closed on a non-loopback bind address. |

## Worked with local backend/shim

| Surface | File:line evidence | Fidelity / limitation |
|---|---|---|
| Startup/service aggregate | `crates/buzz-relay/src/main.rs:1208-1305` | Production router with a local backend bundle; production-only workers are not started. |
| Durable SQLite core | `crates/buzz-db/src/sqlite.rs:137-458`; dispatch at `crates/buzz-db/src/lib.rs:1710-1812,1891-1961,2187-2256,4848-4850,5176-5178` | Real normalized community/channel/member/event/reaction/DM tables and SQL filtering. Thread ancestry remains canonical NIP-10 tags; denormalized thread metadata/counters are deferred. |
| Conformance row projection | `crates/buzz-db/src/lib.rs:1670-1699`; `crates/buzz-db/src/sqlite.rs:824-844` | SQLite now resolves each returned row's channel community, removing the PG-only warning while retaining the non-interference trace seam. |
| Pub/sub + presence | `crates/buzz-pubsub/src/lib.rs:133-160,994-1057`; startup `main.rs:1252,1286-1292` | Process-local, community-scoped fan-out and expiring presence leases; no cross-process Redis semantics. |
| NIP-98 replay | `crates/buzz-pubsub/src/nip98_replay.rs:1-213`; startup `main.rs:1270,1277` | Bounded in-process replay fence; restart does not retain replay history. |
| Media | `crates/buzz-media/src/storage.rs:89-98,104-313`; startup `main.rs:1275` | Filesystem CAS path supports canonical upload/download, sidecars, ranges and pages; no S3 replication. |
| Profiles/users/channels/membership | SQLite methods `crates/buzz-db/src/sqlite.rs:715-844,903-1079`; dispatch `crates/buzz-db/src/lib.rs:2299-2467,2672-2779,4334-4467` | Startup owner bootstrap plus public relay-admin and channel membership flows work locally. |
| Feed | `crates/buzz-db/src/sqlite.rs:461-530`; routed dispatch `crates/buzz-db/src/lib.rs:3444-3470,3532-3558,3618-3642` | WP-B mentions/needs-action/activity query shapes work; advanced non-routed feed methods remain PostgreSQL-only. |
| Reactions | `crates/buzz-db/src/sqlite.rs:533-599`; routed writes/removal `crates/buzz-db/src/lib.rs:2213-2256,3302-3344` | Atomic reaction/event insertion, dedupe, and removal used by CLI matrix. Aggregate/direct helper family is not fully ported. |
| DM open | `crates/buzz-db/src/sqlite.rs:601-713`; command seam `crates/buzz-db/src/lib.rs:2845-2858`; `crates/buzz-relay/src/handlers/command_executor.rs:373-413` | DM open and its command idempotency execute transactionally in SQLite. Other command kinds are rejected before persistence; broader DM list/find/create public methods remain PostgreSQL-only. |
| Workflow dispatch gate | `crates/buzz-relay/src/handlers/event.rs:520-559` | Local profile does not invoke the PostgreSQL workflow engine, matching startup's disabled declaration. |

## Stubbed / deliberately permissive

| Surface | File:line evidence | Local behavior |
|---|---|---|
| Search | `crates/buzz-relay/src/main.rs:1274`; `api/bridge.rs:1732-1741` | Service is unavailable and explicitly returns 501; FTS5 is deferred. |
| Git, push, workflows, reapers, usage, replica tasks | `crates/buzz-relay/src/main.rs:1295-1296`; Git route gate `crates/buzz-relay/src/router.rs:49-52` | Not started/exposed in the single-node profile. |
| Admission rate limiting | `crates/buzz-pubsub/src/rate_limiter.rs:97-103`; startup `main.rs:1278` | Explicit permissive in-process policy, not Redis quotas. |
| Audit | `crates/buzz-relay/src/main.rs:1280-1282` | No audit service/worker in single-node startup. |
| Relay signing identity | `crates/buzz-relay/src/main.rs:1245-1251` | If no key is configured, local mode uses a public deterministic development key; safe only with the enforced loopback bind and unsuitable for shared/production use. |
| Unsupported command families | `crates/buzz-relay/src/handlers/command_executor.rs:120-128` | Commands other than the dedicated atomic DM-open path are explicitly rejected before an idempotency event can be persisted. |
| Thread summaries and NIP-43 publications | `crates/buzz-relay/src/handlers/side_effects.rs` (`emit_live_thread_summary`, `publish_nip43_membership_list`, `publish_nip43_delta`) | PostgreSQL-backed thread/list snapshots are omitted locally; globally scoped NIP-43 deltas are also suppressed to avoid leaking the loopback roster. Durable NIP-10 tags and the SQLite relay-members table remain authoritative. |

## Blocked PostgreSQL-only public DB methods

These calls fail with typed `DbError::UnsupportedBackend` if reached on SQLite; they are not silently emulated. The list is representative of the remaining families rather than a claim that every `Db` method is ported.

| Family | File:line evidence | Consequence |
|---|---|---|
| Direct deletion helpers | `crates/buzz-db/src/lib.rs:1919-1937` | Standard event deletion uses the routed atomic helper, but callers of these direct methods remain blocked. |
| Channel policy | `crates/buzz-db/src/lib.rs:2785-2793` | Channel add-policy mutation is unavailable locally. |
| Direct DM helpers/list | `crates/buzz-db/src/lib.rs:2795-2823` | WP-B DM open uses the SQLite command seam; separate find/create/list APIs remain blocked. |
| Thread counters | `crates/buzz-db/src/lib.rs:3263-3277` | No denormalized decrement; tag-based ancestry only. |
| Direct/aggregate reaction helpers | `crates/buzz-db/src/lib.rs:3280-3299,3347-3416` | Ingest/removal tracer path works; direct add, active-record lookup, and aggregate reads are not routed. |
| Non-routed feed helpers | `crates/buzz-db/src/lib.rs:3418-3436,3510-3528,3598-3614` | HTTP/CLI routed feed works; callers choosing writer-only helpers remain blocked. |

## Verification artifacts

Fresh runtime evidence is under `.scratch/wp-b-matrix/` (ephemeral, not committed):

```text
PASS wp_b_cli_matrix
PASS media_byte_comparison
PASS kind_9030_admission
PASS nip42,event_insert,req_history,live_fanout
PASS sqlite_restart_history
PASS no_postgres_workflow_conformance_leakage
PASS no_global_nip43_publications
```

The matrix covers profiles, channel creation/membership, owner↔agent chat, reply/thread/mention, reactions through removal, DM open plus both-direction sends, owner/agent feeds, explicit search 501, and canonical media upload/download byte comparison. A scan of fresh first-boot and restart logs found no `Workflow trigger failed`, `conformance row-community lookup failed`, or `PostgreSQL operation on SQLite` lines; the fresh SQLite file contains zero kind 8000/8001/13534 global NIP-43 publication rows. Package test counts and the exact verified commit are reported with the final commit because line numbers above describe the formatted final working tree.
