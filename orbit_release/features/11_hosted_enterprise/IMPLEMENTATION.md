# Feature 11 — Hosted / Enterprise ORBIT Foundation

> **Priority**: P1 — build after the local-first brain is stable.
> **Target**: Hosted subscription and enterprise deployment.
> **Dependencies**: Features 01–10.

## Product principle

ORBIT has **one memory engine and two deployment modes**:

- **Local ORBIT**: all processing and storage remain on the device unless the user explicitly enables an external provider or cloud sync.
- **Hosted / Enterprise ORBIT**: the same logical memory schema and retrieval algorithms are exposed through a multi-tenant service, while desktop processing remains local-first by default.

The hosted edition exists to provide account identity, encrypted replication, multi-device continuity, organization controls and optional hosted compute. It should not require the desktop product to become network-dependent.

## Hosted architecture

```text
                       orbit.example.com
                              │
                  ┌───────────┴───────────┐
                  │ Web App / Control Plane│
                  │ signup · billing       │
                  │ workspace · devices   │
                  └───────────┬───────────┘
                              │
                         Auth / API
                              │
              ┌───────────────┼────────────────┐
              ▼               ▼                ▼
        PostgreSQL +        Object          Sync API
        pgvector            Storage         + Event Log
              │               │                │
              └───────────────┼────────────────┘
                              ▼
                    Background Workers
              ingest · embeddings · graph ·
                 consolidation · reindex
                              │
                    Optional graph service
                         when justified

                  ▲                    ▲
                  │ encrypted logical │ sync
                  │      state        │
          ┌───────┴───────┐    ┌─────┴─────────┐
          │ ORBIT Desktop │    │ Future Mobile │
          │ Rust + Tauri  │    │ ORBIT Client  │
          └───────────────┘    └───────────────┘
              local-first           local-first
```

## Product entry and authentication model

The hosted/enterprise edition has a dedicated **ORBIT Authentication & Account Server**. It should be separately deployable from the memory data plane so credential infrastructure and brain/workspace storage have independent security boundaries. This service is separate from the memory data plane. It is responsible for identity, registration, email/password authentication, recovery, subscriptions, device registration and desktop deep-link authorization.

```text
Tauri Desktop
   │
   ├─ Log In ───────────────┐
   ├─ Sign Up ──────────────┤→ ORBIT Auth Server / Web UI
   └─ Continue Local         │
                             │ email + password + verification
                             ↓
                       account / subscription
                             │
                     one-time auth code
                             ↓
                    deep-link back to Tauri
                             ↓
                 device registration + policy
```

The desktop stores a secure local session using the OS keychain. Account metadata and device metadata are never treated as long-term memory.

### Desktop account state

| Data | Desktop storage | Cloud source |
|---|---|---|
| Plaintext password | Never stored | Auth server only |
| Session/refresh secret | OS keychain | Issued by auth server |
| Account ID | Local encrypted app state | Auth server |
| Workspace selection | Local app state | Control plane |
| Entitlement cache | Local signed/short-lived cache | Billing/entitlement service |
| Brain content | Local memory stores | Only if sync is enabled |

## Enterprise personal-data / organization-data separation

A signed-in person can operate both a personal workspace and one or more enterprise workspaces. The backend must model ownership explicitly rather than deriving it from the current user alone.

| Domain | Owner | Default visibility | Typical content |
|---|---|---|---|
| Personal | User account | User only | personal notes, personal projects, private memory |
| Enterprise | Organization/tenant | ACL controlled | company repositories, policies, team memories, enterprise decisions |
| Enterprise project | Organization + project | Project ACL | repo-specific context, customer/project knowledge |
| Local-only | User/device or enterprise policy | Device only | excluded secrets, sensitive paths, temporary work |

The same email/account can have membership in an organization without giving that organization visibility into the person's personal workspace. Enterprise records should carry organization/tenant ownership and cannot be copied into personal workspaces through generic sync.

## Hosted components

### 1. Identity and account control plane

Responsibilities:

- account signup/login
- organization and workspace membership
- device registration/revocation
- subscription entitlement state
- OAuth/OIDC session lifecycle
- audit events for security-sensitive operations

The browser is the account UI. The desktop app should not own signup forms or password handling.

### 2. PostgreSQL + pgvector

Use PostgreSQL as the hosted authoritative relational store. Use pgvector for server-side vector retrieval where server-side indexing is enabled.

Store:

- users and organizations
- workspaces and memberships
- canonical logical memories
- memory provenance
- sync events and snapshots
- subscription entitlements
- access policies
- optional embeddings / retrieval artifacts

The hosted database is a **server adapter**, not the canonical definition of the ORBIT memory model.

### 3. Object storage

Use object storage for large immutable artifacts:

- source snapshots
- attachments
- exported brain archives
- large transcript payloads
- encrypted backup packages

Keep large blobs out of PostgreSQL unless the item is small and strongly benefits from transactional coupling.

### 4. Sync service

The sync service accepts and returns logical mutations. It must never replicate SQLite WAL/page files, LanceDB directories or graph database files.

Every mutation should include at least:

- `event_id`
- `workspace_id`
- `device_id`
- `entity_id`
- `event_type`
- `logical_timestamp`
- `content_hash`
- `schema_version`
- `payload_ref`
- `parent_event_ids` or an equivalent causal/version marker

The server applies authorization before accepting or returning an event.

### 5. Background worker system

Separate slow work from interactive API requests:

- ingestion workers
- embedding workers
- graph/entity workers
- consolidation workers
- sync workers
- reindex workers
- evaluation workers

Workers must be idempotent. A retried event must not create duplicate memories or duplicate graph relations.

## Identity/device metadata is a separate control-plane data class

Keep these records outside the memory graph and source-content schema:

- account ID and verified email
- subscription/entitlement state
- organization memberships
- device ID and device public key
- platform/app version
- device label
- security/risk telemetry
- login, device and authorization audit events

A device identifier is not a secret. A raw MAC address should not be required for authentication or used as a long-lived user identifier. Where additional device-risk signals are required, use a purpose-limited derived fingerprint and document its retention.

## Local-first cloud behavior

A signed-in ORBIT user should experience:

```text
Local edit
   ↓
Local SQLite event
   ↓
Local retrieval/index update
   ↓
User continues working immediately
   ↓
Sync outbox records logical mutation
   ↓
When online + entitled:
upload encrypted event/state
   ↓
Server validates + stores canonical replica
   ↓
Other registered device fetches changes
   ↓
Device applies event
   ↓
Device rebuilds/updates local indexes
```

The network should therefore be a **replication path**, not part of the interactive recall hot path.

## Cloud sync granularity

Offer two conceptual sync scopes:

### Brain sync

Synchronize durable memories, decisions, preferences, workspace metadata and the minimum source/provenance needed to reconstruct context on another device.

### Full workspace sync

Synchronize selected source files/snapshots as well, subject to user/enterprise policy.

A workspace may explicitly exclude:

- `.env` files
- secrets
- generated build directories
- vendor directories
- credentials
- private paths
- files marked local-only

## Hosted processing policy

The enterprise service can support server-side processing later, but it must be explicit:

```text
Policy: LOCAL_ONLY
    → desktop processes data; cloud stores sync state only

Policy: CLOUD_SYNC
    → desktop processes data; encrypted cloud replica stores selected data

Policy: HOSTED_PROCESSING
    → specific operations may execute on ORBIT servers under tenant policy
```

Hosted processing must never be silently enabled because a user signed in.

## Enterprise storing/processing governance

All enterprise ingestion and processing must carry a `WorkspacePolicy` context. The policy travels with the logical memory/event through the pipeline.

```text
INGEST
  ↓
Identity + workspace resolution
  ↓
Classification / secret detection
  ↓
Policy decision
  ├─ where may this data be stored?
  ├─ may it sync?
  ├─ may embeddings be generated locally/cloud?
  ├─ may a hosted reranker/LLM receive it?
  ├─ how long may it be retained?
  └─ who may retrieve it?
  ↓
LOCAL PROCESSING / STORAGE
  ↓
OPTIONAL ENCRYPTED SYNC
  ↓
HOSTED STORAGE / PROCESSING (only if policy permits)
  ↓
PERMISSION-AWARE RETRIEVAL
  ↓
AGENT CONTEXT
```

Required policy fields should include:

- `data_classification`
- `owner_type` / `owner_id`
- `tenant_id` / `workspace_id` / `project_id`
- `sync_mode`
- `processing_mode`
- `retention_policy_id`
- `residency_region`
- `allowed_processors` / model providers
- `source_exclusions`
- `secret_handling_mode`
- `audit_level`
- `policy_version`

Policy evaluation must happen before cloud upload, hosted inference, retrieval candidate generation, graph expansion across scopes, and context export. This gives enterprise administrators a single control point for storage and processing rules.

The implementation should be compliance-ready rather than hard-coded to one regulation. Jurisdiction-specific requirements can be represented through policy bundles and deployment-region configuration.

## Enterprise controls

Enterprise workspaces should support:

- tenant isolation
- workspace-level access controls
- device revocation
- data retention policies
- deletion workflows
- audit logs
- regional storage selection
- encryption at rest and in transit
- organization-managed identity/SSO integration as a later extension
- dedicated workers or storage for large tenants
- configurable cloud-processing policy

## Graph scaling policy

Do not introduce Neo4j or FalkorDB into the first hosted release simply because the local edition has a graph layer.

Start with PostgreSQL relational edges and recursive retrieval. Introduce a dedicated graph service only when measured hosted workloads show that multi-hop traversal, graph concurrency or graph analytics has become the bottleneck.

If that occurs, implement the graph adapter behind the same `GraphStore` interface used locally.

## Hosted retrieval

The hosted service should expose the same conceptual pipeline:

```text
query
 → permission filtering
 → lexical / vector / graph candidate generation
 → RRF
 → context-aware reranking
 → temporal validity
 → context packing
 → evidence + provenance
```

The Rust ranking and context algorithms should remain shared wherever practical. Storage and transport adapters are what change.

## Privacy contract

The product documentation should state clearly:

1. Local mode does not require cloud storage.
2. Cloud sync is opt-in.
3. Cloud-synced data belongs to a user's/workspace's encrypted replica according to the service's retention policy.
4. Hosted processing is separately controlled from cloud storage.
5. Local data remains available offline.
6. Sign-out does not automatically erase the local brain; a deliberate device-data removal action is required.

---

## 4-Tier Implementation Layer Architecture

### 1. Frontend Tier: Web Control Plane & Desktop Sync Cards
- **Web App (`orbit.example.com`)**: Next.js / React web portal for account creation, team management, billing, and device registration.
- **Desktop Sync Status UI (`desktop/src/features/settings/ui/HostedCommunitiesSettingsCard.tsx`)**:
  - Displays cloud connection status, last sync timestamp, and device count.
  - "Sync Now" action and cloud policy viewer.

### 2. Desktop Backend Tier: Tauri Deep-Link & Outbox Manager
- **OAuth PKCE Client (`desktop/src-tauri/src/deep_link.rs`)**:
  - Handles `orbit://oauth/callback` deep link to exchange single-use authorization code for native session credentials.
  - Stores session token in OS hardware keyring via `buzz-auth`.
- **Sync Outbox Worker**:
  - Encrypts and batches local mutation log items (`sync/changelog.jsonl`) for transmission to the hosted relay.

### 3. Core Workspace Crates Tier (`crates/buzz-relay`, `crates/buzz-auth`, `crates/buzz-db`)
- `crates/buzz-relay`: Multi-tenant NIP-29 WebSocket relay server handling authenticated event streaming, presence, and sync.
- `crates/buzz-auth`: Cryptographic key verification, NIP-42 authentication, and role-based access control.
- `crates/buzz-db`: PostgreSQL 16 + pgvector enterprise storage adapter.

### 4. Packaging, Enterprise & Container Tier
- **Container Deployment**: Docker Compose (`docker-compose.yml`) and Kubernetes Helm charts deploying `buzz-relay`, PostgreSQL 16, and Redis pub/sub.
- **Client Independence**: The native desktop app remains 100% functional without containers when running locally.


