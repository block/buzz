//! Deterministic shared-document manifests for durable agent workflows.
//!
//! Format adapters provide original bytes plus extracted pages. This module
//! owns immutable hashes, stable coordinates, integrity checks, and bounded
//! retrieval shared by every worker in a run.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Current serialized manifest format version.
pub const DOCUMENT_MANIFEST_VERSION: u32 = 1;
/// Default maximum source size (256 MiB).
pub const DEFAULT_MAX_DOCUMENT_BYTES: usize = 256 * 1024 * 1024;
/// Default maximum extracted pages.
pub const DEFAULT_MAX_PAGES: usize = 10_000;
/// Default target maximum UTF-8 bytes per chunk.
pub const DEFAULT_CHUNK_BYTES: usize = 8 * 1024;
/// Maximum chunks returned by one retrieval call.
pub const MAX_RETRIEVAL_CHUNKS: usize = 32;
/// Maximum aggregate bytes returned by one retrieval call.
pub const MAX_RETRIEVAL_BYTES: usize = 256 * 1024;

/// Limits applied while building a document manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestLimits {
    /// Maximum original source bytes.
    pub max_document_bytes: usize,
    /// Maximum extracted pages.
    pub max_pages: usize,
    /// Maximum UTF-8 bytes in one extracted page.
    pub max_page_text_bytes: usize,
    /// Target maximum UTF-8 bytes per chunk.
    pub chunk_bytes: usize,
    /// Maximum chunks in the manifest.
    pub max_chunks: usize,
}

impl Default for IngestLimits {
    fn default() -> Self {
        Self {
            max_document_bytes: DEFAULT_MAX_DOCUMENT_BYTES,
            max_pages: DEFAULT_MAX_PAGES,
            max_page_text_bytes: 4 * 1024 * 1024,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            max_chunks: 100_000,
        }
    }
}

/// One page extracted by a format-specific adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPage {
    /// One-based physical page number in the source.
    pub physical_page: u32,
    /// Optional logical label printed by the source, such as fls. 184.
    pub logical_label: Option<String>,
    /// Extracted UTF-8 text, preserved exactly.
    pub text: String,
}

/// Input used to build an immutable document manifest.
#[derive(Debug)]
pub struct DocumentInput<'a> {
    /// Stable source name.
    pub source_name: &'a str,
    /// Source media type.
    pub content_type: &'a str,
    /// Original bytes used for the document SHA-256.
    pub source_bytes: &'a [u8],
    /// Extracted pages in physical source order.
    pub pages: &'a [ExtractedPage],
}

/// One immutable, source-addressable text chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentChunk {
    /// Stable zero-based sequence within the document.
    pub sequence: u32,
    /// Stable chunk identifier with a sha256 prefix.
    pub id: String,
    /// One-based physical source page.
    pub physical_page: u32,
    /// Optional logical page label.
    pub logical_label: Option<String>,
    /// Zero-based chunk sequence within the page.
    pub page_chunk: u32,
    /// Inclusive UTF-8 byte offset within the page.
    pub byte_start: u32,
    /// Exclusive UTF-8 byte offset within the page.
    pub byte_end: u32,
    /// Exact extracted text.
    pub text: String,
    /// SHA-256 of canonical coordinate and text.
    pub sha256: String,
}

/// Immutable shared-document manifest consumed by all workers in one run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentManifest {
    /// Manifest format version.
    pub version: u32,
    /// Stable source name.
    pub source_name: String,
    /// Source media type.
    pub content_type: String,
    /// Original source byte length.
    pub byte_len: u64,
    /// SHA-256 of original source bytes.
    pub document_sha256: String,
    /// Number of extracted pages.
    pub page_count: u32,
    /// Ordered immutable chunks.
    pub chunks: Vec<DocumentChunk>,
    /// SHA-256 of canonical manifest fields.
    pub manifest_sha256: String,
}

/// Selectors and bounds for deterministic retrieval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetrievalQuery {
    /// Stable chunk ids to include; empty means any.
    pub chunk_ids: Vec<String>,
    /// Physical pages to include; empty means any.
    pub physical_pages: Vec<u32>,
    /// Logical labels to include; empty means any.
    pub logical_labels: Vec<String>,
    /// Case-insensitive terms that must all occur.
    pub terms: Vec<String>,
    /// Requested chunk limit, clamped to the global ceiling.
    pub limit: usize,
    /// Requested aggregate byte limit, clamped to the global ceiling.
    pub max_bytes: usize,
}

impl Default for RetrievalQuery {
    fn default() -> Self {
        Self {
            chunk_ids: Vec::new(),
            physical_pages: Vec::new(),
            logical_labels: Vec::new(),
            terms: Vec::new(),
            limit: 10,
            max_bytes: 64 * 1024,
        }
    }
}

/// Failure while building or verifying a document manifest.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DocumentError {
    /// A configured limit is zero.
    #[error("invalid ingestion limit: {0}")]
    InvalidLimit(&'static str),
    /// Source exceeds the byte ceiling.
    #[error("document has {actual} bytes; limit is {limit}")]
    DocumentTooLarge {
        /// Actual source byte length.
        actual: usize,
        /// Configured byte ceiling.
        limit: usize,
    },
    /// Extracted pages exceed the ceiling.
    #[error("document has {actual} pages; limit is {limit}")]
    TooManyPages {
        /// Actual extracted page count.
        actual: usize,
        /// Configured page ceiling.
        limit: usize,
    },
    /// Physical pages are invalid or unordered.
    #[error("physical pages must be unique, non-zero, and strictly increasing")]
    InvalidPageOrder,
    /// One extracted page exceeds its ceiling.
    #[error("page {page} has {actual} text bytes; limit is {limit}")]
    PageTextTooLarge {
        /// Physical page.
        page: u32,
        /// Actual bytes.
        actual: usize,
        /// Configured limit.
        limit: usize,
    },
    /// Chunk count exceeds its ceiling.
    #[error("manifest has more than {limit} chunks")]
    TooManyChunks {
        /// Configured chunk ceiling.
        limit: usize,
    },
    /// A coordinate cannot be represented.
    #[error("document coordinate exceeds manifest format")]
    CoordinateOverflow,
    /// Canonical serialization failed.
    #[error("manifest serialization failed: {0}")]
    Serialization(String),
    /// Source bytes do not match the manifest.
    #[error("source document hash mismatch")]
    SourceHashMismatch,
    /// One chunk hash or coordinate is invalid.
    #[error("chunk integrity check failed at sequence {sequence}")]
    ChunkIntegrity {
        /// Sequence of the first invalid chunk.
        sequence: u32,
    },
    /// Canonical manifest fields do not match the manifest hash.
    #[error("manifest hash mismatch")]
    ManifestHashMismatch,
}

#[derive(Serialize)]
struct ChunkDigest<'a> {
    sequence: u32,
    physical_page: u32,
    logical_label: &'a Option<String>,
    page_chunk: u32,
    byte_start: u32,
    byte_end: u32,
    text: &'a str,
}

#[derive(Serialize)]
struct ManifestDigest<'a> {
    version: u32,
    source_name: &'a str,
    content_type: &'a str,
    byte_len: u64,
    document_sha256: &'a str,
    page_count: u32,
    chunks: &'a [DocumentChunk],
}

/// Build a deterministic manifest from original bytes and extracted pages.
pub fn build_document_manifest(
    input: DocumentInput<'_>,
    limits: IngestLimits,
) -> Result<DocumentManifest, DocumentError> {
    validate_limits(limits)?;
    if input.source_bytes.len() > limits.max_document_bytes {
        return Err(DocumentError::DocumentTooLarge {
            actual: input.source_bytes.len(),
            limit: limits.max_document_bytes,
        });
    }
    if input.pages.len() > limits.max_pages {
        return Err(DocumentError::TooManyPages {
            actual: input.pages.len(),
            limit: limits.max_pages,
        });
    }
    validate_page_order(input.pages)?;

    let mut chunks = Vec::new();
    for page in input.pages {
        if page.text.len() > limits.max_page_text_bytes {
            return Err(DocumentError::PageTextTooLarge {
                page: page.physical_page,
                actual: page.text.len(),
                limit: limits.max_page_text_bytes,
            });
        }
        for (page_chunk, (start, end)) in utf8_ranges(&page.text, limits.chunk_bytes).enumerate() {
            if chunks.len() >= limits.max_chunks {
                return Err(DocumentError::TooManyChunks {
                    limit: limits.max_chunks,
                });
            }
            let sequence = to_u32(chunks.len())?;
            let page_chunk = to_u32(page_chunk)?;
            let byte_start = to_u32(start)?;
            let byte_end = to_u32(end)?;
            let text = page.text[start..end].to_owned();
            let digest = ChunkDigest {
                sequence,
                physical_page: page.physical_page,
                logical_label: &page.logical_label,
                page_chunk,
                byte_start,
                byte_end,
                text: &text,
            };
            let sha256 = canonical_sha256(&digest)?;
            chunks.push(DocumentChunk {
                sequence,
                id: format!("sha256:{sha256}"),
                physical_page: page.physical_page,
                logical_label: page.logical_label.clone(),
                page_chunk,
                byte_start,
                byte_end,
                text,
                sha256,
            });
        }
    }

    let mut manifest = DocumentManifest {
        version: DOCUMENT_MANIFEST_VERSION,
        source_name: input.source_name.to_owned(),
        content_type: input.content_type.to_owned(),
        byte_len: u64::try_from(input.source_bytes.len())
            .map_err(|_| DocumentError::CoordinateOverflow)?,
        document_sha256: sha256_hex(input.source_bytes),
        page_count: to_u32(input.pages.len())?,
        chunks,
        manifest_sha256: String::new(),
    };
    manifest.manifest_sha256 = calculate_manifest_hash(&manifest)?;
    Ok(manifest)
}

/// Verify source bytes and every immutable coordinate and hash.
pub fn verify_document_manifest(
    manifest: &DocumentManifest,
    source_bytes: &[u8],
) -> Result<(), DocumentError> {
    let source_len =
        u64::try_from(source_bytes.len()).map_err(|_| DocumentError::CoordinateOverflow)?;
    if manifest.version != DOCUMENT_MANIFEST_VERSION
        || manifest.byte_len != source_len
        || manifest.document_sha256 != sha256_hex(source_bytes)
    {
        return Err(DocumentError::SourceHashMismatch);
    }
    let distinct_pages = manifest
        .chunks
        .iter()
        .map(|chunk| chunk.physical_page)
        .collect::<std::collections::BTreeSet<_>>();
    if to_u32(distinct_pages.len())? > manifest.page_count {
        return Err(DocumentError::ManifestHashMismatch);
    }
    let mut previous_page = 0u32;
    let mut previous_page_chunk = 0u32;
    let mut previous_byte_end = 0u32;
    for (index, chunk) in manifest.chunks.iter().enumerate() {
        let expected_sequence = to_u32(index)?;
        let new_page = chunk.physical_page != previous_page;
        let expected_page_chunk = if new_page {
            Some(0)
        } else {
            previous_page_chunk.checked_add(1)
        };
        let expected_byte_start = if new_page { 0 } else { previous_byte_end };
        let digest = ChunkDigest {
            sequence: chunk.sequence,
            physical_page: chunk.physical_page,
            logical_label: &chunk.logical_label,
            page_chunk: chunk.page_chunk,
            byte_start: chunk.byte_start,
            byte_end: chunk.byte_end,
            text: &chunk.text,
        };
        let expected_hash = canonical_sha256(&digest)?;
        if chunk.sequence != expected_sequence
            || chunk.physical_page == 0
            || chunk.physical_page < previous_page
            || expected_page_chunk != Some(chunk.page_chunk)
            || chunk.byte_start != expected_byte_start
            || chunk.byte_start >= chunk.byte_end
            || chunk.sha256 != expected_hash
            || chunk.id != format!("sha256:{expected_hash}")
        {
            return Err(DocumentError::ChunkIntegrity {
                sequence: chunk.sequence,
            });
        }
        previous_page = chunk.physical_page;
        previous_page_chunk = chunk.page_chunk;
        previous_byte_end = chunk.byte_end;
    }
    if calculate_manifest_hash(manifest)? != manifest.manifest_sha256 {
        return Err(DocumentError::ManifestHashMismatch);
    }
    Ok(())
}

/// Retrieve chunks in source order under strict output bounds.
pub fn retrieve_document_chunks<'a>(
    manifest: &'a DocumentManifest,
    query: &RetrievalQuery,
) -> Vec<&'a DocumentChunk> {
    let limit = query.limit.clamp(1, MAX_RETRIEVAL_CHUNKS);
    let max_bytes = query.max_bytes.clamp(1, MAX_RETRIEVAL_BYTES);
    let terms: Vec<String> = query
        .terms
        .iter()
        .map(|term| term.to_lowercase())
        .filter(|term| !term.is_empty())
        .collect();
    let mut selected = Vec::new();
    let mut bytes = 0usize;
    for chunk in &manifest.chunks {
        if !query.chunk_ids.is_empty() && !query.chunk_ids.contains(&chunk.id) {
            continue;
        }
        if !query.physical_pages.is_empty() && !query.physical_pages.contains(&chunk.physical_page)
        {
            continue;
        }
        if !query.logical_labels.is_empty()
            && !chunk
                .logical_label
                .as_ref()
                .is_some_and(|label| query.logical_labels.contains(label))
        {
            continue;
        }
        if !terms.is_empty() {
            let haystack = chunk.text.to_lowercase();
            if !terms.iter().all(|term| haystack.contains(term)) {
                continue;
            }
        }
        if selected.len() >= limit || bytes.saturating_add(chunk.text.len()) > max_bytes {
            break;
        }
        bytes += chunk.text.len();
        selected.push(chunk);
    }
    selected
}

fn validate_limits(limits: IngestLimits) -> Result<(), DocumentError> {
    for (value, name) in [
        (limits.max_document_bytes, "max_document_bytes"),
        (limits.max_pages, "max_pages"),
        (limits.max_page_text_bytes, "max_page_text_bytes"),
        (limits.chunk_bytes, "chunk_bytes"),
        (limits.max_chunks, "max_chunks"),
    ] {
        if value == 0 {
            return Err(DocumentError::InvalidLimit(name));
        }
    }
    Ok(())
}

fn validate_page_order(pages: &[ExtractedPage]) -> Result<(), DocumentError> {
    let mut previous = 0;
    for page in pages {
        if page.physical_page == 0 || page.physical_page <= previous {
            return Err(DocumentError::InvalidPageOrder);
        }
        previous = page.physical_page;
    }
    Ok(())
}

fn utf8_ranges(text: &str, maximum: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut start = 0;
    std::iter::from_fn(move || {
        if start >= text.len() {
            return None;
        }
        let mut end = start.saturating_add(maximum).min(text.len());
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start {
            end = text[start..]
                .char_indices()
                .nth(1)
                .map_or(text.len(), |(offset, _)| start + offset);
        }
        let result = (start, end);
        start = end;
        Some(result)
    })
}

fn calculate_manifest_hash(manifest: &DocumentManifest) -> Result<String, DocumentError> {
    canonical_sha256(&ManifestDigest {
        version: manifest.version,
        source_name: &manifest.source_name,
        content_type: &manifest.content_type,
        byte_len: manifest.byte_len,
        document_sha256: &manifest.document_sha256,
        page_count: manifest.page_count,
        chunks: &manifest.chunks,
    })
}

fn canonical_sha256(value: &impl Serialize) -> Result<String, DocumentError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| DocumentError::Serialization(error.to_string()))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn to_u32(value: usize) -> Result<u32, DocumentError> {
    u32::try_from(value).map_err(|_| DocumentError::CoordinateOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages() -> Vec<ExtractedPage> {
        vec![
            ExtractedPage {
                physical_page: 1,
                logical_label: Some("fls. 184".into()),
                text: "Defesa α confirma pagina fisica um.".into(),
            },
            ExtractedPage {
                physical_page: 2,
                logical_label: Some("fls. 185".into()),
                text: "Contraditorio independente na pagina dois.".into(),
            },
        ]
    }

    fn manifest(chunk_bytes: usize) -> DocumentManifest {
        let pages = pages();
        build_document_manifest(
            DocumentInput {
                source_name: "synthetic.pdf",
                content_type: "application/pdf",
                source_bytes: b"%PDF synthetic immutable bytes",
                pages: &pages,
            },
            IngestLimits {
                chunk_bytes,
                ..IngestLimits::default()
            },
        )
        .expect("manifest should build")
    }

    #[test]
    fn deterministic_and_preserves_page_coordinates() {
        let first = manifest(16);
        assert_eq!(first, manifest(16));
        assert_eq!(first.page_count, 2);
        assert_eq!(first.chunks[0].physical_page, 1);
        assert_eq!(first.chunks[0].logical_label.as_deref(), Some("fls. 184"));
        verify_document_manifest(&first, b"%PDF synthetic immutable bytes")
            .expect("valid manifest");
    }

    #[test]
    fn rejects_source_and_chunk_tampering() {
        let original = manifest(32);
        assert_eq!(
            verify_document_manifest(&original, b"different"),
            Err(DocumentError::SourceHashMismatch)
        );
        let mut tampered = original;
        tampered.chunks[0].text.push_str(" injected");
        assert!(matches!(
            verify_document_manifest(&tampered, b"%PDF synthetic immutable bytes"),
            Err(DocumentError::ChunkIntegrity { sequence: 0 })
        ));
    }

    #[test]
    fn retrieval_is_filtered_and_bounded() {
        let manifest = manifest(16);
        let selected = retrieve_document_chunks(
            &manifest,
            &RetrievalQuery {
                logical_labels: vec!["fls. 185".into()],
                limit: usize::MAX,
                max_bytes: usize::MAX,
                ..RetrievalQuery::default()
            },
        );
        assert!(!selected.is_empty());
        assert!(selected
            .iter()
            .all(|chunk| chunk.logical_label.as_deref() == Some("fls. 185")));
        let bounded = retrieve_document_chunks(
            &manifest,
            &RetrievalQuery {
                limit: usize::MAX,
                max_bytes: 16,
                ..RetrievalQuery::default()
            },
        );
        assert!(bounded.len() <= MAX_RETRIEVAL_CHUNKS);
        assert!(bounded.iter().map(|chunk| chunk.text.len()).sum::<usize>() <= 16);
    }

    #[test]
    fn rejects_invalid_order_and_limits() {
        let mut bad_pages = pages();
        bad_pages[1].physical_page = 1;
        let error = build_document_manifest(
            DocumentInput {
                source_name: "bad.pdf",
                content_type: "application/pdf",
                source_bytes: b"bytes",
                pages: &bad_pages,
            },
            IngestLimits::default(),
        )
        .expect_err("duplicate physical page must fail");
        assert_eq!(error, DocumentError::InvalidPageOrder);
        let error = build_document_manifest(
            DocumentInput {
                source_name: "large.pdf",
                content_type: "application/pdf",
                source_bytes: b"too large",
                pages: &[],
            },
            IngestLimits {
                max_document_bytes: 1,
                ..IngestLimits::default()
            },
        )
        .expect_err("source limit must fail");
        assert!(matches!(error, DocumentError::DocumentTooLarge { .. }));
    }
}
