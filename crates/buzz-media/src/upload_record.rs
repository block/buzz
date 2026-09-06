//! Per-upload-event records — the moderation side channel.
//!
//! Content-addressed storage keys facts about *bytes*; moderation and legal
//! reporting (e.g. NCMEC CyberTipline) need facts about *upload events*: who
//! uploaded, when, from which network address. This module writes one small
//! JSON record per signed upload authorization — including the idempotent
//! re-upload short-circuit, which does no blob PUT and is therefore invisible
//! to any blob-creation-driven pipeline. Transport retries that reuse the same
//! signed authorization repair that record in place; a newly signed re-upload
//! creates a distinct record.
//!
//! Key layout (alongside the existing `_meta/` sidecar convention):
//!
//! ```text
//! _uploads/{community}/{sha256}/{event_id}.json
//! _upload_record_repairs/{community}/{sha256}/{event_id}.json
//! ```
//!
//! `event_id` is a time-sortable ULID derived from the signed Blossom upload
//! authorization event. Reusing that authorization for an idempotent request
//! retry therefore repairs the same record instead of appending a duplicate.
//! Fresh uploads first write the second key as a durable repair obligation.
//! It is removed only after the sidecar classification and moderation record
//! agree, so a relay restart or exhausted in-request correction cannot strand
//! an authoritative sidecar next to an incorrect record.
//! Records are unreachable through the media serve path by construction
//! (`validate_media_path` requires a bare 64-hex first segment), and the bucket
//! is only accessible via the relay's IAM role.
//!
//! The whole feature is **off by default** and gated behind
//! `BUZZ_MEDIA_UPLOAD_RECORDS`. IP collection is a second, independent opt-in
//! (`BUZZ_MEDIA_UPLOAD_IP_HEADER`) and is *fail-empty*: a missing, malformed,
//! or non-public address records nothing — a wrong IP is worse than no IP,
//! so absent is always preferable. The IP goes only into this
//! record — never blob metadata, never the upload response, never the
//! hash-chained audit log.
//!
//! ## Consumer contract (buzz-moderation)
//!
//! The moderation pipeline triggers on `ObjectCreated` events under the
//! `_uploads/` prefix and parses this record instead of HEADing blobs:
//!
//! - For fresh uploads, the record is written after the blob and derived
//!   artifacts but before the sidecar serve gate. Record existence therefore
//!   implies the scan inputs are readable, while record failure cannot leave
//!   unscanned media publicly servable.
//! - `ext`, `mime_type`, and `size` are always present so the consumer can
//!   derive the blob key (`{sha256}.{ext}`) and scan eligibility without
//!   extra round-trips.
//! - `uploader_name`, `ip`, and `port` are omitted (never `null`) when
//!   unknown or when collection is disabled.
//! - Consumers must tolerate unknown fields; `version` bumps only on
//!   breaking changes to existing fields.

use std::net::IpAddr;
use std::pin::Pin;

use buzz_core::tenant::TenantContext;
use serde::{Deserialize, Serialize};

/// Current record schema version. Additive fields do not bump this.
pub const UPLOAD_RECORD_VERSION: u32 = 1;

/// Root prefix scanned by the relay's bounded upload-record repair worker.
pub const UPLOAD_RECORD_REPAIR_PREFIX: &str = "_upload_record_repairs/";

const MAX_UPLOAD_RECORD_REPAIR_JSON_BYTES: usize = 64 * 1024;
const UPLOAD_RECORD_REPAIR_JOURNAL_VERSION: u32 = 1;
const UPLOAD_RECORD_INITIAL_WRITE_LEASE_SECS: i64 = 120;
const UPLOAD_RECORD_REPAIR_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const UPLOAD_RECORD_REPAIR_ITEM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// One accepted upload event. See module docs for the consumer contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadRecord {
    /// Schema version ([`UPLOAD_RECORD_VERSION`]).
    pub version: u32,
    /// Time-sortable ULID derived from the signed upload authorization event.
    /// Idempotent retries reuse it. Also the key suffix.
    pub event_id: String,
    /// Content hash of the uploaded bytes (64 lowercase hex chars).
    pub sha256: String,
    /// Canonical extension — consumers derive the blob key `{sha256}.{ext}`.
    pub ext: String,
    /// Sniffed MIME type of the uploaded bytes.
    pub mime_type: String,
    /// Size of the uploaded bytes.
    pub size: u64,
    /// Unix seconds when the relay accepted *this* upload event. On an
    /// idempotent re-upload this is the re-upload time, not the original
    /// blob's `uploaded_at`.
    pub uploaded_at: i64,
    /// Server-resolved community id (UUID). Never client-supplied.
    pub community_id: String,
    /// Server-resolved tenant host the upload was bound to.
    pub community_host: String,
    /// Authenticated uploader pubkey (64 lowercase hex chars).
    pub uploader_id: String,
    /// Same pubkey, bech32 `npub` encoding.
    pub uploader_npub: String,
    /// Uploader's configured display name at upload time. Best-effort label;
    /// `uploader_id` is authoritative. Omitted when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploader_name: Option<String>,
    /// Uploader's public IP as reported by the configured edge header.
    /// Present only when `BUZZ_MEDIA_UPLOAD_IP_HEADER` is set AND the header
    /// held a valid public address (fail-empty). Omitted, never `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    /// Uploader's source port as reported by the configured edge header.
    /// Standard edge headers don't carry the client port, so this is
    /// best-effort and usually absent. Only recorded alongside `ip`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

/// Network address facts extracted from trusted edge headers by the HTTP
/// handler, already validated (fail-empty). `Default` is "nothing collected".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UploadNetworkInfo {
    /// Validated public IP of the uploader, or `None`.
    pub ip: Option<IpAddr>,
    /// Source port of the uploader, or `None`. Ignored when `ip` is `None`.
    pub port: Option<u16>,
}

/// Per-event upload attribution passed into the upload pipeline by the
/// handler. Only consulted when upload records are enabled.
#[derive(Debug, Clone, Default)]
pub struct UploadAttribution {
    /// Uploader's configured display name, if known (sanitized, bounded).
    pub uploader_name: Option<String>,
    /// Validated network facts from trusted edge headers.
    pub net: UploadNetworkInfo,
}

/// Facts about the accepted upload, computed by the upload pipeline.
#[derive(Debug, Clone, Copy)]
pub struct UploadEventFacts<'a> {
    /// Content hash (64 lowercase hex chars).
    pub sha256: &'a str,
    /// Canonical extension.
    pub ext: &'a str,
    /// Sniffed MIME type.
    pub mime: &'a str,
    /// Uploaded byte size.
    pub size: u64,
    /// Unix seconds this upload event was accepted.
    pub uploaded_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UploadRecordRepairJournal {
    version: u32,
    record: UploadRecord,
    canonical_meta: crate::storage::BlobMeta,
    recover_missing_record_after: i64,
}

/// Durable repair obligation armed before a fresh upload record is written.
#[derive(Clone)]
pub(crate) struct PendingUploadRecordRepair {
    repair_key: String,
    record_key: String,
    record: UploadRecord,
}

impl PendingUploadRecordRepair {
    #[cfg(test)]
    pub(crate) fn repair_key(&self) -> &str {
        &self.repair_key
    }

    #[cfg(test)]
    pub(crate) fn record_key(&self) -> &str {
        &self.record_key
    }
}

/// Result of one bounded page processed by the independent repair worker.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UploadRecordRepairPage {
    /// Obligations whose moderation record was made canonical and then cleared.
    pub repaired: usize,
    /// Obligations waiting for a sidecar to be published.
    pub waiting_for_sidecar: usize,
    /// Obligations whose bounded initial record write may still be in flight.
    pub waiting_for_record: usize,
    /// Malformed or transiently failed obligations left durable for a later pass.
    pub failed: usize,
    /// Continuation token for the next bounded page.
    pub next_continuation_token: Option<String>,
    /// Whether more obligations existed when this page was listed.
    pub is_truncated: bool,
}

fn validated_repair_continuation_token(
    is_truncated: bool,
    continuation_token: Option<String>,
) -> Result<Option<String>, crate::error::MediaError> {
    if is_truncated && continuation_token.is_none() {
        return Err(crate::error::MediaError::StorageError(
            "truncated upload-record repair page has no continuation token".to_string(),
        ));
    }
    Ok(continuation_token)
}

fn build_upload_record(
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    attribution: &UploadAttribution,
    facts: UploadEventFacts<'_>,
) -> Result<UploadRecord, crate::error::MediaError> {
    use nostr::ToBech32;

    let event_id = upload_event_id(auth_event);
    // Ports are only meaningful next to the address they were observed with.
    let ip = attribution.net.ip;
    let port = ip.and(attribution.net.port);
    Ok(UploadRecord {
        version: UPLOAD_RECORD_VERSION,
        event_id,
        sha256: facts.sha256.to_string(),
        ext: facts.ext.to_string(),
        mime_type: facts.mime.to_string(),
        size: facts.size,
        uploaded_at: facts.uploaded_at,
        community_id: ctx.community().to_string(),
        community_host: ctx.host().to_string(),
        uploader_id: auth_event.pubkey.to_hex(),
        uploader_npub: auth_event.pubkey.to_bech32().map_err(|e| {
            // Unreachable for a valid pubkey; surfaced rather than unwrapped.
            crate::error::MediaError::StorageError(format!("npub encoding failed: {e}"))
        })?,
        uploader_name: attribution.uploader_name.clone(),
        ip: ip.map(|addr| addr.to_string()),
        port,
    })
}

/// Build and store the per-event record for one accepted upload.
///
/// Called after blob and derived-artifact durability but before the sidecar
/// publish gate on fresh uploads; called on the existing published state for
/// idempotent re-uploads. A write failure propagates and fails the upload. The
/// record's `ObjectCreated` event is the moderation pipeline's only scan
/// trigger, so no newly published media may exist without a record.
pub async fn record_upload_event(
    storage: &crate::storage::MediaStorage,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    attribution: &UploadAttribution,
    facts: UploadEventFacts<'_>,
) -> Result<(), crate::error::MediaError> {
    let record = build_upload_record(ctx, auth_event, attribution, facts)?;
    let key = upload_record_key(ctx, facts.sha256, &record.event_id);
    let json = serde_json::to_vec(&record)?;
    storage.put(&key, &json, "application/json").await
}

/// Persist a repair obligation before writing a fresh moderation record.
pub(crate) async fn arm_upload_record_repair(
    storage: &crate::storage::MediaStorage,
    ctx: &TenantContext,
    auth_event: &nostr::Event,
    attribution: &UploadAttribution,
    facts: UploadEventFacts<'_>,
    canonical_meta: &crate::storage::BlobMeta,
) -> Result<PendingUploadRecordRepair, crate::error::MediaError> {
    let record = build_upload_record(ctx, auth_event, attribution, facts)?;
    let record_key = upload_record_key(ctx, facts.sha256, &record.event_id);
    let repair_key = upload_record_repair_key(ctx, facts.sha256, &record.event_id);
    let journal = serde_json::to_vec(&UploadRecordRepairJournal {
        version: UPLOAD_RECORD_REPAIR_JOURNAL_VERSION,
        record: record.clone(),
        canonical_meta: canonical_meta.clone(),
        recover_missing_record_after: chrono::Utc::now()
            .timestamp()
            .saturating_add(UPLOAD_RECORD_INITIAL_WRITE_LEASE_SECS),
    })?;
    if !storage
        .put_if_absent(&repair_key, &journal, "application/json")
        .await?
    {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair already in progress for this authorization".to_string(),
        ));
    }
    Ok(PendingUploadRecordRepair {
        repair_key,
        record_key,
        record,
    })
}

/// Write one version of the moderation record while retaining its durable journal.
pub(crate) async fn write_upload_record_facts(
    storage: &crate::storage::MediaStorage,
    repair: &PendingUploadRecordRepair,
    meta: &crate::storage::BlobMeta,
) -> Result<(), crate::error::MediaError> {
    let mut record = repair.record.clone();
    record.ext.clone_from(&meta.ext);
    record.mime_type.clone_from(&meta.mime_type);
    let json = serde_json::to_vec(&record)?;
    storage
        .put(&repair.record_key, &json, "application/json")
        .await
}

/// Clear a repair obligation only after the record and sidecar facts converge.
pub(crate) async fn clear_upload_record_repair(
    storage: &crate::storage::MediaStorage,
    repair: &PendingUploadRecordRepair,
) -> Result<(), crate::error::MediaError> {
    storage.delete(&repair.repair_key).await
}

fn upload_record_repair_key(ctx: &TenantContext, sha256: &str, event_id: &str) -> String {
    format!(
        "{UPLOAD_RECORD_REPAIR_PREFIX}{}/{sha256}/{event_id}.json",
        ctx.community()
    )
}

fn record_keys_from_journal(
    journal: &UploadRecordRepairJournal,
) -> Result<(String, String), crate::error::MediaError> {
    let record = &journal.record;
    let community = uuid::Uuid::parse_str(&record.community_id).map_err(|_| {
        crate::error::MediaError::StorageError(
            "upload-record repair has invalid community id".to_string(),
        )
    })?;
    if community.to_string() != record.community_id
        || record.sha256.len() != 64
        || !record
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !matches!(
            ulid::Ulid::from_string(&record.event_id),
            Ok(event_id) if event_id.to_string() == record.event_id
        )
    {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair has invalid canonical identity".to_string(),
        ));
    }
    Ok((
        format!(
            "_uploads/{}/{}/{}.json",
            record.community_id, record.sha256, record.event_id
        ),
        format!("_meta/{}/{}.json", record.community_id, record.sha256),
    ))
}

/// Outcome of one independently fenced upload-record repair attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadRecordRepairOutcome {
    /// Record and sidecar facts converged and the journal was cleared.
    Repaired,
    /// The initial request still owns its bounded record-write window.
    WaitingForRecord,
    /// No authoritative sidecar exists yet.
    WaitingForSidecar,
}

/// Boxed operation passed through the relay's deletion-fence protector.
pub type UploadRecordRepairFuture<'a> = Pin<
    Box<
        dyn std::future::Future<
                Output = Result<UploadRecordRepairOutcome, crate::error::MediaError>,
            > + Send
            + 'a,
    >,
>;

/// Holds the community serving-write fence around one repair's object mutations.
pub trait UploadRecordRepairProtector: Send + Sync {
    /// Protect `operation` against the whole-community deletion fence.
    fn protect<'a>(
        &'a self,
        community: buzz_core::tenant::CommunityId,
        operation: UploadRecordRepairFuture<'a>,
    ) -> UploadRecordRepairFuture<'a>;
}

async fn process_upload_record_repair(
    storage: &crate::storage::MediaStorage,
    repair_key: &str,
) -> Result<UploadRecordRepairOutcome, crate::error::MediaError> {
    let bytes = get_bounded_repair_json(storage, repair_key).await?;
    let mut journal: UploadRecordRepairJournal = serde_json::from_slice(&bytes)?;
    validate_repair_versions(journal.version, journal.record.version)?;
    let (record_key, sidecar_key) = record_keys_from_journal(&journal)?;
    let expected_repair_key = format!(
        "{UPLOAD_RECORD_REPAIR_PREFIX}{}/{}/{}.json",
        journal.record.community_id, journal.record.sha256, journal.record.event_id
    );
    if repair_key != expected_repair_key {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair key does not match its payload".to_string(),
        ));
    }
    validate_repair_meta(&journal.record, &journal.canonical_meta)?;
    let record_exists = match get_bounded_repair_json(storage, &record_key).await {
        Ok(_) => true,
        Err(crate::error::MediaError::NotFound) => false,
        Err(error) => return Err(error),
    };
    if !record_exists && chrono::Utc::now().timestamp() < journal.recover_missing_record_after {
        return Ok(UploadRecordRepairOutcome::WaitingForRecord);
    }

    let canonical_blob_key = format!("{}.{}", journal.record.sha256, journal.canonical_meta.ext);
    if !storage.head(&canonical_blob_key).await? {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair canonical metadata references a missing blob".to_string(),
        ));
    }
    if !record_exists {
        storage
            .put(
                &record_key,
                &serde_json::to_vec(&journal.record)?,
                "application/json",
            )
            .await?;
    }

    let canonical_sidecar = serde_json::to_vec(&journal.canonical_meta)?;
    let sidecar = if storage
        .put_if_absent(&sidecar_key, &canonical_sidecar, "application/json")
        .await?
    {
        journal.canonical_meta
    } else {
        match get_bounded_repair_json(storage, &sidecar_key).await {
            Ok(bytes) => serde_json::from_slice::<crate::storage::BlobMeta>(&bytes)?,
            Err(crate::error::MediaError::NotFound) => {
                return Ok(UploadRecordRepairOutcome::WaitingForSidecar);
            }
            Err(error) => return Err(error),
        }
    };
    validate_sidecar_classification(&sidecar)?;
    let published_blob_key = format!("{}.{}", journal.record.sha256, sidecar.ext);
    if !storage.head(&published_blob_key).await? {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair sidecar references a missing blob".to_string(),
        ));
    }
    journal.record.ext = sidecar.ext;
    journal.record.mime_type = sidecar.mime_type;
    storage
        .put(
            &record_key,
            &serde_json::to_vec(&journal.record)?,
            "application/json",
        )
        .await?;
    storage.delete(repair_key).await?;
    Ok(UploadRecordRepairOutcome::Repaired)
}

fn validate_repair_versions(
    journal_version: u32,
    record_version: u32,
) -> Result<(), crate::error::MediaError> {
    if journal_version != UPLOAD_RECORD_REPAIR_JOURNAL_VERSION
        || record_version != UPLOAD_RECORD_VERSION
    {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair has an unsupported schema version".to_string(),
        ));
    }
    Ok(())
}

fn validate_repair_meta(
    record: &UploadRecord,
    meta: &crate::storage::BlobMeta,
) -> Result<(), crate::error::MediaError> {
    validate_sidecar_classification(meta)?;
    if meta.ext != record.ext || meta.mime_type != record.mime_type || meta.size != record.size {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair canonical metadata does not match its record".to_string(),
        ));
    }
    Ok(())
}

fn validate_sidecar_classification(
    sidecar: &crate::storage::BlobMeta,
) -> Result<(), crate::error::MediaError> {
    if sidecar.ext.is_empty()
        || sidecar.ext.len() > 8
        || !sidecar.ext.bytes().all(|byte| byte.is_ascii_alphanumeric())
        || sidecar.mime_type.is_empty()
    {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair sidecar has invalid classification".to_string(),
        ));
    }
    Ok(())
}

async fn get_bounded_repair_json(
    storage: &crate::storage::MediaStorage,
    key: &str,
) -> Result<Vec<u8>, crate::error::MediaError> {
    let bytes = storage
        .get_range(key, 0, MAX_UPLOAD_RECORD_REPAIR_JSON_BYTES as u64)
        .await?;
    if bytes.len() > MAX_UPLOAD_RECORD_REPAIR_JSON_BYTES {
        return Err(crate::error::MediaError::StorageError(
            "upload-record repair JSON exceeds 64 KiB".to_string(),
        ));
    }
    Ok(bytes)
}

/// Process one durable upload-record repair while preserving an S3 cursor.
///
/// Listing failure propagates because the relay cannot recover without list
/// access. Individual obligations remain durable and are counted as failures so
/// one malformed or transiently unavailable item cannot starve later entries.
pub async fn process_upload_record_repair_page(
    storage: &crate::storage::MediaStorage,
    protector: &impl UploadRecordRepairProtector,
    continuation_token: Option<String>,
) -> Result<UploadRecordRepairPage, crate::error::MediaError> {
    let page = tokio::time::timeout(
        UPLOAD_RECORD_REPAIR_LIST_TIMEOUT,
        storage.list_prefix_page(UPLOAD_RECORD_REPAIR_PREFIX, continuation_token, 1),
    )
    .await
    .map_err(|_| {
        crate::error::MediaError::StorageError(
            "upload-record repair listing exceeded its deadline".to_string(),
        )
    })??;
    let next_continuation_token =
        validated_repair_continuation_token(page.is_truncated, page.next_continuation_token)?;
    let mut outcome = UploadRecordRepairPage {
        next_continuation_token,
        is_truncated: page.is_truncated,
        ..UploadRecordRepairPage::default()
    };
    for (repair_key, _) in page.objects {
        let community = match crate::bucket_index::classify_key(&repair_key) {
            crate::bucket_index::KeyClass::UploadRecordRepair { community, .. } => {
                buzz_core::tenant::CommunityId::from_uuid(community)
            }
            _ => {
                outcome.failed += 1;
                tracing::warn!(
                    repair_key,
                    "malformed upload-record repair key remains pending"
                );
                continue;
            }
        };
        let repair = protector.protect(
            community,
            Box::pin(process_upload_record_repair(storage, &repair_key)),
        );
        match tokio::time::timeout(UPLOAD_RECORD_REPAIR_ITEM_TIMEOUT, repair).await {
            Err(_) => {
                outcome.failed += 1;
                tracing::warn!(
                    repair_key,
                    "upload-record repair timed out and remains pending"
                );
            }
            Ok(Ok(UploadRecordRepairOutcome::Repaired)) => outcome.repaired += 1,
            Ok(Ok(UploadRecordRepairOutcome::WaitingForRecord)) => {
                outcome.waiting_for_record += 1;
            }
            Ok(Ok(UploadRecordRepairOutcome::WaitingForSidecar)) => {
                outcome.waiting_for_sidecar += 1;
            }
            Ok(Err(error)) => {
                outcome.failed += 1;
                tracing::warn!(
                    repair_key,
                    error = %error,
                    "upload-record repair remains pending"
                );
            }
        }
    }
    Ok(outcome)
}

fn upload_event_id(auth_event: &nostr::Event) -> String {
    let event_bytes = auth_event.id.to_bytes();
    let mut random_bytes = [0u8; 16];
    random_bytes[6..].copy_from_slice(&event_bytes[..10]);
    let random = u128::from_be_bytes(random_bytes);
    ulid::Ulid::from_parts(
        auth_event.created_at.as_secs().saturating_mul(1_000),
        random,
    )
    .to_string()
}

/// Build the per-event record key:
/// `_uploads/{community}/{sha256}/{event_id}.json`.
///
/// `ctx` must be the server-resolved request tenant — same fence as the
/// `_meta/` sidecar key (see [`crate::storage::MediaStorage::sidecar_key`]).
pub fn upload_record_key(ctx: &TenantContext, sha256: &str, event_id: &str) -> String {
    format!("_uploads/{}/{sha256}/{event_id}.json", ctx.community())
}

/// Parse an IP header value, accepting only public addresses (fail-empty).
///
/// Returns `None` — record nothing — for anything that is not a single,
/// syntactically valid, public IP: garbage, comma lists, private ranges,
/// loopback, link-local, CGNAT, multicast, documentation, ULA, etc. Never
/// guesses and never falls back to the socket address.
pub fn parse_public_ip(raw: &str) -> Option<IpAddr> {
    let ip: IpAddr = raw.trim().parse().ok()?;
    is_public_ip(&ip).then_some(ip)
}

/// Parse a port header value: a single decimal u16, non-zero.
pub fn parse_port(raw: &str) -> Option<u16> {
    raw.trim().parse::<u16>().ok().filter(|&p| p != 0)
}

/// Conservative "is this a public, routable address" check.
///
/// `IpAddr::is_global` is unstable, so this enumerates the reserved ranges
/// explicitly. Anything not recognizably public is rejected — the cost of a
/// false negative is an absent field; the cost of a false positive is a wrong
/// address in a federal report.
fn is_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || v4.is_unspecified()
                // This network 0.0.0.0/8 (RFC 1122)
                || octets[0] == 0
                // CGNAT 100.64.0.0/10 (RFC 6598)
                || (octets[0] == 100 && (octets[1] & 0b1100_0000) == 64)
                // Reserved 240.0.0.0/4 (RFC 1112) — is_broadcast covers .255 only
                || octets[0] >= 240
                // Benchmarking 198.18.0.0/15 (RFC 2544)
                || (octets[0] == 198 && (octets[1] & 0xFE) == 18)
                // IETF protocol assignments 192.0.0.0/24 (RFC 6890)
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0))
        }
        IpAddr::V6(v6) => {
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                // Unique local fc00::/7 (RFC 4193)
                || (seg[0] & 0xFE00) == 0xFC00
                // Link-local fe80::/10 (RFC 4291)
                || (seg[0] & 0xFFC0) == 0xFE80
                // Discard-only 100::/64 (RFC 6666)
                || (seg[0] == 0x0100 && seg[1..4] == [0, 0, 0])
                // Teredo 2001::/32 (RFC 4380)
                || (seg[0] == 0x2001 && seg[1] == 0)
                // Benchmarking 2001:2::/48 (RFC 5180)
                || (seg[0] == 0x2001 && seg[1] == 2 && seg[2] == 0)
                // Documentation 2001:db8::/32 (RFC 3849)
                || (seg[0] == 0x2001 && seg[1] == 0x0DB8)
                // 6to4 2002::/16 (RFC 3056)
                || seg[0] == 0x2002
                // Documentation 3fff::/20 (RFC 9637)
                || (seg[0] & 0xFFF0) == 0x3FF0
                // IPv4-mapped ::ffff:0:0/96 — the embedded v4 was already
                // rejected above if it arrived as dotted quad; reject the
                // mapped form outright rather than re-deriving it.
                || v6.to_ipv4_mapped().is_some())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::tenant::CommunityId;

    fn tenant() -> TenantContext {
        TenantContext::resolved(
            CommunityId::from_uuid(uuid::Uuid::from_u128(7)),
            "chat.example.com",
        )
    }

    #[test]
    fn record_key_matches_spec_layout() {
        let ctx = tenant();
        let sha = "a".repeat(64);
        let key = upload_record_key(&ctx, &sha, "01J9W3ULIDULIDULIDULIDULID");
        assert_eq!(
            key,
            format!(
                "_uploads/{}/{sha}/01J9W3ULIDULIDULIDULIDULID.json",
                ctx.community()
            )
        );
    }

    #[test]
    fn truncated_repair_page_without_cursor_fails_closed() {
        assert!(validated_repair_continuation_token(true, None).is_err());
        assert_eq!(
            validated_repair_continuation_token(true, Some("next".to_string())).unwrap(),
            Some("next".to_string())
        );
    }

    #[test]
    fn repair_journal_rejects_unknown_schema_versions() {
        assert!(validate_repair_versions(1, UPLOAD_RECORD_VERSION).is_ok());
        assert!(validate_repair_versions(2, UPLOAD_RECORD_VERSION).is_err());
        assert!(validate_repair_versions(1, UPLOAD_RECORD_VERSION + 1).is_err());
    }

    #[test]
    fn upload_auth_event_id_is_stable_for_idempotent_record_writes() {
        use nostr::{EventBuilder, Keys, Kind, Timestamp};

        let auth = EventBuilder::new(Kind::from(24242), "Upload")
            .custom_created_at(Timestamp::from(1_700_000_000))
            .sign_with_keys(&Keys::generate())
            .unwrap();

        let first = upload_event_id(&auth);
        let retry = upload_event_id(&auth);

        assert_eq!(first, retry);
        assert_eq!(first.len(), 26);
        assert_eq!(
            ulid::Ulid::from_string(&first).unwrap().timestamp_ms(),
            1_700_000_000_000
        );
    }

    #[test]
    fn record_serializes_full_shape() {
        let record = UploadRecord {
            version: UPLOAD_RECORD_VERSION,
            event_id: "01J9W3TEST".into(),
            sha256: "b".repeat(64),
            ext: "png".into(),
            mime_type: "image/png".into(),
            size: 12345,
            uploaded_at: 1_783_358_352,
            community_id: uuid::Uuid::from_u128(7).to_string(),
            community_host: "chat.example.com".into(),
            uploader_id: "c".repeat(64),
            uploader_npub: "npub1example".into(),
            uploader_name: Some("alice".into()),
            ip: Some("203.0.113.7".into()),
            port: Some(51234),
        };
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["ext"], "png");
        assert_eq!(json["mime_type"], "image/png");
        assert_eq!(json["size"], 12345);
        assert_eq!(json["ip"], "203.0.113.7");
        assert_eq!(json["port"], 51234);
        assert_eq!(json["uploader_name"], "alice");
    }

    #[test]
    fn record_omits_absent_optionals_entirely() {
        let record = UploadRecord {
            version: UPLOAD_RECORD_VERSION,
            event_id: "01J9W3TEST".into(),
            sha256: "b".repeat(64),
            ext: "mp4".into(),
            mime_type: "video/mp4".into(),
            size: 1,
            uploaded_at: 0,
            community_id: "cid".into(),
            community_host: "h".into(),
            uploader_id: "c".repeat(64),
            uploader_npub: "npub1example".into(),
            uploader_name: None,
            ip: None,
            port: None,
        };
        let json = serde_json::to_value(&record).unwrap();
        // Omitted, not null — the consumer contract.
        assert!(json.get("uploader_name").is_none());
        assert!(json.get("ip").is_none());
        assert!(json.get("port").is_none());
    }

    #[test]
    fn record_deserialization_tolerates_unknown_fields() {
        // Forward-compat: additive relay fields must not break older parsers
        // of the same version (mirrors the consumer's requirement).
        let json = r#"{
            "version": 1, "event_id": "01J", "sha256": "ab", "ext": "png",
            "mime_type": "image/png", "size": 1, "uploaded_at": 2,
            "community_id": "c", "community_host": "h",
            "uploader_id": "u", "uploader_npub": "n",
            "some_future_field": {"nested": true}
        }"#;
        let record: UploadRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.version, 1);
        assert_eq!(record.ip, None);
    }

    #[test]
    fn public_ips_accepted() {
        for raw in [
            "8.8.8.8",
            "1.1.1.1",
            "  93.184.216.34  ", // trims whitespace
            "2600:1f18::1",
            "2a00:1450:4009:81f::200e",
        ] {
            assert!(parse_public_ip(raw).is_some(), "should accept {raw}");
        }
    }

    #[test]
    fn non_public_ips_fail_empty() {
        for raw in [
            "",
            "not-an-ip",
            "10.0.0.1",         // private
            "172.16.0.1",       // private
            "192.168.1.1",      // private
            "127.0.0.1",        // loopback
            "169.254.1.1",      // link-local
            "100.64.0.1",       // CGNAT
            "100.127.255.255",  // CGNAT upper edge
            "0.0.0.0",          // unspecified
            "0.1.2.3",          // this network 0.0.0.0/8
            "255.255.255.255",  // broadcast
            "224.0.0.1",        // multicast
            "240.0.0.1",        // reserved
            "198.18.0.1",       // benchmarking
            "192.0.0.1",        // IETF assignments
            "203.0.113.7",      // documentation (TEST-NET-3)
            "198.51.100.1",     // documentation (TEST-NET-2)
            "192.0.2.1",        // documentation (TEST-NET-1)
            "::1",              // v6 loopback
            "::",               // v6 unspecified
            "fe80::1",          // v6 link-local
            "fc00::1",          // v6 ULA
            "fd12:3456::1",     // v6 ULA
            "ff02::1",          // v6 multicast
            "100::1",           // v6 discard-only
            "2001::1",          // Teredo
            "2001:2::1",        // v6 benchmarking
            "2001:db8::1",      // v6 documentation
            "2002::1",          // 6to4
            "3fff::1",          // v6 documentation
            "::ffff:8.8.8.8",   // v4-mapped — reject the mapped form
            "8.8.8.8, 1.1.1.1", // comma list — not a single IP
            "8.8.8.8:443",      // ip:port — not a bare IP
        ] {
            assert!(parse_public_ip(raw).is_none(), "should reject {raw:?}");
        }
    }

    #[test]
    fn port_parses_single_nonzero_u16() {
        assert_eq!(parse_port("51234"), Some(51234));
        assert_eq!(parse_port(" 443 "), Some(443));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port("65536"), None);
        assert_eq!(parse_port("-1"), None);
        assert_eq!(parse_port("443, 444"), None);
        assert_eq!(parse_port("abc"), None);
        assert_eq!(parse_port(""), None);
    }
}
