//! Per-upload-event records — the moderation side channel.
//!
//! Content-addressed storage keys facts about *bytes*; moderation and legal
//! reporting (e.g. NCMEC CyberTipline) need facts about *upload events*: who
//! uploaded, when, from which network address. This module writes one small
//! append-only JSON record per **accepted** upload — including the idempotent
//! re-upload short-circuit, which does no blob PUT and is therefore invisible
//! to any blob-creation-driven pipeline.
//!
//! Key layout (alongside the existing `_meta/` sidecar convention):
//!
//! ```text
//! _uploads/{community}/{sha256}/{event_id}.json
//! ```
//!
//! `event_id` is a ULID — unique and time-sortable, one record per accepted
//! upload event. Records are unreachable through the media serve path by
//! construction (`validate_media_path` requires a bare 64-hex first segment),
//! and the bucket is only accessible via the relay's IAM role.
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
//! - For fresh uploads, the record is written after immutable blob staging and
//!   the canonical PostgreSQL publication commit, but before cache projections.
//!   Record existence therefore implies the scan inputs are readable. A record
//!   failure leaves the DB-witnessed object authoritative but unprojected for
//!   moderation retry; a sidecar never grants canonical visibility.
//! - `ext`, `mime_type`, and `size` are always present so the consumer can
//!   derive the blob key (`{sha256}.{ext}`) and scan eligibility without
//!   extra round-trips.
//! - `uploader_name`, `ip`, and `port` are omitted (never `null`) when
//!   unknown or when collection is disabled.
//! - Consumers must tolerate unknown fields; `version` bumps only on
//!   breaking changes to existing fields.

use std::fmt;
use std::net::IpAddr;

use buzz_core::tenant::TenantContext;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const UPLOAD_RECORD_PLAN_PREFIX: &str = "_publication-plans/moderation";
const UPLOAD_RECORD_REPLAY_PREFIX: &str = "_publication-plans/moderation-replay";
const MAX_UPLOAD_RECORD_BYTES: u64 = 16 * 1024;
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// Current record schema version. Additive fields do not bump this.
pub const UPLOAD_RECORD_VERSION: u32 = 1;

/// One accepted upload event. See module docs for the consumer contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadRecord {
    /// Schema version ([`UPLOAD_RECORD_VERSION`]).
    pub version: u32,
    /// ULID — unique per accepted upload, time-sortable. Also the key suffix.
    pub event_id: String,
    /// Signed Blossom authorization event id that makes retry identity stable.
    #[serde(default)]
    pub request_event_id: String,
    /// Content hash of the uploaded bytes (64 lowercase hex chars).
    pub sha256: String,
    /// Canonical extension — consumers derive the blob key `{sha256}.{ext}`.
    pub ext: String,
    /// Sniffed MIME type of the uploaded bytes.
    pub mime_type: String,
    /// Size of the uploaded bytes. Zero denotes the canonical empty
    /// octet-stream attachment.
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

/// Immutable moderation projection staged before publication commit.
///
/// The plan lives outside the consumer-triggering `_uploads/` prefix. Its
/// digest and projection key can be committed in the publication transaction,
/// allowing a recovery worker to resume exact bytes after a process crash.
#[derive(Clone)]
pub struct StagedUploadRecord {
    record: UploadRecord,
    plan_key: String,
    projection_key: String,
    canonical_bytes: Vec<u8>,
    digest: [u8; 32],
}

/// Immutable first-result index for one signed upload identity.
#[derive(Debug, Serialize, Deserialize)]
struct UploadRecordReplayIndex {
    version: u32,
    community_id: String,
    request_event_id: String,
    uploader_id: String,
    event_id: String,
    sha256: String,
    plan_digest: String,
}

impl fmt::Debug for StagedUploadRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StagedUploadRecord([REDACTED])")
    }
}

impl StagedUploadRecord {
    /// Digest of the exact canonical record bytes persisted before commit.
    pub const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// Content-addressed recovery-plan key safe to persist transactionally.
    pub fn plan_key(&self) -> &str {
        &self.plan_key
    }

    /// Final consumer-triggering projection key.
    pub fn projection_key(&self) -> &str {
        &self.projection_key
    }

    /// Canonical bytes whose digest is returned by [`Self::digest`].
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    /// Validated record facts carried by this plan.
    pub const fn record(&self) -> &UploadRecord {
        &self.record
    }

    /// Publish exact bytes after the canonical DB commit.
    ///
    /// Repeating this operation is an exact-byte no-op. An existing object
    /// with different bytes fails closed instead of being overwritten.
    pub async fn publish_exact(
        &self,
        storage: &crate::storage::MediaStorage,
    ) -> Result<UploadProjectionResult, crate::error::MediaError> {
        let recovered = load_upload_record_plan(storage, self.digest).await?;
        if recovered.projection_key != self.projection_key
            || recovered.canonical_bytes != self.canonical_bytes
        {
            return Err(invalid_record(
                "staged upload record changed before projection",
            ));
        }
        put_exact_object(
            storage,
            &self.projection_key,
            &self.canonical_bytes,
            "application/json",
        )
        .await
    }
}

/// Outcome of an idempotent moderation projection attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadProjectionResult {
    /// Exact bytes were written and verified.
    Created,
    /// Exact bytes already existed at the deterministic projection key.
    Deduplicated,
}

/// Stage deterministic record bytes outside the consumer-triggering prefix.
pub async fn stage_upload_record(
    storage: &crate::storage::MediaStorage,
    ctx: &TenantContext,
    uploader: &nostr::PublicKey,
    request_event_id: &str,
    attribution: &UploadAttribution,
    facts: UploadEventFacts<'_>,
) -> Result<StagedUploadRecord, crate::error::MediaError> {
    let staged = build_staged_upload_record(ctx, uploader, request_event_id, attribution, facts)?;
    put_exact_object(
        storage,
        &staged.plan_key,
        &staged.canonical_bytes,
        "application/json",
    )
    .await?;

    // Mutable observation fields (host alias, profile label, source address)
    // remain in the first audit record, but never select a new canonical
    // result for the same signed event. The stable replay index is immutable;
    // concurrent retries load and validate the first winner.
    let replay_index = UploadRecordReplayIndex {
        version: UPLOAD_RECORD_VERSION,
        community_id: staged.record.community_id.clone(),
        request_event_id: staged.record.request_event_id.clone(),
        uploader_id: staged.record.uploader_id.clone(),
        event_id: staged.record.event_id.clone(),
        sha256: staged.record.sha256.clone(),
        plan_digest: hex::encode(staged.digest),
    };
    let replay_bytes = serde_json::to_vec(&replay_index)?;
    let persisted = storage
        .put_immutable_first(
            &upload_record_replay_key(ctx, &staged.record.event_id),
            &replay_bytes,
            "application/json",
        )
        .await?;
    if persisted.is_empty() || persisted.len() as u64 > MAX_UPLOAD_RECORD_BYTES {
        return Err(invalid_record("upload replay index exceeds its bound"));
    }
    let winner: UploadRecordReplayIndex = serde_json::from_slice(&persisted)?;
    if serde_json::to_vec(&winner)? != persisted
        || winner.version != UPLOAD_RECORD_VERSION
        || winner.community_id != staged.record.community_id
        || winner.request_event_id != staged.record.request_event_id
        || winner.uploader_id != staged.record.uploader_id
        || winner.event_id != staged.record.event_id
        || winner.sha256 != staged.record.sha256
        || !is_lower_hex_32(&winner.plan_digest)
    {
        return Err(invalid_record("upload replay index identity mismatch"));
    }
    let digest: [u8; 32] = hex::decode(&winner.plan_digest)
        .map_err(|_| invalid_record("upload replay index digest is malformed"))?
        .try_into()
        .map_err(|_| invalid_record("upload replay index digest has wrong length"))?;
    let selected = load_upload_record_plan(storage, digest).await?;
    select_persisted_replay(staged, selected)
}

/// Recover a staged moderation projection from its committed digest.
pub async fn load_upload_record_plan(
    storage: &crate::storage::MediaStorage,
    digest: [u8; 32],
) -> Result<StagedUploadRecord, crate::error::MediaError> {
    if digest == [0; 32] {
        return Err(invalid_record("upload record plan digest is unallocated"));
    }
    let plan_key = upload_record_plan_key(digest);
    let canonical_bytes = read_bounded_object(storage, &plan_key).await?;
    let actual: [u8; 32] = Sha256::digest(&canonical_bytes).into();
    if actual != digest {
        return Err(invalid_record("upload record plan digest mismatch"));
    }
    let record: UploadRecord = serde_json::from_slice(&canonical_bytes)?;
    validate_record(&record)?;
    if serde_json::to_vec(&record)? != canonical_bytes {
        return Err(invalid_record("upload record plan is not canonical"));
    }
    let projection_key = format!(
        "_uploads/{}/{}/{}.json",
        record.community_id, record.sha256, record.event_id
    );
    Ok(StagedUploadRecord {
        record,
        plan_key,
        projection_key,
        canonical_bytes,
        digest,
    })
}

fn build_staged_upload_record(
    ctx: &TenantContext,
    uploader: &nostr::PublicKey,
    request_event_id: &str,
    attribution: &UploadAttribution,
    facts: UploadEventFacts<'_>,
) -> Result<StagedUploadRecord, crate::error::MediaError> {
    use nostr::ToBech32;

    if !is_lower_hex_32(request_event_id)
        || !is_lower_hex_32(facts.sha256)
        || facts.ext.is_empty()
        || facts.ext.len() > 8
        || !facts
            .ext
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || facts.mime.is_empty()
        || facts.mime.len() > 255
        || !facts.mime.contains('/')
        || (facts.size == 0
            && (facts.sha256 != EMPTY_SHA256
                || facts.ext != "bin"
                || facts.mime != "application/octet-stream"))
        || facts.uploaded_at <= 0
    {
        return Err(invalid_record("invalid deterministic upload record facts"));
    }
    let uploader_id = uploader.to_hex();
    let event_id = deterministic_event_id(ctx, &uploader_id, request_event_id, facts)?;
    let ip = attribution.net.ip;
    let record = UploadRecord {
        version: UPLOAD_RECORD_VERSION,
        event_id,
        request_event_id: request_event_id.to_owned(),
        sha256: facts.sha256.to_owned(),
        ext: facts.ext.to_owned(),
        mime_type: facts.mime.to_owned(),
        size: facts.size,
        uploaded_at: facts.uploaded_at,
        community_id: ctx.community().to_string(),
        community_host: ctx.host().to_string(),
        uploader_id,
        uploader_npub: uploader
            .to_bech32()
            .map_err(|error| invalid_record(&format!("npub encoding failed: {error}")))?,
        uploader_name: attribution.uploader_name.clone(),
        ip: ip.map(|address| address.to_string()),
        port: ip.and(attribution.net.port),
    };
    validate_record(&record)?;
    let canonical_bytes = serde_json::to_vec(&record)?;
    let digest: [u8; 32] = Sha256::digest(&canonical_bytes).into();
    let plan_key = upload_record_plan_key(digest);
    let projection_key = upload_record_key(ctx, facts.sha256, &record.event_id);
    Ok(StagedUploadRecord {
        record,
        plan_key,
        projection_key,
        canonical_bytes,
        digest,
    })
}

/// Legacy immediate writer for transports without protected publication.
///
/// Protected transports must use [`stage_upload_record`] and commit its exact
/// plan before invoking [`StagedUploadRecord::publish_exact`].
pub async fn record_upload_event(
    storage: &crate::storage::MediaStorage,
    ctx: &TenantContext,
    uploader: &nostr::PublicKey,
    attribution: &UploadAttribution,
    facts: UploadEventFacts<'_>,
) -> Result<(), crate::error::MediaError> {
    use nostr::ToBech32;

    let event_id = ulid::Ulid::new().to_string();
    let request_event_id = hex::encode(Sha256::digest(event_id.as_bytes()));
    // Ports are only meaningful next to the address they were observed with.
    let ip = attribution.net.ip;
    let port = ip.and(attribution.net.port);
    let record = UploadRecord {
        version: UPLOAD_RECORD_VERSION,
        event_id: event_id.clone(),
        request_event_id,
        sha256: facts.sha256.to_string(),
        ext: facts.ext.to_string(),
        mime_type: facts.mime.to_string(),
        size: facts.size,
        uploaded_at: facts.uploaded_at,
        community_id: ctx.community().to_string(),
        community_host: ctx.host().to_string(),
        uploader_id: uploader.to_hex(),
        uploader_npub: uploader.to_bech32().map_err(|e| {
            // Unreachable for a valid pubkey; surfaced rather than unwrapped.
            crate::error::MediaError::StorageError(format!("npub encoding failed: {e}"))
        })?,
        uploader_name: attribution.uploader_name.clone(),
        ip: ip.map(|addr| addr.to_string()),
        port,
    };
    let key = upload_record_key(ctx, facts.sha256, &event_id);
    let json = serde_json::to_vec(&record)?;
    storage.put(&key, &json, "application/json").await
}

/// Build the per-event record key:
/// `_uploads/{community}/{sha256}/{event_id}.json`.
///
/// `ctx` must be the server-resolved request tenant — same fence as the
/// `_meta/` sidecar key (see [`crate::storage::MediaStorage::sidecar_key`]).
pub fn upload_record_key(ctx: &TenantContext, sha256: &str, event_id: &str) -> String {
    format!("_uploads/{}/{sha256}/{event_id}.json", ctx.community())
}

/// Content-addressed recovery-plan key for canonical moderation bytes.
pub fn upload_record_plan_key(digest: [u8; 32]) -> String {
    format!("{UPLOAD_RECORD_PLAN_PREFIX}/{}.json", hex::encode(digest))
}

fn upload_record_replay_key(ctx: &TenantContext, event_id: &str) -> String {
    format!(
        "{UPLOAD_RECORD_REPLAY_PREFIX}/{}/{}.json",
        ctx.community(),
        event_id
    )
}

fn ensure_same_replay_identity(
    candidate: &StagedUploadRecord,
    selected: &StagedUploadRecord,
) -> Result<(), crate::error::MediaError> {
    let candidate = &candidate.record;
    let selected = &selected.record;
    if candidate.version != selected.version
        || candidate.event_id != selected.event_id
        || candidate.request_event_id != selected.request_event_id
        || candidate.sha256 != selected.sha256
        || candidate.ext != selected.ext
        || candidate.mime_type != selected.mime_type
        || candidate.size != selected.size
        || candidate.uploaded_at != selected.uploaded_at
        || candidate.community_id != selected.community_id
        || candidate.uploader_id != selected.uploader_id
        || candidate.uploader_npub != selected.uploader_npub
    {
        return Err(invalid_record(
            "persisted upload replay result does not match signed identity",
        ));
    }
    Ok(())
}

fn select_persisted_replay(
    candidate: StagedUploadRecord,
    selected: StagedUploadRecord,
) -> Result<StagedUploadRecord, crate::error::MediaError> {
    ensure_same_replay_identity(&candidate, &selected)?;
    Ok(selected)
}

fn deterministic_event_id(
    ctx: &TenantContext,
    uploader_id: &str,
    request_event_id: &str,
    facts: UploadEventFacts<'_>,
) -> Result<String, crate::error::MediaError> {
    let seconds = u64::try_from(facts.uploaded_at)
        .map_err(|_| invalid_record("upload timestamp is negative"))?;
    let millis = seconds
        .checked_mul(1_000)
        .ok_or_else(|| invalid_record("upload timestamp overflows milliseconds"))?;
    if millis > 0x0000_ffff_ffff_ffff {
        return Err(invalid_record("upload timestamp exceeds ULID range"));
    }

    let mut hasher = Sha256::new();
    digest_field(&mut hasher, b"buzz-moderation-projection-v2");
    digest_field(&mut hasher, ctx.community().to_string().as_bytes());
    digest_field(&mut hasher, uploader_id.as_bytes());
    digest_field(&mut hasher, request_event_id.as_bytes());
    digest_field(&mut hasher, facts.sha256.as_bytes());
    digest_field(&mut hasher, facts.ext.as_bytes());
    digest_field(&mut hasher, facts.mime.as_bytes());
    digest_field(&mut hasher, &facts.size.to_be_bytes());
    digest_field(&mut hasher, &facts.uploaded_at.to_be_bytes());
    let identity = hasher.finalize();
    let mut ulid_bytes = [0_u8; 16];
    ulid_bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
    ulid_bytes[6..].copy_from_slice(&identity[..10]);
    Ok(ulid::Ulid::from(u128::from_be_bytes(ulid_bytes)).to_string())
}

fn digest_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn validate_record(record: &UploadRecord) -> Result<(), crate::error::MediaError> {
    if record.version != UPLOAD_RECORD_VERSION
        || record.event_id.parse::<ulid::Ulid>().is_err()
        || !is_lower_hex_32(&record.request_event_id)
        || !is_lower_hex_32(&record.sha256)
        || record.ext.is_empty()
        || record.ext.len() > 8
        || !record
            .ext
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        || record.mime_type.is_empty()
        || record.mime_type.len() > 255
        || !record.mime_type.contains('/')
        || (record.size == 0
            && (record.sha256 != EMPTY_SHA256
                || record.ext != "bin"
                || record.mime_type != "application/octet-stream"))
        || record.uploaded_at <= 0
        || uuid::Uuid::parse_str(&record.community_id).is_err()
        || record.community_host.is_empty()
        || record.community_host != record.community_host.trim()
        || !is_lower_hex_32(&record.uploader_id)
        || record
            .uploader_name
            .as_deref()
            .is_some_and(|value| value.len() > 256 || value.chars().any(char::is_control))
        || record
            .ip
            .as_deref()
            .is_some_and(|value| parse_public_ip(value).is_none())
        || (record.port.is_some() && record.ip.is_none())
    {
        return Err(invalid_record("invalid canonical upload record"));
    }
    Ok(())
}

fn is_lower_hex_32(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

async fn put_exact_object(
    storage: &crate::storage::MediaStorage,
    key: &str,
    expected: &[u8],
    content_type: &str,
) -> Result<UploadProjectionResult, crate::error::MediaError> {
    if storage
        .put_immutable_exact(key, expected, content_type)
        .await?
    {
        Ok(UploadProjectionResult::Created)
    } else {
        Ok(UploadProjectionResult::Deduplicated)
    }
}

async fn read_bounded_object(
    storage: &crate::storage::MediaStorage,
    key: &str,
) -> Result<Vec<u8>, crate::error::MediaError> {
    let size = storage
        .head_with_metadata(key)
        .await?
        .ok_or(crate::error::MediaError::NotFound)?
        .size;
    if size == 0 || size > MAX_UPLOAD_RECORD_BYTES {
        return Err(invalid_record("upload record object has invalid size"));
    }
    let mut stream = storage.get_stream(key).await?;
    let mut bytes = Vec::with_capacity(size as usize);
    let mut total = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total =
            total
                .checked_add(u64::try_from(chunk.len()).map_err(|_| {
                    invalid_record("upload record chunk length exceeds supported size")
                })?)
                .ok_or_else(|| invalid_record("upload record size overflow"))?;
        if total > MAX_UPLOAD_RECORD_BYTES {
            return Err(invalid_record("upload record object exceeds maximum size"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if total != size {
        return Err(invalid_record(
            "upload record object size changed during read",
        ));
    }
    Ok(bytes)
}

fn invalid_record(message: &str) -> crate::error::MediaError {
    crate::error::MediaError::StorageError(message.to_owned())
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
    fn record_serializes_full_shape() {
        let record = UploadRecord {
            version: UPLOAD_RECORD_VERSION,
            event_id: "01J9W3TEST".into(),
            request_event_id: "d".repeat(64),
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
            request_event_id: "d".repeat(64),
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
    fn deterministic_plan_replay_is_byte_identical_and_domain_bound() {
        let keys = nostr::Keys::generate();
        let attribution = UploadAttribution::default();
        let operation_id = "d".repeat(64);
        let sha256 = "a".repeat(64);
        let facts = UploadEventFacts {
            sha256: &sha256,
            ext: "png",
            mime: "image/png",
            size: 42,
            uploaded_at: 1_783_358_352,
        };
        let one = build_staged_upload_record(
            &tenant(),
            &keys.public_key(),
            &operation_id,
            &attribution,
            facts,
        )
        .expect("first plan");
        let replay = build_staged_upload_record(
            &tenant(),
            &keys.public_key(),
            &operation_id,
            &attribution,
            facts,
        )
        .expect("replay plan");
        let other_tenant = TenantContext::resolved(
            CommunityId::from_uuid(uuid::Uuid::from_u128(8)),
            "other.example.com",
        );
        let other = build_staged_upload_record(
            &other_tenant,
            &keys.public_key(),
            &operation_id,
            &attribution,
            facts,
        )
        .expect("other domain plan");

        assert_eq!(one.digest(), replay.digest());
        assert_eq!(one.plan_key(), replay.plan_key());
        assert_eq!(one.projection_key(), replay.projection_key());
        assert_eq!(one.canonical_bytes(), replay.canonical_bytes());
        assert_ne!(one.digest(), other.digest());
        assert_ne!(one.projection_key(), other.projection_key());
        assert_eq!(format!("{one:?}"), "StagedUploadRecord([REDACTED])");
    }

    #[test]
    fn deterministic_plan_preserves_zero_byte_upload_attribution() {
        let keys = nostr::Keys::generate();
        let sha256 = hex::encode(Sha256::digest([]));
        let facts = UploadEventFacts {
            sha256: &sha256,
            ext: "bin",
            mime: "application/octet-stream",
            size: 0,
            uploaded_at: 1_783_358_352,
        };

        let staged = build_staged_upload_record(
            &tenant(),
            &keys.public_key(),
            &"d".repeat(64),
            &UploadAttribution::default(),
            facts,
        )
        .expect("zero-byte attribution plan");

        assert_eq!(staged.record().size, 0);
        validate_record(staged.record()).expect("canonical zero-byte record");

        let wrong_sha = "a".repeat(64);
        let false_empty = UploadEventFacts {
            sha256: &wrong_sha,
            ..facts
        };
        assert!(build_staged_upload_record(
            &tenant(),
            &keys.public_key(),
            &"d".repeat(64),
            &UploadAttribution::default(),
            false_empty,
        )
        .is_err());
    }

    #[test]
    fn deterministic_plan_identity_excludes_retry_variant_attribution() {
        let keys = nostr::Keys::generate();
        let sha256 = "a".repeat(64);
        let facts = UploadEventFacts {
            sha256: &sha256,
            ext: "png",
            mime: "image/png",
            size: 42,
            uploaded_at: 1_783_358_352,
        };
        let plain = build_staged_upload_record(
            &tenant(),
            &keys.public_key(),
            &"d".repeat(64),
            &UploadAttribution::default(),
            facts,
        )
        .expect("plain plan");
        let named = build_staged_upload_record(
            &tenant(),
            &keys.public_key(),
            &"d".repeat(64),
            &UploadAttribution {
                uploader_name: Some("alice".into()),
                net: UploadNetworkInfo::default(),
            },
            facts,
        )
        .expect("named plan");
        let other_operation = build_staged_upload_record(
            &tenant(),
            &keys.public_key(),
            &"e".repeat(64),
            &UploadAttribution::default(),
            facts,
        )
        .expect("other operation plan");

        assert_ne!(plain.digest(), named.digest());
        assert_ne!(plain.digest(), other_operation.digest());
        assert_eq!(plain.record().event_id, named.record().event_id);
        assert_eq!(plain.projection_key(), named.projection_key());
        assert_ne!(plain.record().event_id, other_operation.record().event_id);
    }

    #[test]
    fn changed_profile_network_and_host_retry_selects_first_exact_result() {
        let keys = nostr::Keys::generate();
        let operation_id = "d".repeat(64);
        let sha256 = "a".repeat(64);
        let facts = UploadEventFacts {
            sha256: &sha256,
            ext: "png",
            mime: "image/png",
            size: 42,
            uploaded_at: 1_783_358_352,
        };
        let first = build_staged_upload_record(
            &tenant(),
            &keys.public_key(),
            &operation_id,
            &UploadAttribution {
                uploader_name: Some("first-name".into()),
                net: UploadNetworkInfo {
                    ip: Some("8.8.8.8".parse().expect("public test IP")),
                    port: Some(443),
                },
            },
            facts,
        )
        .expect("first result");
        let alias_tenant = TenantContext::resolved(tenant().community(), "alias.example.com");
        let retry = build_staged_upload_record(
            &alias_tenant,
            &keys.public_key(),
            &operation_id,
            &UploadAttribution {
                uploader_name: Some("changed-name".into()),
                net: UploadNetworkInfo {
                    ip: Some("1.1.1.1".parse().expect("public test IP")),
                    port: Some(8443),
                },
            },
            facts,
        )
        .expect("retry candidate");

        assert_eq!(first.record().event_id, retry.record().event_id);
        assert_eq!(first.projection_key(), retry.projection_key());
        assert_ne!(first.digest(), retry.digest());
        let first_digest = first.digest();
        let first_bytes = first.canonical_bytes().to_vec();
        let selected = select_persisted_replay(retry, first).expect("first result wins");
        assert_eq!(selected.digest(), first_digest);
        assert_eq!(selected.canonical_bytes(), first_bytes);
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
