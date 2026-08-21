use anyhow::{bail, Context, Result};
use buzz_media::migration::{MigrationObject, RequestPacer};
use buzz_media::{MediaConfig, MediaError, MediaMigrationPhase, MediaStorage, S3AddressingStyle};
use clap::Args;

#[derive(Debug, Args)]
pub struct CommonArgs {
    #[arg(long, env = "BUZZ_S3_ENDPOINT")]
    pub s3_endpoint: String,
    #[arg(long, env = "BUZZ_S3_BUCKET")]
    pub s3_bucket: String,
    #[arg(long, env = "BUZZ_S3_REGION", default_value = "us-east-1")]
    pub s3_region: String,
    #[arg(long, env = "BUZZ_S3_ACCESS_KEY", default_value = "")]
    pub s3_access_key: String,
    #[arg(long, env = "BUZZ_S3_SECRET_KEY", default_value = "")]
    pub s3_secret_key: String,
    #[arg(long, env = "BUZZ_S3_ADDRESSING_STYLE", default_value = "path")]
    pub s3_addressing_style: S3AddressingStyle,
    /// Maximum S3 requests per second, including listing and verification.
    #[arg(
        long,
        env = "BUZZ_MEDIA_MIGRATION_REQUESTS_PER_SECOND",
        default_value_t = 25
    )]
    pub requests_per_second: u32,
    #[arg(long, env = "BUZZ_MEDIA_MIGRATION_PAGE_SIZE", default_value_t = 100)]
    pub page_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestinationVerification {
    /// The legacy source is already gone (or never existed). There is nothing
    /// left for backfill to copy or cleanup to delete for this object.
    LegacySourceAbsent,
    /// The legacy source exists and the sharded destination exists with the same bytes.
    Verified,
    /// The legacy source exists, but the sharded destination has not been created yet.
    DestinationMissing,
    /// Both objects exist, but the sharded destination does not match the legacy bytes.
    ByteMismatch,
}

fn classify_destination_verification(
    source: Result<Vec<u8>, MediaError>,
    destination: Result<Vec<u8>, MediaError>,
    object: &MigrationObject,
) -> Result<DestinationVerification> {
    if matches!(source, Err(MediaError::NotFound)) {
        if let Err(error) = destination {
            if !matches!(error, MediaError::NotFound) {
                return Err(error).with_context(|| {
                    format!(
                        "read sharded destination for verification: {}",
                        object.sharded
                    )
                });
            }
        }
        return Ok(DestinationVerification::LegacySourceAbsent);
    }

    let source = match source {
        Ok(bytes) => bytes,
        Err(MediaError::NotFound) => unreachable!("handled above"),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("read legacy source for verification: {}", object.legacy)
            });
        }
    };

    let destination = match destination {
        Ok(bytes) => bytes,
        Err(MediaError::NotFound) => return Ok(DestinationVerification::DestinationMissing),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "read sharded destination for verification: {}",
                    object.sharded
                )
            });
        }
    };

    Ok(if destination == source {
        DestinationVerification::Verified
    } else {
        DestinationVerification::ByteMismatch
    })
}

pub async fn verify_destination(
    storage: &MediaStorage,
    pacer: &mut RequestPacer,
    object: &MigrationObject,
) -> Result<DestinationVerification> {
    pacer.wait().await;
    let source = storage.get(&object.legacy).await;

    pacer.wait().await;
    let destination = storage.get(&object.sharded).await;

    classify_destination_verification(source, destination, object)
}

impl CommonArgs {
    pub fn storage(&self) -> Result<MediaStorage> {
        if self.requests_per_second == 0 {
            bail!("requests-per-second must be greater than zero");
        }
        if !(1..=1000).contains(&self.page_size) {
            bail!("page-size must be between 1 and 1000");
        }
        MediaStorage::new(&MediaConfig {
            s3_endpoint: self.s3_endpoint.clone(),
            s3_access_key: self.s3_access_key.clone(),
            s3_secret_key: self.s3_secret_key.clone(),
            s3_bucket: self.s3_bucket.clone(),
            s3_region: self.s3_region.clone(),
            s3_addressing_style: self.s3_addressing_style,
            migration_phase: MediaMigrationPhase::LegacyOnly,
            max_image_bytes: 1,
            max_gif_bytes: 1,
            max_video_bytes: 1,
            max_file_bytes: 1,
            public_base_url: "http://localhost/media".into(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        })
        .context("create media S3 client")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object() -> MigrationObject {
        MigrationObject {
            legacy: "abc.png".to_string(),
            sharded: "media/ab/cd/abc.png".to_string(),
        }
    }

    #[test]
    fn classifies_matching_destination_as_verified() {
        let result = classify_destination_verification(
            Ok(b"same".to_vec()),
            Ok(b"same".to_vec()),
            &object(),
        )
        .expect("verification should classify");

        assert_eq!(result, DestinationVerification::Verified);
    }

    #[test]
    fn classifies_absent_legacy_source_as_skip() {
        let result = classify_destination_verification(
            Err(MediaError::NotFound),
            Ok(b"sharded-only".to_vec()),
            &object(),
        )
        .expect("not-found legacy source should be non-fatal");

        assert_eq!(result, DestinationVerification::LegacySourceAbsent);
    }

    #[test]
    fn classifies_present_source_with_missing_destination_as_missing() {
        let result = classify_destination_verification(
            Ok(b"source".to_vec()),
            Err(MediaError::NotFound),
            &object(),
        )
        .expect("missing destination should be classified");

        assert_eq!(result, DestinationVerification::DestinationMissing);
    }

    #[test]
    fn classifies_present_source_with_mismatched_destination_as_mismatch() {
        let result = classify_destination_verification(
            Ok(b"source".to_vec()),
            Ok(b"different".to_vec()),
            &object(),
        )
        .expect("mismatched destination should be classified");

        assert_eq!(result, DestinationVerification::ByteMismatch);
    }

    #[test]
    fn keeps_non_not_found_source_errors_fatal() {
        let error = classify_destination_verification(
            Err(MediaError::StorageError("source boom".to_string())),
            Ok(b"destination".to_vec()),
            &object(),
        )
        .expect_err("source storage errors must remain fatal");

        assert!(
            error
                .to_string()
                .contains("read legacy source for verification"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn keeps_non_not_found_destination_errors_fatal_with_present_source() {
        let error = classify_destination_verification(
            Ok(b"source".to_vec()),
            Err(MediaError::StorageError("destination boom".to_string())),
            &object(),
        )
        .expect_err("destination storage errors must remain fatal");

        assert!(
            error
                .to_string()
                .contains("read sharded destination for verification"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn keeps_non_not_found_destination_errors_fatal_with_absent_source() {
        let error = classify_destination_verification(
            Err(MediaError::NotFound),
            Err(MediaError::StorageError("destination boom".to_string())),
            &object(),
        )
        .expect_err("destination storage errors must remain fatal");

        assert!(
            error
                .to_string()
                .contains("read sharded destination for verification"),
            "unexpected error: {error}"
        );
    }
}
