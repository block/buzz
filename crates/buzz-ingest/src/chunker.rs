#![deny(unsafe_code)]
//! AST-aware semantic chunker for Orbit memory ingestion.
//!
//! Preserves syntactic function, struct, class, and heading boundaries across:
//! - Rust (`.rs`)
//! - TypeScript / JavaScript (`.ts`, `.tsx`, `.js`, `.jsx`)
//! - Python (`.py`)
//! - Go (`.go`)
//! - Markdown / Documentation (`.md`, `.markdown`)
//! - Generic text fallback with 50-token sliding overlap

use std::path::Path;
use buzz_core::memory::Chunk;
use uuid::Uuid;

/// Detected programming language or file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    /// Rust
    Rust,
    /// TypeScript / JavaScript
    TypeScript,
    /// Python
    Python,
    /// Go
    Go,
    /// Markdown documentation
    Markdown,
    /// Plaintext or unsupported extension
    Generic,
}

impl SupportedLanguage {
    /// Detects language from file extension.
    pub fn from_path(path: impl AsRef<Path>) -> Self {
        let ext = path
            .as_ref()
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ext.as_str() {
            "rs" => Self::Rust,
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Self::TypeScript,
            "py" | "pyi" => Self::Python,
            "go" => Self::Go,
            "md" | "markdown" | "mdx" => Self::Markdown,
            _ => Self::Generic,
        }
    }
}

/// AST-aware semantic chunker.
#[derive(Debug, Clone)]
pub struct AstChunker {
    target_chunk_tokens: usize,
    overlap_tokens: usize,
}

impl Default for AstChunker {
    fn default() -> Self {
        Self {
            target_chunk_tokens: 512,
            overlap_tokens: 50,
        }
    }
}

impl AstChunker {
    /// Creates a new chunker with custom target token count and overlap.
    pub fn new(target_chunk_tokens: usize, overlap_tokens: usize) -> Self {
        Self {
            target_chunk_tokens: target_chunk_tokens.max(64),
            overlap_tokens: overlap_tokens.min(target_chunk_tokens / 2),
        }
    }

    /// Estimates token count (~4 characters per token heuristic).
    pub fn estimate_tokens(text: &str) -> usize {
        (text.len() / 4).max(1)
    }

    /// Chunks a source file into semantic AST chunks with line provenance.
    pub fn chunk(
        &self,
        document_id: Uuid,
        workspace_path: &str,
        file_path: impl AsRef<Path>,
        content: &str,
        scope: &str,
    ) -> Vec<Chunk> {
        let lang = SupportedLanguage::from_path(file_path);
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Vec::new();
        }

        let raw_sections = match lang {
            SupportedLanguage::Rust => self.split_rust(&lines),
            SupportedLanguage::TypeScript => self.split_typescript(&lines),
            SupportedLanguage::Python => self.split_python(&lines),
            SupportedLanguage::Go => self.split_go(&lines),
            SupportedLanguage::Markdown => self.split_markdown(&lines),
            SupportedLanguage::Generic => self.split_generic(&lines),
        };

        let mut chunks = Vec::with_capacity(raw_sections.len());
        for (idx, (line_start, line_end, text)) in raw_sections.into_iter().enumerate() {
            let tokens = Self::estimate_tokens(&text) as i32;
            let mut chunk = Chunk::new(document_id, workspace_path, idx as i32, text, scope);
            chunk.token_count = tokens;
            chunk.line_start = Some(line_start as i32);
            chunk.line_end = Some(line_end as i32);
            chunks.push(chunk);
        }

        chunks
    }

    // ─── Rust AST Boundary Detection ───────────────────────────────────────
    // ponytail: line-based regex/prefix AST boundary scan, full tree-sitter grammars in V2
    fn split_rust(&self, lines: &[&str]) -> Vec<(usize, usize, String)> {
        let is_boundary = |line: &str| -> bool {
            let trimmed = line.trim_start();
            let prefixes = [
                "pub fn ", "fn ", "pub async fn ", "async fn ",
                "pub struct ", "struct ", "pub enum ", "enum ",
                "pub trait ", "trait ", "impl ", "pub mod ", "mod ",
                "pub type ", "type ", "#[derive", "#[tokio::test]", "#[test]",
            ];
            prefixes.iter().any(|&p| trimmed.starts_with(p))
        };

        self.group_by_boundaries(lines, is_boundary)
    }

    // ─── TypeScript / JavaScript Boundary Detection ────────────────────────
    fn split_typescript(&self, lines: &[&str]) -> Vec<(usize, usize, String)> {
        let is_boundary = |line: &str| -> bool {
            let trimmed = line.trim_start();
            let prefixes = [
                "export function ", "function ", "export async function ", "async function ",
                "export class ", "class ", "export interface ", "interface ",
                "export type ", "type ", "export const ", "const ",
                "export default ", "describe(", "it(", "test(",
            ];
            prefixes.iter().any(|&p| trimmed.starts_with(p))
        };

        self.group_by_boundaries(lines, is_boundary)
    }

    // ─── Python Boundary Detection ─────────────────────────────────────────
    fn split_python(&self, lines: &[&str]) -> Vec<(usize, usize, String)> {
        let is_boundary = |line: &str| -> bool {
            // Top-level or class-level def/class (zero or 4 space indentation)
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            (indent == 0 || indent == 4)
                && (trimmed.starts_with("def ")
                    || trimmed.starts_with("async def ")
                    || trimmed.starts_with("class ")
                    || trimmed.starts_with("@"))
        };

        self.group_by_boundaries(lines, is_boundary)
    }

    // ─── Go Boundary Detection ─────────────────────────────────────────────
    fn split_go(&self, lines: &[&str]) -> Vec<(usize, usize, String)> {
        let is_boundary = |line: &str| -> bool {
            let trimmed = line.trim_start();
            trimmed.starts_with("func ")
                || trimmed.starts_with("type ")
                || trimmed.starts_with("const ")
                || trimmed.starts_with("var ")
        };

        self.group_by_boundaries(lines, is_boundary)
    }

    // ─── Markdown Heading Boundary Detection ───────────────────────────────
    fn split_markdown(&self, lines: &[&str]) -> Vec<(usize, usize, String)> {
        let is_boundary = |line: &str| -> bool {
            let trimmed = line.trim_start();
            trimmed.starts_with("# ")
                || trimmed.starts_with("## ")
                || trimmed.starts_with("### ")
                || trimmed.starts_with("#### ")
        };

        self.group_by_boundaries(lines, is_boundary)
    }

    // ─── Generic Line Window with Overlap ──────────────────────────────────
    fn split_generic(&self, lines: &[&str]) -> Vec<(usize, usize, String)> {
        let mut sections = Vec::new();
        let target_lines = (self.target_chunk_tokens * 4) / 40; // rough lines per chunk (~50 lines)
        let overlap_lines = (self.overlap_tokens * 4) / 40;

        let mut start = 0;
        while start < lines.len() {
            let end = (start + target_lines.max(10)).min(lines.len());
            let chunk_lines = &lines[start..end];
            let text = chunk_lines.join("\n");
            sections.push((start + 1, end, text));

            if end >= lines.len() {
                break;
            }
            start = end.saturating_sub(overlap_lines.max(2));
        }

        sections
    }

    // ─── Boundary Grouping Helper ──────────────────────────────────────────
    fn group_by_boundaries<F>(&self, lines: &[&str], is_boundary: F) -> Vec<(usize, usize, String)>
    where
        F: Fn(&str) -> bool,
    {
        let mut sections = Vec::new();
        let mut current_lines: Vec<&str> = Vec::new();
        let mut chunk_start_line = 1;

        for (i, &line) in lines.iter().enumerate() {
            let line_num = i + 1;
            let current_token_estimate = current_lines.iter().map(|l| l.len() / 4 + 1).sum::<usize>();

            let has_content = current_lines.iter().any(|l| !l.trim().is_empty());

            if is_boundary(line) && has_content {
                // Flush accumulated chunk
                let text = current_lines.join("\n").trim().to_string();
                if !text.is_empty() {
                    let end_line = line_num - 1;
                    sections.push((chunk_start_line, end_line, text));
                }

                current_lines.clear();
                chunk_start_line = line_num;
            }

            current_lines.push(line);

            // If chunk grows too large, hard split
            if current_token_estimate >= self.target_chunk_tokens {
                let text = current_lines.join("\n").trim().to_string();
                if !text.is_empty() {
                    sections.push((chunk_start_line, line_num, text));
                }
                current_lines.clear();
                chunk_start_line = line_num + 1;
            }
        }

        if !current_lines.is_empty() {
            let text = current_lines.join("\n").trim().to_string();
            if !text.is_empty() {
                sections.push((chunk_start_line, lines.len(), text));
            }
        }

        sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_ast_chunking() {
        let chunker = AstChunker::default();
        let code = r#"
pub struct User {
    pub name: String,
    pub age: u32,
}

impl User {
    pub fn new(name: &str, age: u32) -> Self {
        Self {
            name: name.to_string(),
            age,
        }
    }
}

pub fn calculate_hash(data: &[u8]) -> u64 {
    42
}
"#;
        let chunks = chunker.chunk(Uuid::new_v4(), "/test", "src/user.rs", code, "project");
        assert!(chunks.len() >= 2, "Expected at least 2 AST chunks, got {}", chunks.len());
        // Verify line numbering
        assert!(chunks[0].line_start.unwrap() >= 1);
        assert!(chunks[0].line_end.unwrap() >= chunks[0].line_start.unwrap());
    }

    #[test]
    fn test_markdown_heading_chunking() {
        let chunker = AstChunker::default();
        let doc = r#"# Orbit Architecture

Overview of the five-layer context model.

## Storage Foundation

Details about SQLite and LanceDB.

## Ingestion Pipeline

Details about file watchers and AST chunking.
"#;
        let chunks = chunker.chunk(Uuid::new_v4(), "/test", "README.md", doc, "project");
        assert!(chunks.len() >= 2, "Expected headings to split chunks");
    }

    #[test]
    fn test_python_ast_chunking() {
        let chunker = AstChunker::default();
        let py_code = r#"
class DatabaseEngine:
    def __init__(self, path):
        self.path = path

    def connect(self):
        return True

def standalone_worker(task_id):
    print("Running task", task_id)
"#;
        let chunks = chunker.chunk(Uuid::new_v4(), "/test", "db.py", py_code, "project");
        assert!(!chunks.is_empty());
    }
}
