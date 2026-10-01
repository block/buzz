# ORBIT Hosted / Enterprise Architecture

> **Status:** Planned post-local-V1 architecture
> **Primary principle:** Local-first processing with optional encrypted cloud replication and enterprise control.

## Product model

ORBIT is one memory product with a local-first desktop engine and an optional hosted control/data plane. The product has three operational states:

1. **Local-only** — no account is required to use the brain.
2. **Account + Cloud Sync** — the user has an account and selected ORBIT data is replicated across devices while processing remains local-first.
3. **Enterprise Managed** — organization-owned workspaces add tenant controls, governance, retention, access policies and optional hosted processing.

Authentication and subscription are managed by a dedicated hosted web service. The desktop remembers a secure local session so returning users experience a conventional native-app login state.

### Local ORBIT

- Tauri + Rust desktop application
- local SQLite + LanceDB + embedded graph
- local embeddings/reranking when available
- offline operation
- no account required for the local brain
- no cloud copy unless the user enables it

### Hosted / Enterprise ORBIT

- ORBIT web control plane
- account / organization / workspace management
- PostgreSQL + pgvector
- object storage
- sync API
- background workers
- optional graph service
- optional hosted model gateway
- multi-device synchronization
- organization policies and auditability

The **desktop processing path remains local-first even after sign-in**. Authentication changes identity and entitlement; it does not automatically move retrieval to the cloud.

## The key separation

```text
            PRODUCT CONTROL PLANE
 signup · login · billing · devices · workspace policy
                         │
                         ▼
                hosted ORBIT services
                         │
               encrypted logical sync
                         │
              ┌──────────┴──────────┐
              ▼                     ▼
       Desktop / laptop          Future phone
       local-first engine        local-first client
```

## What is local

By default:

- ingestion
- code parsing
- chunking
- embedding
- graph updates
- recall
- reranking
- context packing
- agent/MCP execution

This keeps interactive IDE context available without network latency.

## What the cloud adds

- identity
- billing/subscription status
- encrypted backup/replication
- multi-device continuity
- organization membership
- enterprise policies
- audit records
- optional server-side processing
- optional server-side retrieval/indexing at larger scale

## Three cloud policies

### Local-only

```text
Compute: local
Storage: local
Sync: off
```

### Cloud-sync

```text
Compute: local
Storage: local + encrypted cloud replica
Sync: on
```

### Hosted-processing

```text
Compute: local by default
Selected operations: server-side by explicit policy
Storage: cloud replica
Sync: on
```

Hosted-processing is a separate policy from cloud-sync because users may want cross-device continuity without sending their working context to a server for inference.

## First-launch authentication experience

```text
                 ORBIT desktop
                       │
          ┌────────────┼────────────┐
          ▼            ▼            ▼
       Log In        Sign Up   Continue Local
          │            │            │
          └──────┬─────┘            └→ local brain
                 ▼
          ORBIT Auth Website
      email + password + verification
                 ↓
          subscription/account
                 ↓
        deep-link authorization
                 ↓
       desktop one-time code exchange
                 ↓
        secure session in OS keychain
                 ↓
          device registration
```

Signup and password handling belong to the ORBIT web authentication server. The Tauri process never stores the plaintext password. Existing authenticated sessions should reopen locally like a standard desktop application.

## Identity data versus brain data

Keep these data classes separate:

- **Identity/control plane:** email, account ID, authentication state, subscription, organization membership.
- **Device/security plane:** random device ID, public key, platform/app version, device label, optional purpose-limited device fingerprint, security telemetry.
- **Memory/workspace plane:** source data, memories, embeddings, graph relations, decisions, sessions and derived indexes.

Device telemetry must not become part of the memory graph. A raw MAC address should not be used as the primary authenticator or long-lived identifier; use a random installation ID and, where justified, a derived device-risk signal.

## Personal versus enterprise workspaces

A user can have both personal and enterprise workspaces. They are separate authorization domains:

```text
Account
 ├── Personal Workspace
 │      └── user-owned brain
 │
 └── Organization Memberships
        ├── Enterprise Workspace A
        │      └── organization-owned data
        └── Enterprise Workspace B
               └── organization-owned data
```

Enterprise administrators should not automatically see the user's personal brain. Likewise, enterprise data must not be copied into personal workspaces by generic sync. Ownership and workspace scope must be applied before retrieval and graph expansion.

## Website → desktop authentication

Use browser-first native OAuth:

```text
Desktop
  │ generate state + PKCE
  ▼
Browser → ORBIT website → login/signup
  │
  ▼
ORBIT-owned HTTPS callback / app link
  │
  ▼
Desktop deep-link handler
  │
  ▼
One-time code exchange
  │
  ▼
Native session + device registration
```

RFC 8252 recommends using an external user-agent for native OAuth authorization and requires PKCE for public native clients. citeturn658627search0turn658627search1

Tauri supports application deep links through custom schemes and HTTPS app/universal links. citeturn658627search4

## Enterprise storage and processing governance

Attach a `WorkspacePolicy` to every cloud-capable ingest, memory, embedding, graph, sync and retrieval operation. The policy should control:

- data classification
- source exclusions / secret scanning
- local-only versus sync eligibility
- hosted-processing eligibility
- allowed model providers/connectors
- data residency
- retention and deletion
- export/legal-hold hooks
- audit level
- policy version

```text
Ingest
  ↓ classify / secret scan
  ↓ resolve owner + tenant + workspace
  ↓ evaluate policy
  ↓ local processing
  ↓ local storage
  ↓ sync gate ──────────────┐
  ↓ hosted-processing gate ─┤
  ↓ permission-aware recall │
  ↓ context export          │
```

The cloud should be unable to receive data that is marked `local_only`. Hosted inference should be impossible when the workspace policy forbids it. These checks must occur before outbound network operations, not after upload.

The control plane should be designed as **compliance-ready**, with jurisdiction-specific rules represented as deployment/workspace policy bundles rather than embedded in the memory model itself.

## Sync model

Never synchronize database implementation files.

Synchronize:

```text
logical events
logical memories
provenance
source references
workspace state
selection/deletion tombstones
```

Rebuild:

```text
SQLite indexes
LanceDB indexes
FTS indexes
graph materializations
cache
reranker features
```

Optionally cache derived embeddings/summaries in the cloud as versioned optimization artifacts.

## Canonical event example

```text
EventID: 01J...
Workspace: ws_123
Device: dev_macbook_01
Entity: mem_456
Type: MEMORY_UPSERT
Schema: 2
LogicalTime: 893241
ContentHash: sha256:...
Parents: [...] 
Payload: encrypted logical memory record
```

The server returns changes after a device's sync cursor. The client applies them idempotently and updates its local indexes.

## Enterprise topology

```text
                       Internet
                          │
                   CDN / WAF / API
                          │
              ┌───────────┴───────────┐
              │ Identity + Control    │
              │ tenants · policies   │
              └───────────┬───────────┘
                          │
            ┌─────────────┼─────────────┐
            ▼             ▼             ▼
        Sync API      Retrieval API   Billing
            │             │
            ▼             ▼
       PostgreSQL      pgvector
            │             │
            ├──── Object storage
            │
            └──── Queue / workers
                       │
         ┌─────────────┼───────────────┐
         ▼             ▼               ▼
     Embedding     Graph/ETL      Consolidation
      workers       workers          workers
```

## Scaling rule

Start hosted with PostgreSQL + pgvector and relational graph tables. Do not introduce Neo4j/FalkorDB unless measured workload requires a dedicated graph service. Preserve `GraphStore` and `VectorStore` adapters so the change remains an infrastructure substitution rather than a retrieval rewrite.

## Data ownership and privacy UX

The desktop settings should make these states obvious:

```text
Brain: Local
Cloud Sync: Off
Processing: On device

Brain: Synced
Cloud Sync: On
Processing: On device
Devices: 3

Brain: Enterprise
Cloud Sync: Managed by workspace
Processing: On device (default)
Hosted processing: Enabled by workspace policy
```

Never imply that a user is cloud-backed merely because they have an account.

