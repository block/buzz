# Orbit Knowledge Graph Enterprise Adapters

> **Status**: Specification & Architectural Boundary  
> **Target Deployments**: Remote Enterprise Relays, Multi-User Teams, Shared Knowledge Clusters  
> **Core Contract**: `buzz_db::memory::traits::GraphStore`

---

## 1. Architectural Boundary Principle

To ensure modularity and zero vendor lock-in, the knowledge graph system in Orbit is strictly decoupled from storage implementations:

1. **Zero Vendor Leaks**:
   - Neither `buzz-search` (SuperRAG), `buzz-ai`, nor `buzz-core` import any vendor-specific client libraries (e.g., `neo4rs`, `falkordb`, or raw Cypher/Bolt types).
   - All consumption occurs through `dyn GraphStore` returning standardized `buzz_core::memory::GraphHit`, `Entity`, and `Relation` domain models.

2. **Standardized Graph Traversal Interface**:
   ```rust
   pub trait GraphStore: Send + Sync + Debug {
       fn upsert_entities(&self, entities: &[Entity]) -> impl Future<Output = Result<()>> + Send;
       fn upsert_relations(&self, relations: &[Relation]) -> impl Future<Output = Result<()>> + Send;
       fn neighborhood_as_of(
           &self,
           seed_ids: &[Uuid],
           hops: u8,
           as_of: Option<DateTime<Utc>>,
           max_nodes: Option<usize>,
       ) -> impl Future<Output = Result<Vec<GraphHit>>> + Send;
       fn invalidate_relation(&self, id: &Uuid, invalid_at: Option<DateTime<Utc>>) -> impl Future<Output = Result<bool>> + Send;
       fn resolve_contradiction(
           &self,
           workspace_path: &str,
           source_id: &Uuid,
           target_id: &Uuid,
           relation_type: &str,
           replacement: &Relation,
       ) -> impl Future<Output = Result<usize>> + Send;
   }
   ```

---

## 2. Desktop V1 Backend: Embedded Engine

- **Default Implementation**: `EmbeddedGraphStore` (`~/.orbit/brain/graph/graph.bin`).
- **Characteristics**:
  - Pure Rust crash-safe atomic journal.
  - Zero external daemons, zero network socket overhead, sub-millisecond BFS hop traversal.
  - Full bi-temporal validity checking (`valid_at`, `invalid_at`).
  - Automatic duplicate node deduplication and JSON metadata merging.

---

## 3. Remote Adapter: Neo4j

### Why Not Default for Desktop V1?
- Neo4j requires a dedicated JVM DBMS runtime. Running a Java process alongside Tauri/Rust imposes substantial memory footprint (~1 GB JVM heap minimum) and complex distribution packaging.
- Neo4j Desktop is positioned by its vendor for developer environments rather than embedded local distribution.

### Enterprise Remote Implementation
- **Crate Boundary**: `crates/buzz-graph-neo4j` (optional feature flag `enterprise-neo4j`).
- **Connection Protocol**: Bolt protocol via `neo4rs` connection pool.
- **Cypher Translation**:
  - Node creation:
    ```cypher
    MERGE (e:OrbitEntity { id: $id })
    ON CREATE SET e.workspace = $ws, e.name = $name, e.type = $type, e.description = $desc, e.metadata = $meta
    ON MATCH SET e.description = e.description + "; " + $desc, e.metadata = $merged_meta
    ```
  - Traversal with bi-temporal fence:
    ```cypher
    MATCH (seed:OrbitEntity { id: $seed_id })-[r:RELATION*1..$hops]-(neighbor:OrbitEntity)
    WHERE ALL(rel in r WHERE rel.valid_at <= $as_of AND (rel.invalid_at IS NULL OR rel.invalid_at > $as_of))
    RETURN neighbor, r, length(r) as depth LIMIT $max_nodes
    ```

---

## 4. Remote Adapter: FalkorDB

### Why Not Default for Desktop V1?
- FalkorDB embedded mode requires orchestrating an underlying Redis server process and loading a compiled C native module.
- Licensing: The FalkorDB server and embedded module are distributed under SSPL, creating distribution and compliance restrictions for commercial client desktop binaries.

### Enterprise Remote Implementation
- **Crate Boundary**: `crates/buzz-graph-falkordb` (optional feature flag `enterprise-falkordb`).
- **Connection Protocol**: RESP3 protocol over TLS to managed FalkorDB cloud or shared internal cluster.
- **OpenCypher Mapping**:
  - Translates `GraphStore` calls to FalkorDB OpenCypher procedures `GRAPH.QUERY <graph_id> <cypher>`.
  - Normalizes results into standard `Vec<GraphHit>` without passing graph matrices or Redis protocol frames to the caller.

---

## 5. Verification & Fallback Guarantee

If any remote adapter fails due to network partition or connection failure, Orbit’s SuperRAG layer provides guaranteed graceful degradation:
- Search automatically falls back to dense vector nearest-neighbors and SQLite FTS lexical hits.
- Zero search queries fail when the graph store is unreachable or empty.
