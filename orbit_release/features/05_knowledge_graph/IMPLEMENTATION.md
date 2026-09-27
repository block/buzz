# Feature 05 — Knowledge Graph (Entities + Bi-Temporal Relations)

> **Priority**: P1 — Structural memory and entity relationship understanding.  
> **Sprint**: Sprint 3 (Weeks 5–6)  
> **Dependencies**: Feature 01 (Storage), Feature 03 (Ingestion)  
> **Crates**: `orbit-core` (entity types and graph logic), `buzz-db` (persistence)  
> **Environment Variables**: `BUZZ_DATABASE_URL`

---

## Overview

Feature 05 constructs a bi-temporal knowledge graph (inspired by the Graphiti architecture) inside `orbit-core` and `buzz-db`. It extracts named entities (technologies, files, people, architectural concepts) and typed edges from ingested chunks, supports temporal invalidation when facts change, and provides 2-hop graph traversal to enrich Multi-RAG search and power the desktop Obsidian-style brain graph.

---

## Architecture & Graph Model

### 1. Bi-Temporal Edge Model

Every relationship in `buzz_relations` tracks three distinct timestamps:
- `valid_at`: When the fact became true in the real world.
- `invalid_at`: When the fact was contradicted or superseded (`NULL` means currently active).
- `recorded_at`: When Orbit recorded the assertion.

```
Time T1: "Alice authors buzz-relay" (valid_at = T1, invalid_at = NULL)
Time T2: "Bob takes over buzz-relay"
         -> Update old edge: invalid_at = T2
         -> Insert new edge: "Bob authors buzz-relay" (valid_at = T2, invalid_at = NULL)
```

### 2. Entity & Relation Types

- **Entity Types**:
  - `Technology`: Rust, PostgreSQL, pgvector, React 19, Tauri 2
  - `Person`: Authors, contributors, reviewers
  - `File`: Path references in repository
  - `Concept`: NIP-29, Multi-RAG, Secret Redaction
  - `Architecture`: Crates, microservices, databases
- **Relation Types**:
  - `implements`, `depends_on`, `decided_by`, `authored_by`, `mentions`, `supersedes`

### 3. 2-Hop Neighborhood Traversal Query

```sql
WITH RECURSIVE graph_walk AS (
    -- Seed: Entities directly matching the chunk or query
    SELECT 
        e.id, e.name, e.entity_type, e.description, 0 AS depth
    FROM buzz_entities e
    WHERE e.workspace_path = $1 AND e.id = ANY($2::uuid[])

    UNION ALL

    -- Traversal: 1 and 2 hops away along active relations
    SELECT 
        target.id, target.name, target.entity_type, target.description, gw.depth + 1
    FROM graph_walk gw
    JOIN buzz_relations r ON r.source_entity_id = gw.id AND r.invalid_at IS NULL
    JOIN buzz_entities target ON target.id = r.target_entity_id
    WHERE gw.depth < 2
)
SELECT DISTINCT id, name, entity_type, description, depth FROM graph_walk;
```

---

## Verification & Quality Gates

- Unit test: verify bi-temporal contradiction resolution updates `invalid_at` instead of deleting records.
- Integration test: verify 2-hop neighborhood expansion retrieves connected entities.
- Run `cargo test -p orbit-core`.
- Run `just ci`.
