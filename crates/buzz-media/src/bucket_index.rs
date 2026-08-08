//! Bucket key taxonomy classifier and pure aggregation fold for the S3-truth
//! storage sweep.
//!
//! This module has **zero S3 I/O** — [`classify_key`] and [`BucketAggregate`]
//! operate on plain `(key, size)` pairs, and [`fold_bucket_listing`] takes a
//! caller-supplied page-fetching closure so the pagination/cap logic is
//! testable against synthetic listings. The relay wires a real
//! [`crate::storage::MediaStorage::list_page`] closure at the call site (see
//! `buzz-relay`'s storage sweep task).
//!
//! Five key classes (thumb matched first, everything unrecognized is
//! `Unknown` — never silently folded into another class):
//!
//! | Class | Shape |
//! |---|---|
//! | thumb | `{sha256}.thumb.jpg` or `media/{hh}/{hh}/{community-uuid}/{sha256}.thumb.jpg` |
//! | blob | `{sha256}.{ext}` or `media/{hh}/{hh}/{community-uuid}/{sha256}.{ext}` (ext: 1-8 mixed-case alphanumeric) |
//! | sidecar | `_meta/{community-uuid}/{sha256}.json` |
//! | auxiliary | `_uploads/{community-uuid}/{sha256}/{ulid}.json` |
//! | unknown | everything else |

use std::collections::HashMap;
use std::future::Future;

use uuid::Uuid;

use crate::error::MediaError;

/// The classification of one bucket key. `Unknown` is the deliberate
/// catch-all — a malformed variant of a known prefix (e.g. a truncated
/// `_uploads/` key) falls to `Unknown` rather than being coerced into
/// `Auxiliary`, so visibility gauges stay loud instead of silently wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyClass {
    /// Legacy or sharded thumbnail, attributed to the blob's sha.
    Thumb {
        community: Option<Uuid>,
        sha256: String,
    },
    /// Legacy or sharded blob; sharded keys carry direct community attribution.
    Blob {
        community: Option<Uuid>,
        sha256: String,
        ext: String,
    },
    /// `_meta/{community}/{sha256}.json` — the (community, sha) binding.
    Sidecar { community: Uuid, sha256: String },
    /// `_uploads/{community}/{sha256}/{event_id}.json` — fleet physical only.
    Auxiliary {
        community: Uuid,
        sha256: String,
        event_id: String,
    },
    /// Anything that doesn't match one of the four strict shapes above.
    Unknown,
}

/// Classify one bucket key. Matches `thumb` first (its suffix is a superset
/// shape of the blob pattern's segment count), then blob, sidecar,
/// auxiliary, and finally unknown. See module docs for the exact shapes.
pub fn classify_key(key: &str) -> KeyClass {
    if let Some((community, filename)) = parse_sharded_prefix(key) {
        if let Some(parsed_sha) = parse_thumb_key(filename) {
            return KeyClass::Thumb {
                community: Some(community),
                sha256: parsed_sha,
            };
        }
        if let Some((parsed_sha, ext)) = parse_blob_key(filename) {
            return KeyClass::Blob {
                community: Some(community),
                sha256: parsed_sha,
                ext,
            };
        }
        return KeyClass::Unknown;
    }
    if let Some(sha256) = parse_thumb_key(key) {
        return KeyClass::Thumb {
            community: None,
            sha256,
        };
    }
    if let Some((sha256, ext)) = parse_blob_key(key) {
        return KeyClass::Blob {
            community: None,
            sha256,
            ext,
        };
    }
    if let Some((community, sha256)) = parse_sidecar_key(key) {
        return KeyClass::Sidecar { community, sha256 };
    }
    if let Some((community, sha256, event_id)) = parse_auxiliary_key(key) {
        return KeyClass::Auxiliary {
            community,
            sha256,
            event_id,
        };
    }
    KeyClass::Unknown
}

/// A 64-char lowercase-hex SHA-256 digest, strictly.
fn is_sha256(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Blob extension charset: 1-8 mixed-case alphanumeric chars (F4-bis — infer
/// 0.19 emits uppercase `Z` for `application/x-compress`, which is
/// legitimate writer output that must classify as a blob, not unknown).
fn is_blob_ext(s: &str) -> bool {
    !s.is_empty() && s.len() <= 8 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Strict Crockford-base32 ULID charset check, uppercase only (matches the
/// ulid crate's `Display` output — see `upload_record.rs`'s writer). Not
/// `ulid::Ulid::from_string`, which is deliberately case-insensitive on
/// decode and would accept lowercase variants the writer never produces —
/// looser than the plan's anchored regex.
fn is_ulid_charset(s: &str) -> bool {
    s.len() == 26
        && s.bytes().all(|b| {
            b.is_ascii_digit()
                || (b'A'..=b'H').contains(&b)
                || b == b'J'
                || b == b'K'
                || b == b'M'
                || b == b'N'
                || (b'P'..=b'T').contains(&b)
                || (b'V'..=b'Z').contains(&b)
        })
}

/// Strict canonical UUID: exactly 36 chars, lowercase hex + hyphens at
/// positions 8/13/18/23. Rejects the braced/urn/no-hyphen forms
/// `Uuid::parse_str` alone would otherwise accept — every UUID this server
/// writes into a key is `Display`-formatted canonically lowercase, so
/// anything else is not a UUID we wrote and must not silently parse as one.
fn parse_canonical_uuid(s: &str) -> Option<Uuid> {
    if s.len() != 36 {
        return None;
    }
    for (i, b) in s.bytes().enumerate() {
        let ok = match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        };
        if !ok {
            return None;
        }
    }
    Uuid::parse_str(s).ok()
}

/// `media/{sha[0:2]}/{sha[2:4]}/{community}/{filename}`. The filename's digest
/// must agree with both shard segments; malformed migration keys stay unknown.
fn parse_sharded_prefix(key: &str) -> Option<(Uuid, &str)> {
    let mut segments = key.split('/');
    if segments.next()? != "media" {
        return None;
    }
    let shard_1 = segments.next()?;
    let shard_2 = segments.next()?;
    let community = parse_canonical_uuid(segments.next()?)?;
    let filename = segments.next()?;
    if segments.next().is_some() || shard_1.len() != 2 || shard_2.len() != 2 {
        return None;
    }
    let sha256 = filename.split('.').next()?;
    if !is_sha256(sha256) || shard_1 != &sha256[..2] || shard_2 != &sha256[2..4] {
        return None;
    }
    Some((community, filename))
}

/// `{sha256}.thumb.jpg`
fn parse_thumb_key(key: &str) -> Option<String> {
    let mut parts = key.split('.');
    let sha256 = parts.next()?;
    let thumb = parts.next()?;
    let jpg = parts.next()?;
    if parts.next().is_some() || thumb != "thumb" || jpg != "jpg" || !is_sha256(sha256) {
        return None;
    }
    Some(sha256.to_string())
}

/// `{sha256}.{ext}` — exactly two dot-separated segments.
fn parse_blob_key(key: &str) -> Option<(String, String)> {
    let mut parts = key.split('.');
    let sha256 = parts.next()?;
    let ext = parts.next()?;
    if parts.next().is_some() || !is_sha256(sha256) || !is_blob_ext(ext) {
        return None;
    }
    Some((sha256.to_string(), ext.to_string()))
}

/// `_meta/{community}/{sha256}.json`
fn parse_sidecar_key(key: &str) -> Option<(Uuid, String)> {
    let mut segments = key.split('/');
    if segments.next()? != "_meta" {
        return None;
    }
    let community = parse_canonical_uuid(segments.next()?)?;
    let last = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    let mut last_parts = last.split('.');
    let sha256 = last_parts.next()?;
    let json = last_parts.next()?;
    if last_parts.next().is_some() || json != "json" || !is_sha256(sha256) {
        return None;
    }
    Some((community, sha256.to_string()))
}

/// `_uploads/{community}/{sha256}/{event_id}.json`, `event_id` a ULID.
fn parse_auxiliary_key(key: &str) -> Option<(Uuid, String, String)> {
    let mut segments = key.split('/');
    if segments.next()? != "_uploads" {
        return None;
    }
    let community = parse_canonical_uuid(segments.next()?)?;
    let sha256 = segments.next()?;
    if !is_sha256(sha256) {
        return None;
    }
    let last = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    let mut last_parts = last.split('.');
    let event_id = last_parts.next()?;
    let json = last_parts.next()?;
    if last_parts.next().is_some() || json != "json" || !is_ulid_charset(event_id) {
        return None;
    }
    Some((community, sha256.to_string(), event_id.to_string()))
}

/// Per-community logical storage: bytes and object count of bound shas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommunityStorage {
    pub bytes: u64,
    pub objects: u64,
}

/// The full computed sweep result: fleet physical/logical totals,
/// per-community logical breakdown, and anomaly/visibility gauges. Pure
/// data — no I/O, cheap to clone into a cached snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BucketSnapshot {
    /// Every listed object, every class (kind=physical).
    pub physical_bytes: u64,
    pub physical_objects: u64,
    /// Sum of per-community logical bytes/objects (kind=logical). Bills a
    /// blob bound to N communities N times — intentional (D-EXT/D-COUNT).
    pub logical_bytes: u64,
    pub logical_objects: u64,
    pub per_community: HashMap<Uuid, CommunityStorage>,
    /// Blob shas with zero sidecar binding in any community.
    pub orphan_blob_bytes: u64,
    pub orphan_blob_count: u64,
    /// Sidecar bindings whose sha has no blob bytes at all.
    pub orphan_sidecar_count: u64,
    /// Shas with more than one blob variant (anomaly — see plan D-EXT).
    pub multi_variant_shas: u64,
    /// Total bytes of ALL blob variants belonging to anomalous shas.
    pub multi_variant_bytes: u64,
    /// Logical blob variants present in both legacy and sharded layouts.
    pub duplicate_layout_variants: u64,
    /// Physical bytes across both layouts for duplicate logical variants.
    pub duplicate_layout_bytes: u64,
    pub unknown_key_bytes: u64,
    pub unknown_key_objects: u64,
}

/// Pure, incremental fold over classified bucket keys. Never retains a full
/// object listing — only per-sha/per-binding running totals, bounded by the
/// number of distinct shas and sidecar bindings actually present.
#[derive(Debug, Default)]
struct LayoutCopies {
    legacy: Option<u64>,
    sharded: HashMap<Uuid, u64>,
}

impl LayoutCopies {
    fn insert(&mut self, community: Option<Uuid>, size: u64) {
        match community {
            Some(community) => {
                self.sharded.insert(community, size);
            }
            None => {
                self.legacy = Some(size);
            }
        }
    }

    fn physical_bytes(&self) -> u64 {
        self.legacy.unwrap_or(0) + self.sharded.values().sum::<u64>()
    }

    fn logical_bytes(&self, community: Uuid) -> u64 {
        self.sharded
            .get(&community)
            .copied()
            .or(self.legacy)
            .unwrap_or(0)
    }
}

#[derive(Debug, Default)]
pub struct BucketAggregate {
    /// (sha, ext) -> physical copies by layout. Layout copies are one logical
    /// variant and must not double bill during migration.
    blob_variants: HashMap<(String, String), LayoutCopies>,
    /// sha -> physical thumbnail copies by layout.
    thumb_copies: HashMap<String, LayoutCopies>,
    /// (community, sha) -> sidecar object's own byte size (informational;
    /// not part of logical bytes).
    sidecar_bindings: HashMap<(Uuid, String), u64>,
    physical_bytes: u64,
    physical_objects: u64,
    unknown_bytes: u64,
    unknown_objects: u64,
}

impl BucketAggregate {
    /// Fold one classified `(key, size)` pair into the running aggregate.
    pub fn fold(&mut self, key: &str, size: u64) {
        self.physical_objects += 1;
        self.physical_bytes += size;
        match classify_key(key) {
            KeyClass::Thumb { community, sha256 } => {
                self.thumb_copies
                    .entry(sha256)
                    .or_default()
                    .insert(community, size);
            }
            KeyClass::Blob {
                community,
                sha256,
                ext,
            } => {
                self.blob_variants
                    .entry((sha256, ext))
                    .or_default()
                    .insert(community, size);
            }
            KeyClass::Sidecar { community, sha256 } => {
                self.sidecar_bindings.insert((community, sha256), size);
            }
            KeyClass::Auxiliary { .. } => {
                // Fleet physical only — never enters logical/orphan math (F4).
            }
            KeyClass::Unknown => {
                self.unknown_objects += 1;
                self.unknown_bytes += size;
            }
        }
    }

    /// Compute the final snapshot from everything folded so far.
    pub fn finish(self) -> BucketSnapshot {
        let bound_shas: std::collections::HashSet<&str> = self
            .sidecar_bindings
            .keys()
            .map(|(_, sha256)| sha256.as_str())
            .collect();

        let mut multi_variant_shas = 0u64;
        let mut multi_variant_bytes = 0u64;
        let mut orphan_blob_count = 0u64;
        let mut orphan_blob_bytes = 0u64;
        let mut duplicate_layout_variants = 0u64;
        let mut duplicate_layout_bytes = 0u64;
        for copies in self.blob_variants.values() {
            if copies.legacy.is_some() && !copies.sharded.is_empty() {
                duplicate_layout_variants += 1;
                duplicate_layout_bytes += copies.physical_bytes();
            }
        }
        let mut variants_by_sha: HashMap<&str, Vec<&LayoutCopies>> = HashMap::new();
        for ((sha256, _), copies) in &self.blob_variants {
            variants_by_sha
                .entry(sha256.as_str())
                .or_default()
                .push(copies);
        }
        for (sha256, variants) in &variants_by_sha {
            let variant_bytes: u64 = variants.iter().map(|copies| copies.physical_bytes()).sum();
            if variants.len() > 1 {
                multi_variant_shas += 1;
                multi_variant_bytes += variant_bytes;
            }
            if !bound_shas.contains(*sha256) {
                orphan_blob_count += 1;
                orphan_blob_bytes += variant_bytes;
            }
        }

        let orphan_sidecar_count = self
            .sidecar_bindings
            .keys()
            .filter(|(_, sha256)| !variants_by_sha.contains_key(sha256.as_str()))
            .count() as u64;

        let mut per_community: HashMap<Uuid, CommunityStorage> = HashMap::new();
        for (community, sha256) in self.sidecar_bindings.keys() {
            let blob_bytes: u64 = variants_by_sha
                .get(sha256.as_str())
                .map(|variants| {
                    variants
                        .iter()
                        .map(|copies| copies.logical_bytes(*community))
                        .sum()
                })
                .unwrap_or(0);
            let thumb_bytes = self
                .thumb_copies
                .get(sha256)
                .map(|copies| copies.logical_bytes(*community))
                .unwrap_or(0);
            let entry = per_community.entry(*community).or_default();
            entry.bytes += blob_bytes + thumb_bytes;
            entry.objects += 1;
        }
        let logical_bytes = per_community.values().map(|c| c.bytes).sum();
        let logical_objects = per_community.values().map(|c| c.objects).sum();

        BucketSnapshot {
            physical_bytes: self.physical_bytes,
            physical_objects: self.physical_objects,
            logical_bytes,
            logical_objects,
            per_community,
            orphan_blob_bytes,
            orphan_blob_count,
            orphan_sidecar_count,
            multi_variant_shas,
            multi_variant_bytes,
            duplicate_layout_variants,
            duplicate_layout_bytes,
            unknown_key_bytes: self.unknown_bytes,
            unknown_key_objects: self.unknown_objects,
        }
    }
}

/// Failure modes for the paginated listing fold. All variants mean "failed
/// sweep, keep the old snapshot" to the caller — never a partial one.
#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    /// Cumulative listed-object count exceeded `cap` mid-listing.
    #[error("object cap exceeded: {seen} listed objects > cap {cap}")]
    CapExceeded { seen: u64, cap: u64 },
    /// The page source (S3, or a test double) failed.
    #[error("storage error during listing: {0}")]
    Storage(#[from] MediaError),
    /// The whole sweep (this fold plus any caller-side wrapping) exceeded its
    /// deadline. Constructed by the relay's sweep task, which wraps this
    /// entire function in `tokio::time::timeout` — kept here (not a
    /// relay-local error type) so `SweepError` stays the single failure
    /// currency the whole sweep pipeline reasons about.
    #[error("sweep timed out after {0:?}")]
    Timeout(std::time::Duration),
    /// A listing page reported `is_truncated=true` but supplied no
    /// continuation token — a malformed S3 response that cannot be resumed.
    #[error("truncated listing page with no continuation token")]
    MalformedPage,
}

/// One page of a bucket listing, decoupled from any S3 crate type so the
/// fold below can be driven by a synthetic closure in tests.
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub objects: Vec<(String, u64)>,
    pub next_continuation_token: Option<String>,
    pub is_truncated: bool,
}

/// Fold an entire paginated bucket listing, checking the object cap BEFORE
/// folding each page and never retaining the full listing — only the
/// bounded per-sha/per-binding aggregate state.
///
/// `fetch_page` is called with `None` for the first page and the previous
/// page's continuation token thereafter; production callers close over a
/// [`crate::storage::MediaStorage`], tests close over canned [`Page`]s.
pub async fn fold_bucket_listing<F, Fut>(
    cap: u64,
    mut fetch_page: F,
) -> Result<BucketSnapshot, SweepError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<Page, MediaError>>,
{
    let mut aggregate = BucketAggregate::default();
    let mut continuation_token = None;
    let mut seen: u64 = 0;

    loop {
        let page = fetch_page(continuation_token.take()).await?;

        seen += page.objects.len() as u64;
        if seen > cap {
            return Err(SweepError::CapExceeded { seen, cap });
        }

        for (key, size) in &page.objects {
            aggregate.fold(key, *size);
        }

        if !page.is_truncated {
            break;
        }
        match page.next_continuation_token {
            Some(token) => continuation_token = Some(token),
            None => return Err(SweepError::MalformedPage),
        }
    }

    Ok(aggregate.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(byte: u8) -> String {
        hex::encode([byte; 32])
    }

    fn community(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    // --- classify_key ---

    #[test]
    fn classifies_thumb_key() {
        let s = sha(0xaa);
        assert_eq!(
            classify_key(&format!("{s}.thumb.jpg")),
            KeyClass::Thumb {
                community: None,
                sha256: s,
            }
        );
    }

    #[test]
    fn classifies_blob_key_lowercase_ext() {
        let s = sha(0xbb);
        assert_eq!(
            classify_key(&format!("{s}.png")),
            KeyClass::Blob {
                community: None,
                sha256: s,
                ext: "png".to_string()
            }
        );
    }

    /// F4-bis: infer 0.19 emits uppercase `Z` for `application/x-compress`,
    /// legitimate writer output — must classify as blob, not unknown.
    #[test]
    fn classifies_blob_key_uppercase_z_extension() {
        let s = sha(0xcc);
        assert_eq!(
            classify_key(&format!("{s}.Z")),
            KeyClass::Blob {
                community: None,
                sha256: s,
                ext: "Z".to_string()
            }
        );
    }

    #[test]
    fn classifies_sharded_blob_and_thumb_keys_with_community() {
        let s = sha(0xab);
        let c = community(10);
        assert_eq!(
            classify_key(&format!("media/ab/ab/{c}/{s}.png")),
            KeyClass::Blob {
                community: Some(c),
                sha256: s.clone(),
                ext: "png".to_string(),
            }
        );
        assert_eq!(
            classify_key(&format!("media/ab/ab/{c}/{s}.thumb.jpg")),
            KeyClass::Thumb {
                community: Some(c),
                sha256: s,
            }
        );
    }

    #[test]
    fn malformed_sharded_keys_are_unknown() {
        let s = sha(0xab);
        let c = community(11);
        for key in [
            format!("media/ff/ab/{c}/{s}.png"),
            format!("media/ab/ff/{c}/{s}.png"),
            format!("media/a/ab/{c}/{s}.png"),
            format!("media/ab/ab/not-a-uuid/{s}.png"),
            format!("media/ab/ab/{c}/{s}.png/extra"),
            format!("media/ab/ab/{c}/{}.png", s.to_uppercase()),
            format!("media/ab/ab/{c}/{s}.tar.gz"),
        ] {
            assert_eq!(classify_key(&key), KeyClass::Unknown, "key: {key}");
        }
    }

    #[test]
    fn classifies_sidecar_key() {
        let s = sha(0xdd);
        let c = community(1);
        assert_eq!(
            classify_key(&format!("_meta/{c}/{s}.json")),
            KeyClass::Sidecar {
                community: c,
                sha256: s
            }
        );
    }

    #[test]
    fn classifies_auxiliary_key() {
        let s = sha(0xee);
        let c = community(2);
        let ulid = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        assert_eq!(
            classify_key(&format!("_uploads/{c}/{s}/{ulid}.json")),
            KeyClass::Auxiliary {
                community: c,
                sha256: s,
                event_id: ulid.to_string(),
            }
        );
    }

    #[test]
    fn malformed_uploads_key_is_unknown_not_auxiliary() {
        let s = sha(0xff);
        let c = community(3);
        // Lowercase ULID: the writer never emits this — must not silently
        // pass as auxiliary; visibility (unknown) beats a wrong guess.
        assert_eq!(
            classify_key(&format!("_uploads/{c}/{s}/01arz3ndektsv4rrffq69g5fav.json")),
            KeyClass::Unknown
        );
        // Wrong-length event id.
        assert_eq!(
            classify_key(&format!("_uploads/{c}/{s}/TOOSHORT.json")),
            KeyClass::Unknown
        );
    }

    #[test]
    fn malformed_sidecar_non_uuid_community_is_unknown() {
        let s = sha(0x11);
        assert_eq!(
            classify_key(&format!("_meta/not-a-uuid/{s}.json")),
            KeyClass::Unknown
        );
    }

    #[test]
    fn malformed_keys_fall_to_unknown() {
        let s = sha(0xab);
        // Uppercase hex in the sha segment.
        assert_eq!(
            classify_key(&format!("{}.png", s.to_uppercase())),
            KeyClass::Unknown
        );
        // Wrong sha length.
        assert_eq!(classify_key("abc123.png"), KeyClass::Unknown);
        // Extra segment.
        assert_eq!(classify_key(&format!("{s}.png.bak")), KeyClass::Unknown);
        // Extension too long (9 chars).
        assert_eq!(classify_key(&format!("{s}.123456789")), KeyClass::Unknown);
        // No extension at all.
        assert_eq!(classify_key(&s), KeyClass::Unknown);
        // Totally unrelated key.
        assert_eq!(classify_key("README.md"), KeyClass::Unknown);
    }

    // --- BucketAggregate / finish ---

    #[test]
    fn empty_listing_yields_zero_snapshot() {
        let snapshot = BucketAggregate::default().finish();
        assert_eq!(snapshot, BucketSnapshot::default());
    }

    #[test]
    fn multi_variant_sha_is_anomalous_and_bills_the_sum() {
        let s = sha(0x33);
        let c = community(4);
        let mut agg = BucketAggregate::default();
        agg.fold(&format!("{s}.jpg"), 100);
        agg.fold(&format!("{s}.png"), 200); // same sha, second variant
        agg.fold(&format!("_meta/{c}/{s}.json"), 10);

        let snap = agg.finish();
        assert_eq!(snap.multi_variant_shas, 1);
        assert_eq!(snap.multi_variant_bytes, 300);
        assert_eq!(snap.orphan_blob_count, 0);
        // Logical bytes bill the sum of both variants (D-EXT).
        assert_eq!(snap.per_community[&c].bytes, 300);
        assert_eq!(snap.per_community[&c].objects, 1);
    }

    #[test]
    fn dual_layout_copies_count_physically_but_dedupe_logical_usage() {
        let s = sha(0xab);
        let c = community(12);
        let mut agg = BucketAggregate::default();
        agg.fold(&format!("{s}.jpg"), 100);
        agg.fold(&format!("media/ab/ab/{c}/{s}.jpg"), 100);
        agg.fold(&format!("{s}.thumb.jpg"), 20);
        agg.fold(&format!("media/ab/ab/{c}/{s}.thumb.jpg"), 20);
        agg.fold(&format!("_meta/{c}/{s}.json"), 10);

        let snap = agg.finish();
        assert_eq!(snap.physical_objects, 5);
        assert_eq!(snap.physical_bytes, 250);
        assert_eq!(snap.logical_objects, 1);
        assert_eq!(snap.logical_bytes, 120);
        assert_eq!(snap.per_community[&c].bytes, 120);
        assert_eq!(snap.multi_variant_shas, 0);
        assert_eq!(snap.duplicate_layout_variants, 1);
        assert_eq!(snap.duplicate_layout_bytes, 200);
        assert_eq!(snap.unknown_key_objects, 0);
    }

    #[test]
    fn sharded_copy_is_attributed_only_to_its_community() {
        let s = sha(0xcd);
        let sharded_community = community(13);
        let other_community = community(14);
        let mut agg = BucketAggregate::default();
        agg.fold(&format!("media/cd/cd/{sharded_community}/{s}.jpg"), 200);
        agg.fold(&format!("_meta/{sharded_community}/{s}.json"), 10);
        agg.fold(&format!("_meta/{other_community}/{s}.json"), 10);

        let snap = agg.finish();
        assert_eq!(snap.per_community[&sharded_community].bytes, 200);
        assert_eq!(snap.per_community[&other_community].bytes, 0);
    }

    #[test]
    fn orphan_blob_has_no_sidecar_binding() {
        let s = sha(0x44);
        let mut agg = BucketAggregate::default();
        agg.fold(&format!("{s}.jpg"), 500);

        let snap = agg.finish();
        assert_eq!(snap.orphan_blob_count, 1);
        assert_eq!(snap.orphan_blob_bytes, 500);
        assert!(snap.per_community.is_empty());
        assert_eq!(snap.logical_bytes, 0);
    }

    #[test]
    fn orphan_sidecar_has_no_blob_bytes() {
        let s = sha(0x55);
        let c = community(5);
        let mut agg = BucketAggregate::default();
        agg.fold(&format!("_meta/{c}/{s}.json"), 20);

        let snap = agg.finish();
        assert_eq!(snap.orphan_sidecar_count, 1);
        // The binding still counts as a logical object per D-COUNT, but
        // contributes zero bytes since there's no blob to bill.
        assert_eq!(snap.per_community[&c].objects, 1);
        assert_eq!(snap.per_community[&c].bytes, 0);
    }

    #[test]
    fn unmapped_community_binding_still_aggregates_under_its_uuid() {
        // bucket_index has no DB access — "unmapped" (no matching community
        // row) is a join the caller performs against per_community's keys.
        // Here we only assert the raw UUID is preserved for that join.
        let s = sha(0x66);
        let c = community(6);
        let mut agg = BucketAggregate::default();
        agg.fold(&format!("{s}.jpg"), 40);
        agg.fold(&format!("_meta/{c}/{s}.json"), 5);

        let snap = agg.finish();
        assert!(snap.per_community.contains_key(&c));
    }

    #[test]
    fn thumb_bytes_attribute_to_the_blobs_sha() {
        let s = sha(0x77);
        let c = community(7);
        let mut agg = BucketAggregate::default();
        agg.fold(&format!("{s}.jpg"), 1000);
        agg.fold(&format!("{s}.thumb.jpg"), 50);
        agg.fold(&format!("_meta/{c}/{s}.json"), 5);

        let snap = agg.finish();
        assert_eq!(snap.per_community[&c].bytes, 1050);
        assert_eq!(snap.per_community[&c].objects, 1);
        // Fleet physical totals count every object separately.
        assert_eq!(snap.physical_objects, 3);
        assert_eq!(snap.physical_bytes, 1055);
    }

    #[test]
    fn unknown_keys_are_counted_but_excluded_from_logical_math() {
        let mut agg = BucketAggregate::default();
        agg.fold("garbage-key", 999);
        agg.fold("_meta/not-a-uuid/x.json", 1);

        let snap = agg.finish();
        assert_eq!(snap.unknown_key_objects, 2);
        assert_eq!(snap.unknown_key_bytes, 1000);
        assert_eq!(snap.physical_objects, 2);
        assert_eq!(snap.logical_bytes, 0);
        assert!(snap.per_community.is_empty());
    }

    #[test]
    fn auxiliary_keys_are_physical_only() {
        let s = sha(0x88);
        let c = community(8);
        let ulid = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let mut agg = BucketAggregate::default();
        agg.fold(&format!("_uploads/{c}/{s}/{ulid}.json"), 30);

        let snap = agg.finish();
        assert_eq!(snap.physical_objects, 1);
        assert_eq!(snap.physical_bytes, 30);
        assert_eq!(snap.logical_bytes, 0);
        assert!(snap.per_community.is_empty());
        assert_eq!(snap.unknown_key_objects, 0);
    }

    // --- fold_bucket_listing (pagination + cap) ---

    #[tokio::test]
    async fn empty_bucket_listing_yields_zero_snapshot() {
        let snapshot = fold_bucket_listing(100, |_token| async {
            Ok(Page {
                objects: vec![],
                next_continuation_token: None,
                is_truncated: false,
            })
        })
        .await
        .expect("empty listing must not fail");
        assert_eq!(snapshot, BucketSnapshot::default());
    }

    #[tokio::test]
    async fn pagination_follows_continuation_tokens_across_pages() {
        let s1 = sha(0x99);
        let s2 = sha(0xa1);
        let pages = std::sync::Arc::new(std::sync::Mutex::new(vec![
            Page {
                objects: vec![(format!("{s1}.jpg"), 10)],
                next_continuation_token: Some("page-2".to_string()),
                is_truncated: true,
            },
            Page {
                objects: vec![(format!("{s2}.jpg"), 20)],
                next_continuation_token: None,
                is_truncated: false,
            },
        ]));

        let snapshot = fold_bucket_listing(100, {
            let pages = std::sync::Arc::clone(&pages);
            move |token| {
                let pages = std::sync::Arc::clone(&pages);
                async move {
                    let mut pages = pages.lock().unwrap();
                    assert!(
                        (token.is_none() && pages.len() == 2)
                            || (token.as_deref() == Some("page-2") && pages.len() == 1)
                    );
                    Ok(pages.remove(0))
                }
            }
        })
        .await
        .expect("two-page listing must succeed");

        assert_eq!(snapshot.physical_objects, 2);
        assert_eq!(snapshot.physical_bytes, 30);
        assert_eq!(snapshot.orphan_blob_count, 2);
    }

    #[tokio::test]
    async fn cap_breach_mid_listing_fails_the_sweep_before_folding_the_page() {
        let objects: Vec<(String, u64)> = (0..5).map(|i| (format!("obj-{i}"), 1)).collect();
        let result = fold_bucket_listing(3, move |_token| {
            let objects = objects.clone();
            async move {
                Ok(Page {
                    objects,
                    next_continuation_token: None,
                    is_truncated: false,
                })
            }
        })
        .await;

        match result {
            Err(SweepError::CapExceeded { seen, cap }) => {
                assert_eq!(seen, 5);
                assert_eq!(cap, 3);
            }
            other => panic!("expected CapExceeded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn storage_error_propagates_from_page_source() {
        let result: Result<BucketSnapshot, SweepError> = fold_bucket_listing(10, |_token| async {
            Err(MediaError::StorageError("boom".to_string()))
        })
        .await;
        assert!(matches!(result, Err(SweepError::Storage(_))));
    }

    #[tokio::test]
    async fn truncated_page_with_no_continuation_token_fails_the_sweep() {
        let result = fold_bucket_listing(100, |_token| async {
            Ok(Page {
                objects: vec![("some-key".to_string(), 1)],
                next_continuation_token: None,
                is_truncated: true,
            })
        })
        .await;
        assert!(
            matches!(result, Err(SweepError::MalformedPage)),
            "truncated page without a continuation token must fail, not return partial data"
        );
    }
}
