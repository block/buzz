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
    /// Whether to write per-upload-event records under `_uploads/`
    /// (moderation side channel). Off by default; set via
    /// `BUZZ_MEDIA_UPLOAD_RECORDS=true`.
    #[serde(default)]
    pub upload_records_enabled: bool,
    /// Trusted edge header to read the uploader's public IP from (e.g.
    /// `cf-connecting-ip`). Unset (default) → no IP is read or recorded.
    /// Only consulted when `upload_records_enabled` is true; the value is
    /// validated as a public IP and dropped otherwise (fail-empty).
    #[serde(default)]
    pub upload_ip_header: Option<String>,
    /// Trusted edge header to read the uploader's source port from. Standard
    /// edges don't emit one, so this is usually unset; a port is only
    /// recorded alongside a valid IP.
    #[serde(default)]
    pub upload_port_header: Option<String>,
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
        // Fail startup on incoherent collection config instead of silently
        // recording nothing — an operator who set an IP header believes they
        // are meeting a reporting obligation.
        if self.upload_ip_header.is_some() && !self.upload_records_enabled {
            return Err(
                "BUZZ_MEDIA_UPLOAD_IP_HEADER is set but BUZZ_MEDIA_UPLOAD_RECORDS is not \
                 enabled — the IP would never be recorded. Enable upload records or unset \
                 the header."
                    .to_string(),
            );
        }
        if self.upload_port_header.is_some() && self.upload_ip_header.is_none() {
            return Err(
                "BUZZ_MEDIA_UPLOAD_PORT_HEADER is set without BUZZ_MEDIA_UPLOAD_IP_HEADER — \
                 a port is only recorded alongside an IP. Set the IP header or unset the \
                 port header."
                    .to_string(),
            );
        }
        for (name, value) in [
            ("BUZZ_MEDIA_UPLOAD_IP_HEADER", &self.upload_ip_header),
            ("BUZZ_MEDIA_UPLOAD_PORT_HEADER", &self.upload_port_header),
        ] {
            if let Some(h) = value {
                if axum::http::HeaderName::from_bytes(h.as_bytes()).is_err() {
                    return Err(format!("{name} is not a valid header name: {h:?}"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::MediaConfig;

    fn valid_config() -> MediaConfig {
        MediaConfig {
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_access_key: "k".to_string(),
            s3_secret_key: "s".to_string(),
            s3_bucket: "buzz-media".to_string(),
            s3_region: "us-east-1".to_string(),
            max_image_bytes: 1,
            max_gif_bytes: 1,
            max_video_bytes: 1,
            max_file_bytes: 1,
            public_base_url: "http://localhost:3000/media".to_string(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        }
    }

    #[test]
    fn upload_record_knobs_default_off_and_validate() {
        assert!(valid_config().validate().is_ok());

        let mut on = valid_config();
        on.upload_records_enabled = true;
        assert!(on.validate().is_ok());

        on.upload_ip_header = Some("cf-connecting-ip".to_string());
        assert!(on.validate().is_ok());

        on.upload_port_header = Some("x-client-port".to_string());
        assert!(on.validate().is_ok());
    }

    #[test]
    fn ip_header_without_records_fails_startup() {
        // An operator who set the header believes IPs are being recorded —
        // fail loudly instead of silently collecting nothing.
        let mut cfg = valid_config();
        cfg.upload_ip_header = Some("cf-connecting-ip".to_string());
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn port_header_without_ip_header_fails_startup() {
        let mut cfg = valid_config();
        cfg.upload_records_enabled = true;
        cfg.upload_port_header = Some("x-client-port".to_string());
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn malformed_header_names_fail_startup() {
        let mut cfg = valid_config();
        cfg.upload_records_enabled = true;
        for bad in ["with space", "colon:name", "bad/header", "bad,header", ""] {
            cfg.upload_ip_header = Some(bad.to_string());
            assert!(cfg.validate().is_err(), "should reject header name {bad:?}");
        }
    }
}
