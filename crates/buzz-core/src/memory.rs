#![deny(unsafe_code)]
//! Core domain models for Orbit long-term memory.
//!
//! Provides zero-I/O data types for documents, chunks, entities, relations,
//! vector embeddings, and memory feedback.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Source classification for an ingested document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    /// Local file from repository or workspace.
    File,
    /// Agent conversation or interaction session.
    Session,
    /// Git commit history or diff.
    Git,
    /// Slack channel message or thread.
    Slack,
    /// GitHub issue or pull request.
    Github,
    /// GitLab merge request or issue.
    Gitlab,
    /// Jira issue or sprint requirement.
    Jira,
    /// Other external source.
    Other(String),
}

impl SourceType {
    /// Returns the string representation.
    pub fn as_str(&self) -> &str {
        match self {
            Self::File => "file",
            Self::Session => "session",
            Self::Git => "git",
            Self::Slack => "slack",
            Self::Github => "github",
            Self::Gitlab => "gitlab",
            Self::Jira => "jira",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl From<&str> for SourceType {
    fn from(s: &str) -> Self {
        match s {
            "file" => Self::File,
            "session" => Self::Session,
            "git" => Self::Git,
            "slack" => Self::Slack,
            "github" => Self::Github,
            "gitlab" => Self::Gitlab,
            "jira" => Self::Jira,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Compute canonical SHA-256 hash for content deduplication and staleness detection.
pub fn compute_content_hash(content: impl AsRef<[u8]>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_ref());
    hex::encode(hasher.finalize())
}

/// Layer 1: Raw immutable source document metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// Unique document identifier.
    pub id: Uuid,
    /// Source classification (`file`, `session`, `git`, etc.).
    pub source_type: String,
    /// Absolute local path or remote URI.
    pub source_uri: String,
    /// Workspace boundary for tenant/isolation scoping.
    pub workspace_path: String,
    /// SHA-256 content digest for deduplication.
    pub content_hash: String,
    /// File modification time in nanoseconds.
    pub mtime_ns: i64,
    /// Byte size of the source content.
    pub size_bytes: i64,
    /// ACL and permission metadata (default `{"public": true}`).
    pub permissions: serde_json::Value,
    /// Arbitrary source metadata (commit SHA, author, branch, etc.).
    pub metadata: serde_json::Value,
    /// Timestamp when document was ingested.
    pub created_at: DateTime<Utc>,
    /// Timestamp when document was last updated.
    pub updated_at: DateTime<Utc>,
}

impl Document {
    /// Create a new document with generated ID and computed content hash.
    pub fn new(
        source_type: impl Into<String>,
        source_uri: impl Into<String>,
        workspace_path: impl Into<String>,
        content: &[u8],
        mtime_ns: i64,
        metadata: serde_json::Value,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            source_type: source_type.into(),
            source_uri: source_uri.into(),
            workspace_path: workspace_path.into(),
            content_hash: compute_content_hash(content),
            mtime_ns,
            size_bytes: content.len() as i64,
            permissions: serde_json::json!({ "public": true }),
            metadata,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Layer 2: Processed, normalized context chunk.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique chunk identifier.
    pub id: Uuid,
    /// Foreign key to parent document.
    pub document_id: Uuid,
    /// Workspace boundary for isolation.
    pub workspace_path: String,
    /// Sequence index within the source document.
    pub chunk_index: i32,
    /// Normalized chunk text content.
    pub content: String,
    /// Estimated LLM token count.
    pub token_count: i32,
    /// Privacy scope (`project`, `global`, `agent_private`).
    pub scope: String,
    /// Ingesting or generating agent identifier.
    pub agent_name: Option<String>,
    /// Starting line number in source file.
    pub line_start: Option<i32>,
    /// Ending line number in source file.
    pub line_end: Option<i32>,
    /// Ingestion timestamp.
    pub created_at: DateTime<Utc>,
}

impl Chunk {
    /// Create a new chunk tied to a document.
    pub fn new(
        document_id: Uuid,
        workspace_path: impl Into<String>,
        chunk_index: i32,
        content: impl Into<String>,
        scope: impl Into<String>,
    ) -> Self {
        let text = content.into();
        // ponytail: fast heuristic token estimate (~4 chars per token), full tokenizer in F02
        let estimated_tokens = (text.len() / 4).max(1) as i32;
        Self {
            id: Uuid::new_v4(),
            document_id,
            workspace_path: workspace_path.into(),
            chunk_index,
            content: text,
            token_count: estimated_tokens,
            scope: scope.into(),
            agent_name: None,
            line_start: None,
            line_end: None,
            created_at: Utc::now(),
        }
    }
}

/// Layer 2 vector embedding record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingRecord {
    /// Authoritative chunk identifier.
    pub chunk_id: Uuid,
    /// Parent document identifier.
    pub document_id: Uuid,
    /// Workspace boundary for tenant isolation.
    pub workspace_path: String,
    /// Dense vector representation (typically 384 dimensions for bge-small).
    pub embedding: Vec<f32>,
    /// Cached text content for instant retrieval without secondary lookup.
    pub content: String,
    /// Record creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Filtering criteria for vector search queries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorFilter {
    /// Filter by workspace boundary.
    pub workspace_path: Option<String>,
    /// Filter by document identifier.
    pub document_id: Option<Uuid>,
    /// Filter by privacy scope.
    pub scope: Option<String>,
}

/// Retrieval result from vector nearest-neighbor search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorHit {
    /// Authoritative chunk identifier.
    pub chunk_id: Uuid,
    /// Parent document identifier.
    pub document_id: Uuid,
    /// Similarity score (higher is more similar, typically cosine similarity).
    pub score: f32,
    /// Text snippet content.
    pub content: String,
    /// Workspace path.
    pub workspace_path: String,
}

/// Layer 3: Semantic knowledge graph entity node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    /// Unique entity identifier.
    pub id: Uuid,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Entity name (e.g. `DatabasePool`, `SuperRAG`).
    pub name: String,
    /// Entity type (`Technology`, `Person`, `File`, `Concept`, `Architecture`).
    pub entity_type: String,
    /// Synthesized description.
    pub description: String,
    /// Metadata attributes (aliases, confidence, source references).
    pub metadata: serde_json::Value,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

impl Entity {
    /// Create a new entity with generated ID.
    pub fn new(
        workspace_path: impl Into<String>,
        name: impl Into<String>,
        entity_type: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_path: workspace_path.into(),
            name: name.into(),
            entity_type: entity_type.into(),
            description: description.into(),
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    /// Generates a deterministic UUID from workspace, entity_type, and name using SHA-256.
    pub fn deterministic_id(workspace_path: &str, entity_type: &str, name: &str) -> Uuid {
        let normalized = format!("{}:{}:{}", workspace_path.trim(), entity_type.trim(), name.trim().to_ascii_lowercase());
        let mut hasher = sha2::Sha256::new();
        hasher.update(normalized.as_bytes());
        let hash = hasher.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&hash[..16]);
        // Set RFC 4122 version 5 and variant bits
        bytes[6] = (bytes[6] & 0x0f) | 0x50;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Uuid::from_bytes(bytes)
    }

    /// Creates a new entity with a deterministic ID.
    pub fn new_deterministic(
        workspace_path: impl Into<String>,
        name: impl Into<String>,
        entity_type: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let ws = workspace_path.into();
        let nm = name.into();
        let et = entity_type.into();
        let id = Self::deterministic_id(&ws, &et, &nm);
        let now = Utc::now();
        Self {
            id,
            workspace_path: ws,
            name: nm,
            entity_type: et,
            description: description.into(),
            metadata: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    /// Merges another entity into this one (combining descriptions, metadata, updating timestamp).
    pub fn merge_with(&mut self, other: &Entity) {
        if self.description.is_empty() {
            self.description = other.description.clone();
        } else if !other.description.is_empty() && !self.description.contains(&other.description) {
            self.description = format!("{}; {}", self.description, other.description);
        }

        // Merge JSON metadata objects if both are objects
        if let (Some(self_obj), Some(other_obj)) = (self.metadata.as_object_mut(), other.metadata.as_object()) {
            for (k, v) in other_obj {
                if !self_obj.contains_key(k) {
                    self_obj.insert(k.clone(), v.clone());
                }
            }
        }

        self.updated_at = Utc::now();
    }
}

/// Standardized knowledge graph node types.
pub struct EntityType;
impl EntityType {
    /// Workspace root container.
    pub const WORKSPACE: &'static str = "Workspace";
    /// Project / crate / module boundary.
    pub const PROJECT: &'static str = "Project";
    /// Source code or documentation file.
    pub const FILE: &'static str = "File";
    /// Code symbol (fn, struct, enum, trait, interface).
    pub const SYMBOL: &'static str = "Symbol";
    /// Interactive session or conversation turn.
    pub const SESSION: &'static str = "Session";
    /// AI agent persona or subagent.
    pub const AGENT: &'static str = "Agent";
    /// Architectural or domain entity.
    pub const ENTITY: &'static str = "Entity";
    /// Architecture Decision Record (ADR).
    pub const DECISION: &'static str = "Decision";
    /// Technology, library, or dependency.
    pub const TECHNOLOGY: &'static str = "Technology";
    /// Conceptual topic or pattern.
    pub const CONCEPT: &'static str = "Concept";
}

/// Standardized knowledge graph edge relation types.
pub struct RelationType;
impl RelationType {
    /// Structural containment (e.g. Workspace -> Project -> File).
    pub const CONTAINS: &'static str = "CONTAINS";
    /// Definition of a symbol within a file (File -> Symbol).
    pub const DEFINES: &'static str = "DEFINES";
    /// Import / usage dependency (File -> File/Technology).
    pub const IMPORTS: &'static str = "IMPORTS";
    /// Functional or structural dependency (Symbol -> Symbol, Project -> Technology).
    pub const DEPENDS_ON: &'static str = "DEPENDS_ON";
    /// Discussion or mention within a session (Session -> Entity/Symbol).
    pub const DISCUSSED_IN: &'static str = "DISCUSSED_IN";
    /// Architectural decision authorship (Decision -> File/Symbol).
    pub const DECIDED_BY: &'static str = "DECIDED_BY";
    /// Temporal replacement or invalidation (Decision -> Decision, Technology -> Technology).
    pub const SUPERSEDES: &'static str = "SUPERSEDES";
    /// Conceptual association (Concept -> Concept, Entity -> Entity).
    pub const RELATES_TO: &'static str = "RELATES_TO";
    /// Origin or provenance derivation (Entity -> Source/Chunk).
    pub const DERIVED_FROM: &'static str = "DERIVED_FROM";
}

/// Layer 3: Bi-temporal relationship edge between entities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    /// Unique relation identifier.
    pub id: Uuid,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Source entity identifier.
    pub source_entity_id: Uuid,
    /// Target entity identifier.
    pub target_entity_id: Uuid,
    /// Typed relationship (`implements`, `depends_on`, `decided_by`, `supersedes`).
    pub relation_type: String,
    /// Confidence score (0.0 to 1.0).
    pub confidence: f32,
    /// When this fact became true in the real world.
    pub valid_at: DateTime<Utc>,
    /// When this fact was invalidated or superseded (`None` = active).
    pub invalid_at: Option<DateTime<Utc>>,
    /// When the system learned this fact.
    pub recorded_at: DateTime<Utc>,
    /// Provenance link to source chunk.
    pub provenance_chunk_id: Option<Uuid>,
    /// Extra metadata.
    pub metadata: serde_json::Value,
}

impl Relation {
    /// Create a new active relation edge.
    pub fn new(
        workspace_path: impl Into<String>,
        source_entity_id: Uuid,
        target_entity_id: Uuid,
        relation_type: impl Into<String>,
        confidence: f32,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            workspace_path: workspace_path.into(),
            source_entity_id,
            target_entity_id,
            relation_type: relation_type.into(),
            confidence,
            valid_at: now,
            invalid_at: None,
            recorded_at: now,
            provenance_chunk_id: None,
            metadata: serde_json::json!({}),
        }
    }

    /// Check if this relation is currently active (not superseded/invalidated).
    pub fn is_active(&self) -> bool {
        self.invalid_at.is_none()
    }

    /// Checks if this relation is valid at a specific timestamp.
    pub fn is_valid_at(&self, as_of: DateTime<Utc>) -> bool {
        self.valid_at <= as_of && self.invalid_at.map(|inv| inv > as_of).unwrap_or(true)
    }

    /// Invalidates this relation as of a specific timestamp (default: now).
    pub fn invalidate(&mut self, invalid_at: Option<DateTime<Utc>>) {
        self.invalid_at = Some(invalid_at.unwrap_or_else(Utc::now));
    }
}

/// Graph neighborhood hit during traversal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphHit {
    /// Discovered entity node.
    pub entity: Entity,
    /// Traversed edge leading to this entity, if applicable.
    pub relation: Option<Relation>,
    /// Traversal hop distance from seed node.
    pub depth: u8,
}

/// User or agent feedback on a retrieved memory item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryFeedback {
    /// Unique feedback identifier.
    pub id: Uuid,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Target memory/chunk/entity identifier.
    pub memory_id: Uuid,
    /// Rating score (-1 for downvote, +1 for upvote).
    pub user_rating: i32,
    /// Optional feedback notes.
    pub feedback_text: Option<String>,
    /// Feedback submission timestamp.
    pub created_at: DateTime<Utc>,
}

/// Ingested source definition (git repository, folder, Slack channel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    /// Unique source identifier.
    pub id: Uuid,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Source type (`file`, `session`, `git`, `slack`, etc.).
    pub source_type: String,
    /// Source URI or filesystem path.
    pub uri: String,
    /// Status (`active`, `paused`, `error`).
    pub status: String,
    /// Last synced timestamp.
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Registration timestamp.
    pub created_at: DateTime<Utc>,
}

/// Historical agent interactive session record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    /// Unique session identifier.
    pub id: Uuid,
    /// Agent identifier (e.g. `antigravity`, `claude_code`, `cursor`).
    pub agent_name: String,
    /// Workspace boundary.
    pub workspace_path: String,
    /// High-level interaction summary.
    pub summary: String,
    /// Total conversation turns.
    pub turns_count: i32,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Architectural decision record (ADR).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    /// Unique decision identifier.
    pub id: Uuid,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Decision title.
    pub title: String,
    /// Status (`proposed`, `accepted`, `superseded`, `rejected`).
    pub state: String,
    /// Rationale and motivation.
    pub rationale: String,
    /// Optional successor decision ID.
    pub superseded_by: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Cloud and team event synchronization log entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncLogEntry {
    /// Unique event identifier.
    pub id: Uuid,
    /// Event type (`document_upsert`, `memory_delete`, etc.).
    pub event_type: String,
    /// Serialized JSON mutation payload.
    pub payload: String,
    /// Synchronization flag (true if delivered to remote relay).
    pub synced: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Background ingestion job status and progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestionJob {
    /// Unique job identifier.
    pub id: Uuid,
    /// Monitored workspace path.
    pub workspace_path: String,
    /// Status (`running`, `completed`, `failed`).
    pub status: String,
    /// Total files discovered.
    pub total_files: i32,
    /// Processed files count.
    pub processed_files: i32,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Completion timestamp.
    pub completed_at: Option<DateTime<Utc>>,
}

/// Layer 5 active working context for an agent task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingContext {
    /// Unique working context identifier.
    pub id: Uuid,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Subagent or task identifier.
    pub task_id: String,
    /// Agent identifier.
    pub agent_name: String,
    /// Target token budget (e.g. 8192).
    pub token_budget: i32,
    /// Allocated tokens in XML payload.
    pub allocated_tokens: i32,
    /// Active memory item IDs.
    pub active_memory_ids: Vec<Uuid>,
    /// Active chunk IDs.
    pub active_chunk_ids: Vec<Uuid>,
    /// Formatted `<orbit_context>` XML.
    pub context_xml: String,
    /// Active context flag.
    pub is_active: bool,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// Layer 4 compiled query routing and semantic cache entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryCacheEntry {
    /// SHA-256 digest of normalized query.
    pub query_hash: String,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Intent classification (`factual`, `code_symbol`, `architectural`).
    pub intent_type: String,
    /// Candidate chunk IDs.
    pub candidate_chunk_ids: Vec<Uuid>,
    /// Reranked top chunk IDs.
    pub reranked_chunk_ids: Vec<Uuid>,
    /// Compiled context token count.
    pub compiled_tokens: i32,
    /// Cache hit frequency.
    pub hit_count: i32,
    /// Expiration timestamp.
    pub expires_at: DateTime<Utc>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

/// Durable memory items (ADRs, guidelines, distilled insights).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryItem {
    /// Unique memory item identifier.
    pub id: Uuid,
    /// Workspace boundary.
    pub workspace_path: String,
    /// Item title.
    pub title: String,
    /// Distilled factual or guidance content.
    pub content: String,
    /// Associated tags.
    pub tags: Vec<String>,
    /// Privacy scope.
    pub scope: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_content_hash() {
        let h1 = compute_content_hash(b"hello orbit");
        let h2 = compute_content_hash(b"hello orbit");
        let h3 = compute_content_hash(b"different content");

        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn test_document_and_chunk_creation() {
        let doc = Document::new(
            "file",
            "/path/to/code.rs",
            "/workspace",
            b"fn main() {}",
            1_000_000,
            serde_json::json!({"lang": "rust"}),
        );
        assert_eq!(doc.source_type, "file");
        assert_eq!(doc.size_bytes, 12);

        let chunk = Chunk::new(doc.id, &doc.workspace_path, 0, "fn main() {}", "project");
        assert_eq!(chunk.document_id, doc.id);
        assert_eq!(chunk.chunk_index, 0);
    }

    #[test]
    fn test_relation_active_status() {
        let src = Uuid::new_v4();
        let dst = Uuid::new_v4();
        let mut rel = Relation::new("/workspace", src, dst, "depends_on", 0.95);
        assert!(rel.is_active());

        rel.invalid_at = Some(Utc::now());
        assert!(!rel.is_active());
    }

    #[test]
    fn test_deterministic_entity_id_and_merge() {
        let ws = "/projects/orbit";
        let id1 = Entity::deterministic_id(ws, EntityType::TECHNOLOGY, "SQLite");
        let id2 = Entity::deterministic_id(ws, EntityType::TECHNOLOGY, "sqlite");
        assert_eq!(id1, id2, "Deterministic ID must be case-insensitive for entity name");

        let mut ent1 = Entity::new_deterministic(ws, "SQLite", EntityType::TECHNOLOGY, "Embedded database");
        assert_eq!(ent1.id, id1);

        let ent2 = Entity::new_deterministic(ws, "SQLite", EntityType::TECHNOLOGY, "Fast local storage");
        ent1.merge_with(&ent2);
        assert!(ent1.description.contains("Embedded database"));
        assert!(ent1.description.contains("Fast local storage"));
    }

    #[test]
    fn test_bi_temporal_relation_invalidation() {
        let now = Utc::now();
        let src = Uuid::new_v4();
        let dst = Uuid::new_v4();
        let mut rel = Relation::new("/ws", src, dst, RelationType::DEPENDS_ON, 0.9);
        rel.valid_at = now - chrono::Duration::days(10);

        assert!(rel.is_valid_at(now - chrono::Duration::days(5)));
        assert!(rel.is_valid_at(now));

        // Invalidate 2 days ago
        rel.invalidate(Some(now - chrono::Duration::days(2)));
        assert!(!rel.is_active());
        assert!(rel.is_valid_at(now - chrono::Duration::days(5))); // historical query was valid
        assert!(!rel.is_valid_at(now)); // current query is invalid
    }
}
