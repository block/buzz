//! Flow Studio read-model tables (Buzz Hive projector target).

use buzz_core::tenant::CommunityId;
use sqlx::{PgPool, Row};

use crate::error::Result;

/// A knowledge-base search hit (keyword match on chunk content).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowKnowledgeSearchHit {
    /// Document identifier.
    pub document_id: String,
    /// Chunk index within the document.
    pub chunk_index: i32,
    /// Matching chunk text.
    pub content: String,
}

/// A table row in the Flow Studio read-model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowTableRowRecord {
    /// Row identifier within the table.
    pub row_id: String,
    /// Row payload as JSON.
    pub row_json: serde_json::Value,
}

/// File metadata in the Flow Studio read-model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowFileRecord {
    /// File identifier.
    pub file_id: String,
    /// Original filename.
    pub filename: String,
    /// Blossom media URL when uploaded.
    pub media_url: Option<String>,
    /// Monotonic version counter.
    pub version: i32,
}

/// Upsert a knowledge document row from a Flow Studio ingest event.
pub async fn upsert_knowledge_document(
    pool: &PgPool,
    community_id: CommunityId,
    knowledge_base_id: &str,
    document_id: &str,
    filename: &str,
    mime_type: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO flow_knowledge_documents
            (community_id, document_id, knowledge_base_id, filename, mime_type)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (community_id, document_id) DO UPDATE SET
            knowledge_base_id = EXCLUDED.knowledge_base_id,
            filename = EXCLUDED.filename,
            mime_type = EXCLUDED.mime_type,
            ingested_at = NOW()
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(document_id)
    .bind(knowledge_base_id)
    .bind(filename)
    .bind(mime_type)
    .execute(pool)
    .await?;
    Ok(())
}

/// Upsert a table row from a Flow Studio table event.
pub async fn upsert_table_row(
    pool: &PgPool,
    community_id: CommunityId,
    table_id: &str,
    row_id: &str,
    row_json: &str,
) -> Result<()> {
    let row_value: serde_json::Value = serde_json::from_str(row_json)?;
    sqlx::query(
        r#"
        INSERT INTO flow_table_rows
            (community_id, table_id, row_id, row_json, updated_at, deleted_at)
        VALUES ($1, $2, $3, $4, NOW(), NULL)
        ON CONFLICT (community_id, table_id, row_id) DO UPDATE SET
            row_json = EXCLUDED.row_json,
            updated_at = NOW(),
            deleted_at = NULL
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(table_id)
    .bind(row_id)
    .bind(row_value)
    .execute(pool)
    .await?;
    Ok(())
}

/// Soft-delete a table row from a Flow Studio delete event.
pub async fn delete_table_row(
    pool: &PgPool,
    community_id: CommunityId,
    table_id: &str,
    row_id: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE flow_table_rows
        SET deleted_at = NOW(), updated_at = NOW()
        WHERE community_id = $1 AND table_id = $2 AND row_id = $3
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(table_id)
    .bind(row_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Index a single text chunk for keyword / vector search.
pub async fn upsert_knowledge_embedding(
    pool: &PgPool,
    community_id: CommunityId,
    document_id: &str,
    embedding_id: &str,
    chunk_index: i32,
    content: &str,
    embedding: &[f32],
) -> Result<()> {
    let vector_literal = format_pgvector(embedding);
    sqlx::query(
        r#"
        INSERT INTO flow_knowledge_embeddings
            (community_id, embedding_id, document_id, chunk_index, content, embedding)
        VALUES ($1, $2, $3, $4, $5, $6::vector)
        ON CONFLICT (community_id, embedding_id) DO UPDATE SET
            content = EXCLUDED.content,
            chunk_index = EXCLUDED.chunk_index,
            embedding = EXCLUDED.embedding,
            created_at = NOW()
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(embedding_id)
    .bind(document_id)
    .bind(chunk_index)
    .bind(content)
    .bind(vector_literal)
    .execute(pool)
    .await?;
    Ok(())
}

fn format_pgvector(values: &[f32]) -> String {
    let inner: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    format!("[{}]", inner.join(","))
}

/// Cosine-distance semantic search over indexed chunks (`<=>` operator).
pub async fn search_knowledge_semantic(
    pool: &PgPool,
    community_id: CommunityId,
    knowledge_base_id: &str,
    query_embedding: &[f32],
    limit: i64,
) -> Result<Vec<FlowKnowledgeSearchHit>> {
    let capped = limit.clamp(1, 50);
    let vector_literal = format_pgvector(query_embedding);
    let rows = sqlx::query(
        r#"
        SELECT e.document_id, e.chunk_index, e.content,
               (e.embedding <=> $3::vector) AS distance
        FROM flow_knowledge_embeddings e
        INNER JOIN flow_knowledge_documents d
            ON d.community_id = e.community_id AND d.document_id = e.document_id
        WHERE e.community_id = $1
          AND d.knowledge_base_id = $2
        ORDER BY distance ASC
        LIMIT $4
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(knowledge_base_id)
    .bind(vector_literal)
    .bind(capped)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(FlowKnowledgeSearchHit {
                document_id: row.try_get("document_id").ok()?,
                chunk_index: row.try_get("chunk_index").ok()?,
                content: row.try_get("content").ok()?,
            })
        })
        .collect())
}

/// List active rows for a table.
pub async fn list_table_rows(
    pool: &PgPool,
    community_id: CommunityId,
    table_id: &str,
    limit: i64,
) -> Result<Vec<FlowTableRowRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT row_id, row_json
        FROM flow_table_rows
        WHERE community_id = $1 AND table_id = $2 AND deleted_at IS NULL
        ORDER BY updated_at DESC
        LIMIT $3
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(table_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(FlowTableRowRecord {
                row_id: row.try_get("row_id").ok()?,
                row_json: row.try_get("row_json").ok()?,
            })
        })
        .collect())
}

/// Upsert file metadata from a Flow Studio file event.
pub async fn upsert_flow_file(
    pool: &PgPool,
    community_id: CommunityId,
    file_id: &str,
    filename: &str,
    media_url: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO flow_files
            (community_id, file_id, filename, media_url, version, deleted_at, updated_at)
        VALUES ($1, $2, $3, $4, 1, NULL, NOW())
        ON CONFLICT (community_id, file_id) DO UPDATE SET
            filename = EXCLUDED.filename,
            media_url = EXCLUDED.media_url,
            version = flow_files.version + 1,
            deleted_at = NULL,
            updated_at = NOW()
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(file_id)
    .bind(filename)
    .bind(media_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Soft-delete file metadata.
pub async fn delete_flow_file(
    pool: &PgPool,
    community_id: CommunityId,
    file_id: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE flow_files
        SET deleted_at = NOW(), updated_at = NOW()
        WHERE community_id = $1 AND file_id = $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(file_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// List active files for a community.
pub async fn list_flow_files(
    pool: &PgPool,
    community_id: CommunityId,
    limit: i64,
) -> Result<Vec<FlowFileRecord>> {
    let rows = sqlx::query(
        r#"
        SELECT file_id, filename, media_url, version
        FROM flow_files
        WHERE community_id = $1 AND deleted_at IS NULL
        ORDER BY updated_at DESC
        LIMIT $2
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(FlowFileRecord {
                file_id: row.try_get("file_id").ok()?,
                filename: row.try_get("filename").ok()?,
                media_url: row.try_get("media_url").ok(),
                version: row.try_get("version").ok()?,
            })
        })
        .collect())
}

/// Load the latest saved canvas graph for a flow id (kind 46200, `d` tag).
pub async fn get_latest_flow_graph(
    pool: &PgPool,
    community_id: CommunityId,
    flow_id: &str,
) -> Result<Option<String>> {
    let row = sqlx::query(
        r#"
        SELECT content
        FROM events
        WHERE community_id = $1
          AND kind = 46200
          AND channel_id IS NULL
          AND deleted_at IS NULL
          AND tags @> $2::jsonb
        ORDER BY created_at DESC, id ASC
        LIMIT 1
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(serde_json::json!([["d", flow_id]]))
    .fetch_optional(pool)
    .await?;

    Ok(row.and_then(|row| row.try_get("content").ok()))
}

/// Keyword search over indexed knowledge chunks and document filenames.
pub async fn search_knowledge_content(
    pool: &PgPool,
    community_id: CommunityId,
    knowledge_base_id: &str,
    query: &str,
    limit: i64,
) -> Result<Vec<FlowKnowledgeSearchHit>> {
    let capped = limit.clamp(1, 50);
    let pattern = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));

    let chunk_rows = sqlx::query(
        r#"
        SELECT e.document_id, e.chunk_index, e.content
        FROM flow_knowledge_embeddings e
        INNER JOIN flow_knowledge_documents d
            ON d.community_id = e.community_id AND d.document_id = e.document_id
        WHERE e.community_id = $1
          AND d.knowledge_base_id = $2
          AND e.content ILIKE $3 ESCAPE '\'
        ORDER BY e.created_at DESC
        LIMIT $4
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(knowledge_base_id)
    .bind(&pattern)
    .bind(capped)
    .fetch_all(pool)
    .await?;

    let mut hits: Vec<FlowKnowledgeSearchHit> = chunk_rows
        .into_iter()
        .filter_map(|row| {
            Some(FlowKnowledgeSearchHit {
                document_id: row.try_get("document_id").ok()?,
                chunk_index: row.try_get("chunk_index").ok()?,
                content: row.try_get("content").ok()?,
            })
        })
        .collect();

    if hits.len() >= capped as usize {
        return Ok(hits);
    }

    let remaining = capped - hits.len() as i64;
    let doc_rows = sqlx::query(
        r#"
        SELECT document_id, filename
        FROM flow_knowledge_documents
        WHERE community_id = $1
          AND knowledge_base_id = $2
          AND filename ILIKE $3 ESCAPE '\'
        ORDER BY ingested_at DESC
        LIMIT $4
        "#,
    )
    .bind(community_id.as_uuid())
    .bind(knowledge_base_id)
    .bind(&pattern)
    .bind(remaining)
    .fetch_all(pool)
    .await?;

    for row in doc_rows {
        let document_id: String = row.try_get("document_id").unwrap_or_default();
        if hits.iter().any(|hit| hit.document_id == document_id) {
            continue;
        }
        let filename: String = row.try_get("filename").unwrap_or_default();
        hits.push(FlowKnowledgeSearchHit {
            document_id,
            chunk_index: -1,
            content: filename,
        });
    }

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::tenant::CommunityId;
    use sqlx::PgPool;
    use uuid::Uuid;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz";

    async fn setup_pool() -> PgPool {
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| TEST_DB_URL.to_owned());
        PgPool::connect(&database_url)
            .await
            .expect("connect to test DB")
    }

    async fn make_community(pool: &PgPool) -> CommunityId {
        let id = Uuid::new_v4();
        let host = format!("flow-test-{}.example", id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(id)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert community");
        CommunityId::from_uuid(id)
    }

    /// Community A must not see Flow Studio read-model rows from community B.
    #[tokio::test]
    #[ignore = "requires Postgres with migration 0032 applied"]
    async fn flow_studio_read_model_is_confined_to_community() {
        let pool = setup_pool().await;
        let community_a = make_community(&pool).await;
        let community_b = make_community(&pool).await;

        upsert_table_row(
            &pool,
            community_a,
            "customers",
            "row-a",
            r#"{"name":"A-only"}"#,
        )
        .await
        .expect("insert row A");
        upsert_table_row(
            &pool,
            community_b,
            "customers",
            "row-b",
            r#"{"name":"B-only"}"#,
        )
        .await
        .expect("insert row B");

        let rows_a = list_table_rows(&pool, community_a, "customers", 10)
            .await
            .expect("list A");
        let rows_b = list_table_rows(&pool, community_b, "customers", 10)
            .await
            .expect("list B");

        assert_eq!(rows_a.len(), 1);
        assert_eq!(rows_b.len(), 1);
        assert_eq!(rows_a[0].row_id, "row-a");
        assert_eq!(rows_b[0].row_id, "row-b");

        upsert_flow_file(&pool, community_a, "file-a", "a.txt", None)
            .await
            .expect("file A");
        upsert_flow_file(&pool, community_b, "file-b", "b.txt", None)
            .await
            .expect("file B");

        let files_a = list_flow_files(&pool, community_a, 10)
            .await
            .expect("files A");
        let files_b = list_flow_files(&pool, community_b, 10)
            .await
            .expect("files B");

        assert!(files_a.iter().any(|f| f.file_id == "file-a"));
        assert!(!files_a.iter().any(|f| f.file_id == "file-b"));
        assert!(files_b.iter().any(|f| f.file_id == "file-b"));
        assert!(!files_b.iter().any(|f| f.file_id == "file-a"));
    }
}
