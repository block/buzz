# Feature 01 — Storage Foundation Checklist

## Deliverables & Tasks

### 1. Embedded storage substrate

- [x] SQLite metadata database opens from `~/.orbit/brain/db/orbit.db`
- [x] LanceDB vector store opens from `~/.orbit/brain/vectors/`
- [x] Ladybug/Kuzu graph store opens from `~/.orbit/brain/graph/`
- [x] No PostgreSQL process is required for desktop startup

### 2. Rust storage abstractions

- [x] `MetadataStore` trait implemented
- [x] `VectorStore` trait implemented
- [x] `GraphStore` trait implemented
- [x] `MemoryStore` facade exposes a stable ORBIT API
- [x] Vendor-specific types do not leak into higher-level crates

### 3. Memory consistency

- [x] Every vector row maps to an authoritative `chunk_id`
- [x] Every graph node/edge maps to provenance IDs
- [x] Content-hash deduplication is idempotent
- [x] Deletion/forget operations propagate to all stores
- [x] Crash/restart recovery tests pass

### 4. Security

- [x] Secret redactor runs before persistent storage
- [x] Workspace scoping is enforced at retrieval time
- [x] File ACL metadata is preserved
- [x] Sensitive settings remain in OS keyring

## Verification & Sign-off

- [x] `cargo test -p buzz-db`
- [x] Embedded-store reopen test passes
- [x] 10k-chunk smoke benchmark passes
- [x] Clean machine test confirms zero external DB setup
