# Feature 01 — Storage Foundation Checklist

## Deliverables & Tasks

### 1. Embedded storage substrate

- [ ] SQLite metadata database opens from `orbit_brain/db/orbit.db`
- [ ] LanceDB vector store opens from `orbit_brain/vectors/`
- [ ] Ladybug/Kuzu graph store opens from `orbit_brain/graph/`
- [ ] No PostgreSQL process is required for desktop startup

### 2. Rust storage abstractions

- [ ] `MetadataStore` trait implemented
- [ ] `VectorStore` trait implemented
- [ ] `GraphStore` trait implemented
- [ ] `MemoryStore` facade exposes a stable ORBIT API
- [ ] Vendor-specific types do not leak into higher-level crates

### 3. Memory consistency

- [ ] Every vector row maps to an authoritative `chunk_id`
- [ ] Every graph node/edge maps to provenance IDs
- [ ] Content-hash deduplication is idempotent
- [ ] Deletion/forget operations propagate to all stores
- [ ] Crash/restart recovery tests pass

### 4. Security

- [ ] Secret redactor runs before persistent storage
- [ ] Workspace scoping is enforced at retrieval time
- [ ] File ACL metadata is preserved
- [ ] Sensitive settings remain in OS keyring

## Verification & Sign-off

- [ ] `cargo test -p buzz-db`
- [ ] Embedded-store reopen test passes
- [ ] 10k-chunk smoke benchmark passes
- [ ] Clean machine test confirms zero external DB setup
