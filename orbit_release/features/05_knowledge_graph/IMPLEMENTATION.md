# Feature 05 — Knowledge Graph (Entities + Bi-Temporal Relations)

> **Priority**: P1  
> **Storage**: Embedded Ladybug/Kuzu-compatible graph backend  
> **Dependencies**: Feature 01, Feature 03

## Overview

The graph is not the primary document store. It is a **reasoning index** over durable entities and relations.

ORBIT should use the graph for questions such as:

- “What depends on this service?”
- “Which files implement this decision?”
- “What changed after this architecture decision?”
- “Which sessions discussed this component?”
- “What concepts are connected to this symbol?”

Full source text and embeddings remain in SQLite/LanceDB.

## Graph model

### Nodes

- `Workspace`
- `Project`
- `File`
- `Symbol`
- `Session`
- `Agent`
- `Entity`
- `Decision`
- `Technology`
- `Concept`

### Edges

- `CONTAINS`
- `DEFINES`
- `IMPORTS`
- `DEPENDS_ON`
- `DISCUSSED_IN`
- `DECIDED_BY`
- `SUPERSEDES`
- `RELATES_TO`
- `DERIVED_FROM`

### Temporal fields

Each durable relation carries:

- `valid_at`
- `invalid_at`
- `observed_at`
- `confidence`
- `evidence_id`

## Why not Neo4j for desktop V1?

Neo4j is a capable graph system, including local development/embedded options, but its current desktop/local model is still a dedicated DBMS-oriented runtime rather than the lightest fit for a Rust/Tauri product. Neo4j documentation also positions Desktop as a development environment rather than a production embedded runtime. citeturn684197search0turn684197search8

Therefore Neo4j should be an **optional remote/enterprise adapter**, not the hard dependency of the local brain.

## Why not FalkorDB for desktop V1?

FalkorDB has an official Rust client and an embedded mode, but its embedded mode launches a Redis server plus the FalkorDB module and requires native module provisioning. The server itself is SSPL-licensed, and the client documentation notes that the embedded bundle embeds that SSPL module. That makes it a poor default for a distributable local app where binary simplicity and licensing clarity matter. citeturn830492search0turn830492search3

FalkorDB remains a strong **remote/shared graph adapter** for later deployments.

## Graph traversal

Implement bounded traversal in the storage adapter and expose only normalized results to `buzz-search`:

```rust
pub struct GraphHit {
    pub entity_id: EntityId,
    pub distance: u8,
    pub relation_path: Vec<RelationType>,
    pub evidence_ids: Vec<SourceId>,
    pub confidence: f32,
}
```

Default traversal:

- seed expansion: 1 hop
- reasoning expansion: max 2 hops
- retry path: max 3 hops
- hard node cap: configurable

## Verification & Quality Gates

- [ ] Entity upsert is deterministic
- [ ] Duplicate entities merge correctly
- [ ] Invalidated facts remain auditable
- [ ] 2-hop queries return provenance IDs
- [ ] Graph failures degrade to vector + lexical retrieval
