# ORBIT Hosted / Enterprise Roadmap

> This document is intentionally separate from the local-first V1 implementation plan. It is the build plan to execute after the local memory engine is stable.

## Phase H0 — Define the contract

Deliver:

- shared memory/event schema
- local/hosted storage traits
- device ID model
- sync cursor model
- access-control model
- subscription entitlement model
- evaluation corpus shared by local and hosted systems

Gate:

- same logical memory can be serialized, stored, replayed and reconstructed on another machine.

## Phase H1 — Web account, auth server and device identity

Build:

- first-launch desktop states: `Log In`, `Sign Up`, `Continue Local`
- dedicated ORBIT Auth Server for email/password, verification and recovery
- ORBIT website signup/login
- OAuth authorization server/client registration
- PKCE support
- HTTPS app/universal link
- desktop private-use scheme fallback
- device registration
- device public key and purpose-limited device security metadata
- device registry/revocation
- token/keyring storage
- account/workspace selection

Gate:

- a fresh ORBIT install can authenticate entirely through the browser and return to the app without embedding the website.

## Phase H2 — Cloud sync MVP

Build:

- sync API
- encrypted event transport
- server event store
- workspace authorization
- per-device cursor
- outbox/inbox handling
- idempotent replay
- deletion tombstones
- two-device conflict tests

Gate:

- device B can reconstruct the durable brain created on device A while both devices remain locally usable offline.

## Phase H3 — Hosted storage and retrieval

Build:

- PostgreSQL canonical records
- pgvector indexes
- object storage
- hosted lexical retrieval
- hosted graph tables
- retrieval API
- permission-aware prefiltering

Gate:

- local and hosted systems pass the same retrieval evaluation set within the defined relevance tolerance.

## Phase H3.5 — Workspace governance

Build:

- personal versus organization-owned workspace model
- tenant/workspace/project ACL evaluation before retrieval
- data classification and source exclusions
- local-only / cloud-sync / hosted-processing policy engine
- retention, deletion, export and legal-hold extension points
- policy-version tracking for processing decisions
- model/provider allow-lists for enterprise workspaces

Gate:

- a policy-denied source cannot enter cloud sync or hosted processing through any connector, worker or retrieval path.

## Phase H4 — Enterprise controls

Build:

- organization/workspace administration
- retention policies
- audit logs
- device revocation
- access reviews
- data export/delete
- regional storage policy
- enterprise identity integration
- usage/billing limits

Gate:

- tenant isolation and deletion tests pass with zero cross-tenant evidence leakage.

## Phase H5 — Optional hosted processing

Only after cloud sync is stable:

- hosted embedding workers
- hosted reranker
- hosted extraction
- hosted consolidation
- policy-based inference routing
- cost controls

Gate:

- every server-side processing path has an explicit workspace/user policy and is visible in diagnostics.

## Phase H6 — Scale and specialization

Only when measurements justify it:

- dedicated vector retrieval service
- dedicated graph service (Neo4j/FalkorDB/other)
- tenant-specific workers
- sharded indexes
- regional replicas
- enterprise dedicated deployments

Do not make these prerequisites for the first hosted product.

## Recommended order of investment

```text
Local memory quality
       ↓
Shared contracts
       ↓
Identity + device registration
       ↓
Cloud sync
       ↓
Hosted retrieval
       ↓
Enterprise controls
       ↓
Optional hosted processing
       ↓
Infrastructure specialization
```

The purpose is to prevent ORBIT from spending engineering effort on distributed infrastructure before the core long-term-memory behavior has been validated.

