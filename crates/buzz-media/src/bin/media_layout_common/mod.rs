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
    /// Resume after this sidecar key. The final log line prints the next value.
    #[arg(long, env = "BUZZ_MEDIA_MIGRATION_START_AFTER")]
    pub start_after: Option<String>,
    #[arg(long, env = "BUZZ_MEDIA_MIGRATION_PAGE_SIZE", default_value_t = 100)]
    pub page_size: usize,
}

pub async fn verify_destination(
    storage: &MediaStorage,
    pacer: &mut RequestPacer,
    object: &MigrationObject,
) -> Result<bool> {
    pacer.wait().await;
    let source = storage
        .get(&object.legacy)
        .await
        .with_context(|| format!("read legacy source for verification: {}", object.legacy))?;

    pacer.wait().await;
    let destination = match storage.get(&object.sharded).await {
        Ok(bytes) => bytes,
        Err(MediaError::NotFound) => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "read sharded destination for verification: {}",
                    object.sharded
                )
            });
        }
    };

    Ok(destination == source)
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
