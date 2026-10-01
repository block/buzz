#![deny(unsafe_code)]
//! Semantic chunker for agent conversation sessions.

use chrono::Utc;
use uuid::Uuid;
use buzz_core::memory::Chunk;

/// Chunker that divides normalized session transcripts into retrieval-friendly context chunks.
#[derive(Debug, Clone)]
pub struct SessionChunker {
    /// Target chunk token capacity (approx. 4 chars per token). Default 512 tokens (~2048 chars).
    pub target_token_size: usize,
    /// Overlap in tokens between consecutive chunks. Default 64 tokens (~256 chars).
    pub token_overlap: usize,
}

impl Default for SessionChunker {
    fn default() -> Self {
        Self {
            target_token_size: 512,
            token_overlap: 64,
        }
    }
}

impl SessionChunker {
    /// Creates a session chunker with custom token sizes.
    pub fn new(target_token_size: usize, token_overlap: usize) -> Self {
        Self {
            target_token_size,
            token_overlap,
        }
    }

    /// Splits a normalized session text into `Chunk` instances.
    pub fn chunk_session(
        &self,
        document_id: Uuid,
        workspace_path: &str,
        content: &str,
        agent_name: &str,
        scope: &str,
    ) -> Vec<Chunk> {
        if content.trim().is_empty() {
            return Vec::new();
        }

        let target_chars = self.target_token_size * 4;
        let overlap_chars = self.token_overlap * 4;

        let mut chunks = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        let mut current_chunk_lines: Vec<&str> = Vec::new();
        let mut current_char_len = 0;
        let mut chunk_start_line = 1;
        let mut chunk_index = 0;

        for (idx, line) in lines.iter().enumerate() {
            let line_num = idx + 1;
            let line_len = line.len() + 1; // +1 for newline

            // If adding this line exceeds the target size and we already have content,
            // or if we encounter a new major section ("### User") and already have enough content:
            if current_char_len + line_len > target_chars && !current_chunk_lines.is_empty() {
                let chunk_text = current_chunk_lines.join("\n");
                let token_count = (chunk_text.len() / 4).max(1) as i32;
                let chunk_end_line = line_num.saturating_sub(1) as i32;

                chunks.push(Chunk {
                    id: Uuid::new_v4(),
                    document_id,
                    workspace_path: workspace_path.to_string(),
                    chunk_index,
                    content: chunk_text,
                    token_count,
                    scope: scope.to_string(),
                    agent_name: Some(agent_name.to_string()),
                    line_start: Some(chunk_start_line as i32),
                    line_end: Some(chunk_end_line),
                    created_at: Utc::now(),
                });

                chunk_index += 1;

                // Carry over overlap lines
                let mut kept_lines = Vec::new();
                let mut kept_chars = 0;
                for l in current_chunk_lines.iter().rev() {
                    if kept_chars + l.len() <= overlap_chars {
                        kept_chars += l.len() + 1;
                        kept_lines.push(*l);
                    } else {
                        break;
                    }
                }
                kept_lines.reverse();
                chunk_start_line = line_num.saturating_sub(kept_lines.len());
                current_chunk_lines = kept_lines;
                current_char_len = kept_chars;
            }

            current_chunk_lines.push(line);
            current_char_len += line_len;
        }

        if !current_chunk_lines.is_empty() {
            let chunk_text = current_chunk_lines.join("\n");
            let token_count = (chunk_text.len() / 4).max(1) as i32;
            let chunk_end_line = lines.len() as i32;

            chunks.push(Chunk {
                id: Uuid::new_v4(),
                document_id,
                workspace_path: workspace_path.to_string(),
                chunk_index,
                content: chunk_text,
                token_count,
                scope: scope.to_string(),
                agent_name: Some(agent_name.to_string()),
                line_start: Some(chunk_start_line as i32),
                line_end: Some(chunk_end_line),
                created_at: Utc::now(),
            });
        }

        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_session() {
        let chunker = SessionChunker::new(50, 10);
        let doc_id = Uuid::new_v4();
        let content = "### User\nWhat is Orbit?\n\n### Assistant\nOrbit is an AI memory and context engine that runs locally with SQLite and LanceDB.\nIt supports 9 IDE transcript parsers.\n".repeat(10);

        let chunks = chunker.chunk_session(doc_id, "/workspace", &content, "antigravity", "session");
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].agent_name.as_deref(), Some("antigravity"));
        assert_eq!(chunks[0].document_id, doc_id);
    }
}
