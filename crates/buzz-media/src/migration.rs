//! Shared primitives for media-layout maintenance binaries.

use std::time::Duration;

use crate::{legacy_blob_key, legacy_thumb_key, sharded_blob_key, sharded_thumb_key, BlobMeta};
use buzz_core::tenant::CommunityId;

/// Operation selected by a maintenance binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationOperation {
    Backfill,
    DeleteLegacy,
}

/// Derived payload pair for one community sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationObject {
    pub legacy: String,
    pub sharded: String,
}

/// Parse `_meta/<canonical-community-uuid>/<sha>.json`.
pub fn parse_sidecar_key(key: &str) -> Option<(CommunityId, &str)> {
    let rest = key.strip_prefix("_meta/")?;
    let (community, filename) = rest.split_once('/')?;
    if filename.contains('/') {
        return None;
    }
    let sha = filename.strip_suffix(".json")?;
    let uuid = uuid::Uuid::parse_str(community).ok()?;
    if uuid.hyphenated().to_string() != community {
        return None;
    }
    // Reuse key validation without inventing a second SHA validator.
    legacy_thumb_key(sha).ok()?;
    Some((CommunityId::from_uuid(uuid), sha))
}

/// Derive payload and optional thumbnail pairs represented by a sidecar.
pub fn objects_for_sidecar(
    community: CommunityId,
    sha: &str,
    meta: &BlobMeta,
) -> Result<Vec<MigrationObject>, crate::MediaKeyError> {
    let mut objects = vec![MigrationObject {
        legacy: legacy_blob_key(sha, &meta.ext)?,
        sharded: sharded_blob_key(community, sha, &meta.ext)?,
    }];
    if !meta.thumb_url.is_empty() {
        objects.push(MigrationObject {
            legacy: legacy_thumb_key(sha)?,
            sharded: sharded_thumb_key(community, sha)?,
        });
    }
    Ok(objects)
}

/// Simple globally paced request limiter. One permit is consumed for every S3
/// request, so HEAD+copy/delete work stays below the configured request rate.
pub struct RequestPacer {
    interval: tokio::time::Interval,
}

impl RequestPacer {
    pub fn new(requests_per_second: u32) -> Self {
        let period = Duration::from_secs_f64(1.0 / f64::from(requests_per_second));
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        Self { interval }
    }

    pub async fn wait(&mut self) {
        self.interval.tick().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    #[test]
    fn sidecar_parser_is_strict() {
        let community = uuid::Uuid::from_u128(1);
        assert_eq!(
            parse_sidecar_key(&format!("_meta/{community}/{SHA}.json")),
            Some((CommunityId::from_uuid(community), SHA))
        );
        for invalid in [
            format!("_meta/{community}/{SHA}.json/extra"),
            format!("_meta/{}/{SHA}.json", community.simple()),
            format!("_meta/{community}/ABC.json"),
            format!("media/{community}/{SHA}.json"),
        ] {
            assert_eq!(parse_sidecar_key(&invalid), None, "{invalid}");
        }
    }

    #[test]
    fn derives_blob_and_only_existing_thumbnail_variant() {
        let community = CommunityId::from_uuid(uuid::Uuid::from_u128(1));
        let mut meta = BlobMeta {
            ext: "jpg".into(),
            ..BlobMeta::default()
        };
        assert_eq!(objects_for_sidecar(community, SHA, &meta).unwrap().len(), 1);
        meta.thumb_url = "https://media.example/thumb".into();
        let objects = objects_for_sidecar(community, SHA, &meta).unwrap();
        assert_eq!(objects.len(), 2);
        assert!(objects[1].legacy.ends_with(".thumb.jpg"));
    }
}
