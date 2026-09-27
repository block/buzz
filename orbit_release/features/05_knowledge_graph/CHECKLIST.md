# Feature 05 — Knowledge Graph Checklist

> **Directory**: `orbit_release/features/05_knowledge_graph/`  
> **Status**: `[ ] In Progress / Ready for Implementation`  
> **Estimated Effort**: Sprint 3 (Weeks 5–6)

---

## Deliverables & Tasks

### 1. `orbit-core` Graph Engine Setup
- [ ] Define `Entity` and `Relation` domain models
- [ ] Implement `EntityType` enum (`Technology`, `Person`, `File`, `Concept`, `Architecture`)
- [ ] Implement `RelationType` enum (`implements`, `depends_on`, `decided_by`, `authored_by`, `mentions`, `supersedes`)

### 2. Entity & Relation Extraction
- [ ] Implement heuristic & regex-based entity extractor for code symbols and file references
- [ ] Implement relation extractor for imports, function calls, and markdown references
- [ ] Integrate with `orbit-ai` to generate `summary_embedding` for entities

### 3. Bi-Temporal Engine
- [ ] Implement relation insertion with `valid_at = NOW()` and `invalid_at = NULL`
- [ ] Implement `supersede_relation(old_id, new_id)` setting `invalid_at`
- [ ] Implement contradiction detection for incompatible facts

### 4. 2-Hop Graph Traversal
- [ ] Implement SQL recursive CTE query for 2-hop neighborhood expansion
- [ ] Filter traversal to active relations (`invalid_at IS NULL`)
- [ ] Integrate graph entities into Multi-RAG search Layer 3 scoring (`w_graph`)
- [ ] Expose graph endpoints for the Desktop Brain Panel (Feature 09)

---

## Verification & Sign-off

- [ ] `cargo test -p orbit-core` passes
- [ ] Historical audit trail preserved when relations are superseded
- [ ] 2-hop queries return within <10ms for graphs up to 50,000 entities
- [ ] `just ci` passes cleanly
