//! Consistent read-only PostgreSQL inventory. No membership-only filter: departed
//! users can still have uploads after leaving every channel.
use super::valid_hash;
use super::{invalid, MediaFile, MAX_FILES, MAX_FILE_BYTES};
use crate::{Db, Result};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Exact subject pairs. The human is selected across the database; agent
/// ownership is community-specific and never propagated to another community.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubjectScope {
    /// Normalized human public key.
    pub human: String,
    /// Explicitly included current owned-agent pairs.
    pub agents: BTreeSet<(Uuid, String)>,
}
impl SubjectScope {
    /// Whether a source is associated with the requested person/agent scope.
    pub fn contains(&self, community: Option<Uuid>, key: Option<&str>) -> bool {
        key == Some(self.human.as_str())
            || community
                .zip(key)
                .is_some_and(|(c, k)| self.agents.contains(&(c, k.into())))
    }
}
/// Evidence locates an upload record without persisting IP addresses or URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSource {
    /// Table or storage-record source name.
    pub table: String,
    /// Origin community, nullable for operator-global records.
    pub community: Option<Uuid>,
    /// Complete source primary key.
    pub key: Value,
    /// Hash of inspected payload, for later source revalidation.
    pub fingerprint: String,
    /// Field establishing the upload attribution.
    pub field: String,
    /// Source is associated with the requested subject.
    pub selected_subject: bool,
    /// Whether this is an uploader claim rather than a file reference.
    pub upload_claim: bool,
    /// True for a definite survivor, independent of future subject deletion.
    pub surviving: bool,
}
/// Output of completed DB scans. S3 scan and execution prerequisites are separate.
#[derive(Debug, Serialize, Deserialize)]
pub struct MediaScan {
    /// Database snapshot time; storage listing occurs separately afterwards.
    pub database_snapshot_at: String,
    /// Subject selection resolved before the upload snapshot.
    pub scope: SubjectScope,
    /// Deduplicated upload hashes before joining storage-only upload candidates.
    pub files: Vec<MediaFile>,
    /// Per-source inspected upload row counts.
    pub scanned: BTreeMap<String, usize>,
    /// Known coverage limitations; never converted into successful deletion checks.
    pub unresolved: Vec<String>,
}
#[derive(sqlx::FromRow)]
struct SourceRow {
    source_table: String,
    community_id: Option<Uuid>,
    source_key: Value,
    subject: Option<String>,
    payload: Value,
    direct_hash: Option<String>,
}

/// Operator-selected work limits; exceeding either aborts the entire observation.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MediaScanBudget {
    /// Maximum inspected database upload records.
    pub max_source_rows: usize,
    /// Per-statement deadline for the read-only source scan.
    pub statement_timeout_secs: u64,
}
impl Default for MediaScanBudget {
    fn default() -> Self {
        Self {
            max_source_rows: 1_000_000,
            statement_timeout_secs: 120,
        }
    }
}

impl Db {
    /// Scan upload records in all communities under one read-only repeatable
    /// read transaction. Bounds fail the whole scan; no partial inventory is returned.
    pub async fn scan_user_media(
        &self,
        human: &str,
        include_agents: bool,
        namespace: &str,
    ) -> Result<MediaScan> {
        self.scan_user_media_with_budget(
            human,
            include_agents,
            namespace,
            MediaScanBudget::default(),
        )
        .await
    }

    /// Scan with explicit limits for a larger source database. Memory and per-file
    /// bounds still apply; a larger row budget never permits partial publication.
    pub async fn scan_user_media_with_budget(
        &self,
        human: &str,
        include_agents: bool,
        namespace: &str,
        budget: MediaScanBudget,
    ) -> Result<MediaScan> {
        let scope = self.user_media_scope(human, include_agents).await?;
        self.scan_media_uploads(&scope, namespace, None, budget)
            .await
    }

    /// Resolve the exact subject without scanning upload history.
    pub async fn user_media_scope(
        &self,
        human: &str,
        include_agents: bool,
    ) -> Result<SubjectScope> {
        let human = crate::user_deletion::normalize_user_deletion_pubkey(human)?;
        let agents: Vec<(Uuid, String)> = if include_agents {
            let mut tx = self.pool().begin().await?;
            sqlx::query("SET TRANSACTION READ ONLY")
                .execute(&mut *tx)
                .await?;
            sqlx::query("SET LOCAL statement_timeout='120s'")
                .execute(&mut *tx)
                .await?;
            let agents = sqlx::query_as("SELECT community_id,encode(pubkey,'hex') FROM users WHERE agent_owner_pubkey=decode($1,'hex') ORDER BY community_id,pubkey LIMIT 10001")
                .bind(&human).fetch_all(&mut *tx).await?;
            tx.commit().await?;
            agents
        } else {
            vec![]
        };
        if agents.len() > 10_000 {
            return Err(invalid("owned-agent limit exceeded"));
        }
        Ok(SubjectScope {
            human,
            agents: agents.into_iter().collect(),
        })
    }

    /// Collect upload claims, optionally restricted to candidate hashes for
    /// pre-delete revalidation. This never inspects message contents or links.
    pub async fn scan_media_uploads(
        &self,
        scope: &SubjectScope,
        namespace: &str,
        hashes: Option<&[String]>,
        budget: MediaScanBudget,
    ) -> Result<MediaScan> {
        if budget.max_source_rows == 0 || budget.statement_timeout_secs == 0 {
            return Err(invalid("media scan budgets must be positive"));
        }
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        // Source keys/fingerprints must match downstream deletion sessions even
        // when their connection defaults use a different timezone.
        sqlx::query("SET LOCAL TIME ZONE 'UTC'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT set_config('statement_timeout', $1, true)")
            .bind(format!("{}s", budget.statement_timeout_secs))
            .execute(&mut *tx)
            .await?;
        let database_snapshot_at: String =
            sqlx::query_scalar("SELECT transaction_timestamp()::text")
                .fetch_one(&mut *tx)
                .await?;
        let mut scanned = BTreeMap::new();
        let mut sightings: BTreeMap<String, Vec<MediaSource>> = BTreeMap::new();
        let mut total_bytes = 0usize;
        let mut count = 0usize;
        {
            let mut rows = sqlx::query_as::<_, SourceRow>(include_str!("sources.sql"))
                .bind(hashes)
                .fetch(&mut *tx);
            while let Some(row) = rows.try_next().await? {
                count += 1;
                if count > budget.max_source_rows {
                    return Err(invalid(
                        "source scan exceeds max-source-rows; no partial inventory published",
                    ));
                }
                *scanned.entry(row.source_table.clone()).or_insert(0) += 1;
                let payload = serde_json::to_vec(&row.payload)?;
                if payload.len() > MAX_FILE_BYTES {
                    return Err(invalid("media source exceeds parser byte limit"));
                }
                let hash = row
                    .direct_hash
                    .as_ref()
                    .ok_or_else(|| invalid("upload claim has no hash"))?;
                if !valid_hash(hash) {
                    return Err(invalid("invalid upload hash"));
                }
                let actor = row
                    .subject
                    .as_deref()
                    .ok_or_else(|| invalid("upload claim has no actor"))?;
                crate::user_deletion::normalize_user_deletion_pubkey(actor)?;
                if row.community_id.is_none() {
                    return Err(invalid("upload claim has no community"));
                }
                if !row.payload["sha256"].is_null()
                    && row.payload["sha256"].as_str() != Some(hash.as_str())
                {
                    return Err(invalid("upload claim has an invalid or contradictory hash"));
                }
                let selected = scope.contains(row.community_id, Some(actor));
                let fingerprint = hex::encode(Sha256::digest(serde_json::to_vec(
                    &json!({"key":row.source_key,"community":row.community_id,"subject":row.subject,"payload":row.payload,"direct_hash":hash}),
                )?));
                let evidence = MediaSource {
                    table: row.source_table,
                    community: row.community_id,
                    key: row.source_key,
                    fingerprint,
                    field: "direct_hash".into(),
                    selected_subject: selected,
                    upload_claim: true,
                    surviving: !selected,
                };
                total_bytes += serde_json::to_vec(&evidence)?.len();
                if total_bytes > 128 * 1024 * 1024 {
                    return Err(invalid(
                        "media evidence exceeds 128 MiB; no partial inventory published",
                    ));
                }
                sightings.entry(hash.clone()).or_default().push(evidence);
                if sightings.len() > MAX_FILES {
                    return Err(invalid("media upload file limit exceeded"));
                }
            }
        }
        // The upload policy is version-bound in the stage; zero table
        // counts still appear in the scan receipt.
        for table in ["media_uploaders", "audit_log.media_uploaded"] {
            scanned.entry(table.into()).or_insert(0);
        }
        tx.commit().await?;
        let mut files = Vec::new();
        for (hash, sources) in sightings {
            let file = MediaFile {
                storage_namespace: namespace.into(),
                file_hash: hash,
                status: "unknown".into(),
                evidence_json: json!({"sources":sources}),
                delete_targets_json: json!([]),
                reason: None,
                last_error: None,
            };
            if serde_json::to_vec(&file)?.len() > MAX_FILE_BYTES {
                return Err(invalid("per-file evidence limit exceeded"));
            }
            files.push(file);
        }
        Ok(MediaScan {database_snapshot_at,scope:scope.clone(),files,scanned,unresolved:vec![
            "historical uploads without attribution cannot be assigned; absent records do not prove no earlier other uploader".into(),
        ]})
    }
}
