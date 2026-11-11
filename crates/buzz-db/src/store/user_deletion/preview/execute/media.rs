//! A completed media stage is necessary but not sufficient: each reference must
//! still have the exact source fingerprint inspected by that stage.
use super::*;
use crate::media_deletion::{MediaSource, MAX_FILE_BYTES};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

pub(super) struct Evidence {
    stage: Uuid,
    namespace: String,
    bases: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct Source {
    source_table: String,
    community_id: Option<Uuid>,
    source_key: Value,
    subject: Option<String>,
    payload: Value,
    direct_hash: Option<String>,
    upload_claim: bool,
    subject_owned: bool,
}

impl Evidence {
    pub async fn load(
        tx: &mut Transaction<'_, Postgres>,
        context: &stage::Context<'_>,
    ) -> Result<Self> {
        let stages: Vec<(Uuid, Value)> = sqlx::query_as("SELECT id,metadata_json FROM entity_deletion_stage_history WHERE request_id=$1 AND stage='purge_media' AND target_community_id IS NULL AND status='completed' AND NOT is_current LIMIT 2")
            .bind(context.lease.request_id).fetch_all(&mut **tx).await?;
        let [(stage, metadata)] = stages.as_slice() else {
            return Err(invalid(
                "content deletion requires one completed request-wide media stage",
            ));
        };
        let binding = &metadata["binding"];
        if binding["mode"] != "delete"
            || binding["human"] != normalize_user_deletion_pubkey(&context.options.pubkey)?
            || metadata["media_discovery"]["complete"] != true
        {
            return Err(invalid(
                "media stage has no completed destructive inventory for this user",
            ));
        }
        let namespace = binding["storage_namespace"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid("media namespace missing"))?
            .to_owned();
        let mut bases: Vec<String> = serde_json::from_value(binding["media_bases"].clone())?;
        let hosts: Vec<String> =
            sqlx::query_scalar("SELECT host FROM communities ORDER BY id LIMIT 10001")
                .fetch_all(&mut **tx)
                .await?;
        if hosts.len() > 10_000 {
            return Err(invalid("media community limit exceeded"));
        }
        bases.extend(hosts.into_iter().map(|h| format!("https://{h}/media")));
        bases.push("https://invalid.example/media".into());
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM entity_deletion_media_files WHERE stage_id=$1",
        )
        .bind(stage)
        .fetch_one(&mut **tx)
        .await?;
        if metadata["media_discovery"]["candidate_count"] != json!(count) {
            return Err(invalid("completed media inventory row count changed"));
        }
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM entity_deletion_media_files WHERE stage_id=$1 AND (storage_namespace<>$2 OR status NOT IN ('deleted','dont_delete') OR last_error IS NOT NULL))")
            .bind(stage).bind(&namespace).fetch_one(&mut **tx).await?;
        if pending {
            return Err(invalid(
                "media stage has unfinished or mismatched file evidence",
            ));
        }
        Ok(Self {
            stage: *stage,
            namespace,
            bases,
        })
    }

    pub async fn verify_source(
        &self,
        conn: &mut PgConnection,
        community: Uuid,
        table: &str,
        predicate: &str,
        key: &Value,
    ) -> Result<()> {
        let source_key = match table {
            "events" => "jsonb_build_array(($2->>'created_at')::timestamptz,$2->>'id')",
            "workflows" | "workflow_runs" => "to_jsonb(($2->>'id')::uuid)",
            "workflow_approvals" => "to_jsonb($2->>'token_hash')",
            "subscriptions" => "to_jsonb($2->>'id')",
            "delivery_log" => {
                "jsonb_build_array(($2->>'delivered_at')::timestamptz,($2->>'id')::bigint)"
            }
            _ => return Ok(()),
        };
        // Reuse the scanner's source definition, including attribution and
        // omitted diagnostic fields. A one-row CTE preserves the indexed key
        // predicate: filtering only the computed JSON source_key would scan
        // the whole table once per deleted row. Identifiers come exclusively
        // from selection's static allowlist, never caller input.
        let sql = format!("WITH {table} AS MATERIALIZED (SELECT * FROM public.{table} WHERE community_id=$1 AND {predicate}) SELECT * FROM ({}) sources WHERE source_table=$3 AND community_id=$1 AND source_key={source_key}", include_str!("../../../media_deletion/sources.sql"));
        let row: Source = sqlx::query_as(sqlx::AssertSqlSafe(sql))
            .bind(community)
            .bind(key)
            .bind(table)
            .fetch_optional(&mut *conn)
            .await?
            .ok_or_else(|| invalid("content media source disappeared"))?;
        if serde_json::to_vec(&row.payload)?.len() > MAX_FILE_BYTES {
            return Err(invalid("content media source exceeds parser limit"));
        }
        let references =
            buzz_core::media_inventory::references(&row.payload, &self.bases).map_err(invalid)?;
        let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(&json!({
            "key":row.source_key,"community":row.community_id,"subject":row.subject,
            "payload":row.payload,"direct_hash":row.direct_hash,"upload_claim":row.upload_claim,
            "subject_owned":row.subject_owned
        }))?));
        for (hash, field) in references {
            let evidence: Option<(String, Value)> = sqlx::query_as("SELECT status,evidence_json FROM entity_deletion_media_files WHERE stage_id=$1 AND storage_namespace=$2 AND file_hash=$3 AND last_error IS NULL")
                .bind(self.stage).bind(&self.namespace).bind(&hash).fetch_optional(&mut *conn).await?;
            let Some((status, evidence)) = evidence else {
                return Err(invalid(
                    "content reference is absent from completed media evidence",
                ));
            };
            if !matches!(status.as_str(), "deleted" | "dont_delete") {
                return Err(invalid("content reference has unfinished media work"));
            }
            let sources: Vec<MediaSource> = serde_json::from_value(evidence["sources"].clone())?;
            if !sources.iter().any(|source| {
                source.table == row.source_table
                    && source.community == row.community_id
                    && source.key == row.source_key
                    && source.fingerprint == fingerprint
                    && source.field == field
            }) {
                return Err(invalid("content source changed since media was checked"));
            }
        }
        Ok(())
    }
}
