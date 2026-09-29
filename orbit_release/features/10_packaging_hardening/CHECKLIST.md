# Feature 10 — Packaging & Hardening Checklist

### 1. `orbit_brain/` storage

- [ ] `db/orbit.db` created automatically
- [ ] `vectors/` created automatically
- [ ] `graph/` created automatically
- [ ] `models/`, `transcripts/`, `exports/`, `sync/` created automatically

### 2. Native desktop runtime

- [ ] No PostgreSQL binary bundled
- [ ] No Redis binary bundled
- [ ] No Neo4j/FalkorDB server bundled
- [ ] No Docker dependency

### 3. Sync

- [ ] `sync/changelog.jsonl` records logical mutations
- [ ] Snapshot metadata tracks replication watermarks
- [ ] Replay test reconstructs a clean brain
- [ ] Database WAL files are never used as the cross-device sync protocol

### 4. Packaging

- [ ] Windows installer builds
- [ ] Windows portable build builds
- [ ] macOS `.app` builds
- [ ] macOS `.dmg` builds

### 5. Benchmarking

- [ ] Cold-start benchmark
- [ ] vector retrieval benchmark
- [ ] graph traversal benchmark
- [ ] hybrid recall benchmark
- [ ] memory/disk footprint benchmark
- [ ] restart/recovery benchmark

## Verification & Sign-off

- [ ] `just ci`
- [ ] clean-machine installation test on Windows
- [ ] clean-machine installation test on macOS
