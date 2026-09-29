#![deny(unsafe_code)]
//! Knowledge graph extraction for Orbit.
//!
//! Extracts deterministic entities (File, Symbol, Decision, Technology, Concept)
//! and bi-temporal relations (DEFINES, IMPORTS, DEPENDS_ON, SUPERSEDES)
//! from source code chunks and Architectural Decision Records (ADRs).

use std::collections::HashSet;
use buzz_core::memory::{Chunk, Entity, EntityType, Relation, RelationType};
use uuid::Uuid;

/// Extracted knowledge graph elements from ingested chunks.
#[derive(Debug, Clone, Default)]
pub struct ExtractedGraph {
    /// Extracted entity nodes.
    pub entities: Vec<Entity>,
    /// Extracted relationship edges with provenance.
    pub relations: Vec<Relation>,
}

/// Knowledge graph extractor converting AST chunks and documentation into entities and relations.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeGraphExtractor;

impl KnowledgeGraphExtractor {
    /// Creates a new KnowledgeGraphExtractor.
    pub fn new() -> Self {
        Self
    }

    /// Extracts entities and relations from a set of ingested chunks belonging to a file.
    pub fn extract_from_file_chunks(
        &self,
        workspace_path: &str,
        file_path: &str,
        chunks: &[Chunk],
    ) -> ExtractedGraph {
        let mut extracted = ExtractedGraph::default();
        if chunks.is_empty() {
            return extracted;
        }

        // 1. Create deterministic File entity
        let file_entity = Entity::new_deterministic(
            workspace_path,
            file_path,
            EntityType::FILE,
            format!("Source file {file_path}"),
        );
        let file_id = file_entity.id;
        extracted.entities.push(file_entity);

        // Track symbols and imports to avoid duplicates within same file
        let mut seen_symbols = HashSet::new();
        let mut seen_imports = HashSet::new();

        let is_adr = file_path.to_ascii_lowercase().contains("adr")
            || file_path.to_ascii_lowercase().contains("decisions");

        for chunk in chunks {
            if is_adr {
                self.extract_adr_content(workspace_path, file_id, chunk, &mut extracted);
            } else {
                self.extract_code_symbols(
                    workspace_path,
                    file_id,
                    chunk,
                    &mut seen_symbols,
                    &mut extracted,
                );
                self.extract_code_imports(
                    workspace_path,
                    file_id,
                    chunk,
                    &mut seen_imports,
                    &mut extracted,
                );
            }
        }

        extracted
    }

    /// Extracts code symbols (functions, structs, classes, traits, interfaces) and DEFINES relations.
    fn extract_code_symbols(
        &self,
        workspace_path: &str,
        file_id: Uuid,
        chunk: &Chunk,
        seen: &mut HashSet<String>,
        extracted: &mut ExtractedGraph,
    ) {
        for line in chunk.content.lines() {
            let trimmed = line.trim();

            let symbol_opt = if let Some(rest) = trimmed.strip_prefix("pub fn ")
                .or_else(|| trimmed.strip_prefix("fn "))
                .or_else(|| trimmed.strip_prefix("def "))
                .or_else(|| trimmed.strip_prefix("function "))
                .or_else(|| trimmed.strip_prefix("async fn "))
            {
                let name = rest
                    .split(|c: char| c == '(' || c == '<' || c == ' ' || c == ':')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    Some((name.to_string(), "Function"))
                } else {
                    None
                }
            } else if let Some(rest) = trimmed.strip_prefix("pub struct ")
                .or_else(|| trimmed.strip_prefix("struct "))
                .or_else(|| trimmed.strip_prefix("class "))
                .or_else(|| trimmed.strip_prefix("pub enum "))
                .or_else(|| trimmed.strip_prefix("enum "))
                .or_else(|| trimmed.strip_prefix("pub trait "))
                .or_else(|| trimmed.strip_prefix("trait "))
                .or_else(|| trimmed.strip_prefix("interface "))
                .or_else(|| trimmed.strip_prefix("type "))
            {
                let name = rest
                    .split(|c: char| c == '{' || c == '<' || c == ' ' || c == ';' || c == '(' || c == ':')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    Some((name.to_string(), "Type"))
                } else {
                    None
                }
            } else {
                None
            };

            if let Some((sym_name, kind)) = symbol_opt {
                if seen.insert(sym_name.clone()) {
                    let mut sym_entity = Entity::new_deterministic(
                        workspace_path,
                        &sym_name,
                        EntityType::SYMBOL,
                        format!("{kind} symbol `{sym_name}`"),
                    );
                    sym_entity.metadata = serde_json::json!({
                        "kind": kind,
                        "line": chunk.line_start,
                    });
                    let sym_id = sym_entity.id;
                    extracted.entities.push(sym_entity);

                    // File -> DEFINES -> Symbol
                    let mut rel = Relation::new(
                        workspace_path,
                        file_id,
                        sym_id,
                        RelationType::DEFINES,
                        1.0,
                    );
                    rel.provenance_chunk_id = Some(chunk.id);
                    extracted.relations.push(rel);
                }
            }
        }
    }

    /// Extracts dependencies and imports (IMPORTS relations).
    fn extract_code_imports(
        &self,
        workspace_path: &str,
        file_id: Uuid,
        chunk: &Chunk,
        seen: &mut HashSet<String>,
        extracted: &mut ExtractedGraph,
    ) {
        for line in chunk.content.lines() {
            let trimmed = line.trim();

            let target_opt = if let Some(rest) = trimmed.strip_prefix("use ") {
                let pkg = rest.trim_end_matches(';').split("::").next().unwrap_or("").trim();
                if !pkg.is_empty() && pkg != "crate" && pkg != "super" && pkg != "self" {
                    Some(pkg.to_string())
                } else {
                    None
                }
            } else if let Some(rest) = trimmed.strip_prefix("import ") {
                let pkg = if let Some((_, from_part)) = rest.split_once("from ") {
                    from_part.trim().trim_matches(|c| c == '\'' || c == '"' || c == ';')
                } else {
                    rest.split_whitespace().next().unwrap_or("").trim_matches(|c| c == '\'' || c == '"' || c == ';')
                };
                let root = pkg.split('/').next().unwrap_or("").trim();
                if !root.is_empty() && !root.starts_with('.') {
                    Some(root.to_string())
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(target_pkg) = target_opt {
                if seen.insert(target_pkg.clone()) {
                    let mut tech_entity = Entity::new_deterministic(
                        workspace_path,
                        &target_pkg,
                        EntityType::TECHNOLOGY,
                        format!("Imported dependency `{target_pkg}`"),
                    );
                    tech_entity.metadata = serde_json::json!({
                        "category": "dependency"
                    });
                    let tech_id = tech_entity.id;
                    extracted.entities.push(tech_entity);

                    // File -> IMPORTS -> Technology
                    let mut rel = Relation::new(
                        workspace_path,
                        file_id,
                        tech_id,
                        RelationType::IMPORTS,
                        0.9,
                    );
                    rel.provenance_chunk_id = Some(chunk.id);
                    extracted.relations.push(rel);
                }
            }
        }
    }

    /// Extracts Architecture Decision Records (ADRs) and SUPERSEDES relations.
    fn extract_adr_content(
        &self,
        workspace_path: &str,
        file_id: Uuid,
        chunk: &Chunk,
        extracted: &mut ExtractedGraph,
    ) {
        let content = &chunk.content;
        let mut title = String::new();
        let mut supersedes_target = None;
        let mut status = "Proposed".to_string();

        for line in content.lines() {
            let trimmed = line.trim();
            if title.is_empty() && (trimmed.starts_with("# ") || trimmed.starts_with("## ")) {
                title = trimmed.trim_start_matches('#').trim().to_string();
            } else if trimmed.to_ascii_lowercase().starts_with("status:") {
                status = trimmed[7..].trim().to_string();
            } else if trimmed.to_ascii_lowercase().starts_with("supersedes:") {
                supersedes_target = Some(trimmed[11..].trim().to_string());
            }
        }

        if title.is_empty() {
            title = format!("Decision in chunk {}", chunk.chunk_index);
        }

        let mut decision_entity = Entity::new_deterministic(
            workspace_path,
            &title,
            EntityType::DECISION,
            format!("Architecture Decision: {title}"),
        );
        decision_entity.metadata = serde_json::json!({
            "status": status,
            "supersedes": supersedes_target,
        });
        let decision_id = decision_entity.id;
        extracted.entities.push(decision_entity);

        // File -> CONTAINS -> Decision
        let mut rel_contains = Relation::new(
            workspace_path,
            file_id,
            decision_id,
            RelationType::CONTAINS,
            1.0,
        );
        rel_contains.provenance_chunk_id = Some(chunk.id);
        extracted.relations.push(rel_contains);

        // If it supersedes another decision, create SUPERSEDES relation
        if let Some(target_title) = supersedes_target {
            let target_old_decision = Entity::new_deterministic(
                workspace_path,
                &target_title,
                EntityType::DECISION,
                format!("Architecture Decision: {target_title}"),
            );
            let target_id = target_old_decision.id;
            extracted.entities.push(target_old_decision);

            let mut rel_supersedes = Relation::new(
                workspace_path,
                decision_id,
                target_id,
                RelationType::SUPERSEDES,
                1.0,
            );
            rel_supersedes.provenance_chunk_id = Some(chunk.id);
            extracted.relations.push(rel_supersedes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_extract_symbols_and_imports() {
        let extractor = KnowledgeGraphExtractor::new();
        let ws = "/repo";
        let doc_id = Uuid::new_v4();

        let chunk = Chunk {
            id: Uuid::new_v4(),
            document_id: doc_id,
            workspace_path: ws.to_string(),
            chunk_index: 0,
            content: r#"
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};

pub struct MemoryEngine {
    state: RwLock<()>,
}

pub fn initialize_brain() -> bool {
    true
}
"#.to_string(),
            token_count: 50,
            scope: "project".to_string(),
            agent_name: None,
            line_start: Some(1),
            line_end: Some(12),
            created_at: Utc::now(),
        };

        let extracted = extractor.extract_from_file_chunks(ws, "src/brain.rs", &[chunk.clone()]);
        assert!(!extracted.entities.is_empty());
        assert!(!extracted.relations.is_empty());

        // File entity exists
        assert!(extracted.entities.iter().any(|e| e.name == "src/brain.rs" && e.entity_type == EntityType::FILE));

        // Symbols exist
        assert!(extracted.entities.iter().any(|e| e.name == "MemoryEngine" && e.entity_type == EntityType::SYMBOL));
        assert!(extracted.entities.iter().any(|e| e.name == "initialize_brain" && e.entity_type == EntityType::SYMBOL));

        // Technologies exist
        assert!(extracted.entities.iter().any(|e| e.name == "tokio" && e.entity_type == EntityType::TECHNOLOGY));
        assert!(extracted.entities.iter().any(|e| e.name == "serde" && e.entity_type == EntityType::TECHNOLOGY));

        // Relations exist with provenance
        assert!(extracted.relations.iter().any(|r| r.relation_type == RelationType::DEFINES && r.provenance_chunk_id == Some(chunk.id)));
        assert!(extracted.relations.iter().any(|r| r.relation_type == RelationType::IMPORTS && r.provenance_chunk_id == Some(chunk.id)));
    }

    #[test]
    fn test_extract_adr_supersedes() {
        let extractor = KnowledgeGraphExtractor::new();
        let ws = "/repo";
        let doc_id = Uuid::new_v4();

        let chunk = Chunk {
            id: Uuid::new_v4(),
            document_id: doc_id,
            workspace_path: ws.to_string(),
            chunk_index: 0,
            content: r#"# ADR-002: Use Embedded SQLite
Status: Accepted
Supersedes: ADR-001: Postgres Central Relay

We replace centralized Postgres with embedded SQLite and LanceDB for desktop brain.
"#.to_string(),
            token_count: 40,
            scope: "project".to_string(),
            agent_name: None,
            line_start: Some(1),
            line_end: Some(6),
            created_at: Utc::now(),
        };

        let extracted = extractor.extract_from_file_chunks(ws, "docs/adr/002-sqlite.md", &[chunk.clone()]);
        assert!(extracted.entities.iter().any(|e| e.name == "ADR-002: Use Embedded SQLite" && e.entity_type == EntityType::DECISION));
        assert!(extracted.entities.iter().any(|e| e.name == "ADR-001: Postgres Central Relay" && e.entity_type == EntityType::DECISION));

        // Check SUPERSEDES relation
        let supersedes = extracted.relations.iter().find(|r| r.relation_type == RelationType::SUPERSEDES).expect("supersedes relation exists");
        assert_eq!(supersedes.provenance_chunk_id, Some(chunk.id));
    }
}
