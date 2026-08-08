//! Deterministic object-key derivation for media payloads.
//!
//! Public Blossom URLs stay flat (`/media/<sha>.<ext>`), while S3 payloads use
//! hash-leading shards so aggregate request traffic is distributed before the
//! community segment. Legacy keys remain read candidates during migration.

use buzz_core::tenant::{CommunityId, TenantContext};

/// Invalid data supplied to media object-key construction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MediaKeyError {
    /// SHA-256 must be exactly 64 lowercase hexadecimal characters.
    #[error("invalid SHA-256 digest")]
    InvalidSha256,
    /// Extensions are canonical lowercase alphanumeric tokens of 1-8 bytes.
    #[error("invalid media extension")]
    InvalidExtension,
    /// Only `<sha>.<ext>` and `<sha>.thumb.jpg` payload names are accepted.
    #[error("invalid media payload name")]
    InvalidPayloadName,
}

/// Ordered object keys for compatibility reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaReadCandidates {
    /// Hash-sharded, community-scoped key tried first.
    pub sharded: String,
    /// Flat pre-migration key tried only when `sharded` is not found.
    pub legacy: String,
}

fn validate_sha256(sha256: &str) -> Result<(), MediaKeyError> {
    if sha256.len() == 64
        && sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(MediaKeyError::InvalidSha256)
    }
}

fn validate_extension(ext: &str) -> Result<(), MediaKeyError> {
    if (1..=8).contains(&ext.len())
        && ext
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
    {
        Ok(())
    } else {
        Err(MediaKeyError::InvalidExtension)
    }
}

/// Flat pre-migration blob key: `<sha256>.<ext>`.
pub fn legacy_blob_key(sha256: &str, ext: &str) -> Result<String, MediaKeyError> {
    validate_sha256(sha256)?;
    validate_extension(ext)?;
    Ok(format!("{sha256}.{ext}"))
}

/// Hash-leading blob key: `media/<2>/<2>/<community>/<sha256>.<ext>`.
pub fn sharded_blob_key(
    community: CommunityId,
    sha256: &str,
    ext: &str,
) -> Result<String, MediaKeyError> {
    let filename = legacy_blob_key(sha256, ext)?;
    Ok(format!(
        "media/{}/{}/{community}/{filename}",
        &sha256[..2],
        &sha256[2..4]
    ))
}

/// Flat pre-migration thumbnail key: `<sha256>.thumb.jpg`.
pub fn legacy_thumb_key(sha256: &str) -> Result<String, MediaKeyError> {
    validate_sha256(sha256)?;
    Ok(format!("{sha256}.thumb.jpg"))
}

/// Hash-leading thumbnail key: `media/<2>/<2>/<community>/<sha256>.thumb.jpg`.
pub fn sharded_thumb_key(community: CommunityId, sha256: &str) -> Result<String, MediaKeyError> {
    let filename = legacy_thumb_key(sha256)?;
    Ok(format!(
        "media/{}/{}/{community}/{filename}",
        &sha256[..2],
        &sha256[2..4]
    ))
}

/// Build new-first, legacy-fallback candidates from a validated public payload name.
///
/// The community always comes from the server-resolved tenant context; callers
/// cannot supply it through a URL, sidecar, or upload record.
pub fn read_candidates(
    ctx: &TenantContext,
    payload_name: &str,
) -> Result<MediaReadCandidates, MediaKeyError> {
    if let Some(sha256) = payload_name.strip_suffix(".thumb.jpg") {
        return Ok(MediaReadCandidates {
            sharded: sharded_thumb_key(ctx.community(), sha256)?,
            legacy: legacy_thumb_key(sha256)?,
        });
    }

    let (sha256, ext) = payload_name
        .split_once('.')
        .ok_or(MediaKeyError::InvalidPayloadName)?;
    if ext.contains('.') {
        return Err(MediaKeyError::InvalidPayloadName);
    }
    Ok(MediaReadCandidates {
        sharded: sharded_blob_key(ctx.community(), sha256, ext)?,
        legacy: legacy_blob_key(sha256, ext)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const SHA: &str = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    fn tenant(n: u128) -> TenantContext {
        TenantContext::resolved(CommunityId::from_uuid(Uuid::from_u128(n)), "media.example")
    }

    #[test]
    fn derives_hash_leading_community_scoped_blob_and_thumb_keys() {
        let ctx = tenant(1);
        let community = ctx.community();

        assert_eq!(
            sharded_blob_key(community, SHA, "jpg").unwrap(),
            format!("media/ab/cd/{community}/{SHA}.jpg")
        );
        assert_eq!(
            sharded_thumb_key(community, SHA).unwrap(),
            format!("media/ab/cd/{community}/{SHA}.thumb.jpg")
        );
        assert_ne!(
            sharded_blob_key(community, SHA, "jpg").unwrap(),
            sharded_blob_key(tenant(2).community(), SHA, "jpg").unwrap()
        );
    }

    #[test]
    fn orders_sharded_before_legacy_for_blobs_and_thumbnails() {
        let ctx = tenant(1);
        let community = ctx.community();

        assert_eq!(
            read_candidates(&ctx, &format!("{SHA}.png")).unwrap(),
            MediaReadCandidates {
                sharded: format!("media/ab/cd/{community}/{SHA}.png"),
                legacy: format!("{SHA}.png"),
            }
        );
        assert_eq!(
            read_candidates(&ctx, &format!("{SHA}.thumb.jpg")).unwrap(),
            MediaReadCandidates {
                sharded: format!("media/ab/cd/{community}/{SHA}.thumb.jpg"),
                legacy: format!("{SHA}.thumb.jpg"),
            }
        );
    }

    #[test]
    fn rejects_noncanonical_or_ambiguous_inputs() {
        for sha in ["abc", &"A".repeat(64), &"g".repeat(64)] {
            assert_eq!(
                legacy_blob_key(sha, "jpg"),
                Err(MediaKeyError::InvalidSha256)
            );
        }
        for ext in ["", "JPG", "tar.gz", "toolongext", "../jpg"] {
            assert_eq!(
                legacy_blob_key(SHA, ext),
                Err(MediaKeyError::InvalidExtension)
            );
        }
        assert_eq!(
            read_candidates(&tenant(1), SHA),
            Err(MediaKeyError::InvalidPayloadName)
        );
    }
}
