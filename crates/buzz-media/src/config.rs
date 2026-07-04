//! Media storage configuration.

fn default_max_video_bytes() -> u64 {
    524_288_000 // 500 MB
}

fn default_max_file_bytes() -> u64 {
    104_857_600 // 100 MB
}

fn default_s3_region() -> String {
    "us-east-1".to_string()
}

/// Configuration for media storage (S3/MinIO).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct MediaConfig {
    /// S3-compatible endpoint URL (e.g. "http://localhost:9000").
    pub s3_endpoint: String,
    /// S3 access key.
    pub s3_access_key: String,
    /// S3 secret key.
    pub s3_secret_key: String,
    /// S3 bucket name.
    pub s3_bucket: String,
    /// AWS region for SigV4 request signing (e.g. "us-west-2").
    ///
    /// Must match the region of `s3_endpoint` for real AWS S3, otherwise
    /// requests are signed with the wrong credential scope and AWS rejects
    /// them. Defaults to "us-east-1" to preserve MinIO/local behavior, where
    /// the value is not meaningfully checked.
    #[serde(default = "default_s3_region")]
    pub s3_region: String,
    /// Maximum upload size for images (bytes). Default: 50 MB.
    pub max_image_bytes: u64,
    /// Maximum upload size for animated GIFs (bytes). Default: 10 MB.
    pub max_gif_bytes: u64,
    /// Maximum upload size for video files (bytes). Default: 500 MB.
    #[serde(default = "default_max_video_bytes")]
    pub max_video_bytes: u64,
    /// Maximum upload size for generic (non-image, non-video) files (bytes). Default: 100 MB.
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    /// Public base URL for media URLs in BlobDescriptor (must include `/media` path).
    pub public_base_url: String,
}

impl MediaConfig {
    /// Validate configuration at startup. Returns an error on invalid config.
    pub fn validate(&self) -> Result<(), String> {
        if !self.public_base_url.ends_with("/media") {
            return Err(format!(
                "public_base_url must end with /media: got '{}'",
                self.public_base_url
            ));
        }
        if self.public_base_url.ends_with('/') {
            return Err(format!(
                "public_base_url must not end with /: got '{}'",
                self.public_base_url
            ));
        }
        if self.max_image_bytes == 0 {
            return Err("max_image_bytes must be > 0".to_string());
        }
        if self.max_gif_bytes == 0 || self.max_gif_bytes > self.max_image_bytes {
            return Err("max_gif_bytes must be > 0 and <= max_image_bytes".to_string());
        }
        if self.max_video_bytes == 0 {
            return Err("max_video_bytes must be > 0".to_string());
        }
        if self.max_file_bytes == 0 {
            return Err("max_file_bytes must be > 0".to_string());
        }
        Ok(())
    }
}
