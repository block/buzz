//! S3/MinIO storage client.

use std::path::{Component, Path, PathBuf};
use std::pin::Pin;

use buzz_core::tenant::{CommunityId, TenantContext};

use crate::config::{MediaConfig, S3AddressingStyle};
use crate::error::MediaError;
use bytes::Bytes;
use futures_util::StreamExt;
use s3::creds::Credentials;
use s3::{Bucket, Region};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

/// A stream of byte chunks from S3, usable with `axum::body::Body::from_stream()`.
pub type ByteStream = Pin<Box<dyn futures_core::Stream<Item = Result<Bytes, MediaError>> + Send>>;

/// S3-compatible object storage client.
pub struct MediaStorage {
    backend: MediaBackend,
}

enum MediaBackend {
    S3(S3MediaStorage),
    Filesystem(FilesystemMediaStorage),
}

struct FilesystemMediaStorage {
    root: PathBuf,
}

struct S3MediaStorage {
    bucket: Box<Bucket>,
}

impl MediaStorage {
    /// Create a new storage client from config.
    ///
    /// Credential selection:
    /// - If both `s3_access_key` and `s3_secret_key` are non-empty, use them as
    ///   static credentials (MinIO/local/dev, or any static-key deployment).
    /// - Otherwise, fall back to the AWS default credential chain via
    ///   [`Credentials::default`]: environment, shared profile, web-identity
    ///   token (IRSA on EKS — `AssumeRoleWithWebIdentity`), container, and
    ///   instance-metadata providers, in that order. This lets the relay use
    ///   the pod's IAM role without long-lived static keys.
    pub fn new(config: &MediaConfig) -> Result<Self, MediaError> {
        let region = Region::Custom {
            region: config.s3_region.clone(),
            endpoint: config.s3_endpoint.clone(),
        };
        let creds = match (
            config.s3_access_key.is_empty(),
            config.s3_secret_key.is_empty(),
        ) {
            (false, false) => Credentials::new(
                Some(&config.s3_access_key),
                Some(&config.s3_secret_key),
                None,
                None,
                None,
            ),
            (true, true) => {
                // No static keys configured: resolve from the AWS credential chain
                // (IRSA web-identity, env, profile, instance metadata).
                Credentials::default()
            }
            _ => {
                return Err(MediaError::StorageError(
                    "s3_access_key and s3_secret_key must be configured together, or both empty to use the AWS credential chain"
                        .to_string(),
                ));
            }
        }
        .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let bucket = Bucket::new(&config.s3_bucket, region, creds)
            .map_err(|e| MediaError::StorageError(e.to_string()))?;
        let bucket = match config.s3_addressing_style {
            S3AddressingStyle::Path => bucket.with_path_style(),
            S3AddressingStyle::Virtual => bucket,
        };
        Ok(Self {
            backend: MediaBackend::S3(S3MediaStorage { bucket }),
        })
    }

    /// Creates a filesystem-backed content-addressed media store rooted at `root`.
    pub fn filesystem(root: impl Into<PathBuf>) -> Self {
        Self {
            backend: MediaBackend::Filesystem(FilesystemMediaStorage { root: root.into() }),
        }
    }

    #[cfg(test)]
    fn s3(&self) -> &S3MediaStorage {
        match &self.backend {
            MediaBackend::S3(storage) => storage,
            MediaBackend::Filesystem(_) => panic!("filesystem media storage has no S3 client"),
        }
    }

    fn filesystem_path(storage: &FilesystemMediaStorage, key: &str) -> Result<PathBuf, MediaError> {
        let key_path = Path::new(key);
        if key.is_empty()
            || key.contains('\\')
            || key
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || key_path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(MediaError::StorageError(
                "invalid media storage key".to_owned(),
            ));
        }
        let filename = key_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| MediaError::StorageError("invalid media storage key".to_owned()))?;
        let sha = filename.split('.').next().unwrap_or_default();
        if sha.len() >= 4 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(storage.root.join(&sha[..2]).join(&sha[2..4]).join(key_path))
        } else {
            Ok(storage.root.join("_aux").join(key_path))
        }
    }

    async fn atomic_filesystem_write(path: &Path, bytes: &[u8]) -> Result<(), MediaError> {
        let parent = path
            .parent()
            .ok_or_else(|| MediaError::StorageError("media path has no parent".to_owned()))?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(Self::fs_error)?;
        let temp = tempfile::NamedTempFile::new_in(parent).map_err(Self::fs_error)?;
        tokio::fs::write(temp.path(), bytes)
            .await
            .map_err(Self::fs_error)?;
        temp.persist(path)
            .map_err(|error| Self::fs_error(error.error))?;
        Ok(())
    }

    async fn atomic_filesystem_copy(source: &Path, target: &Path) -> Result<(), MediaError> {
        let parent = target
            .parent()
            .ok_or_else(|| MediaError::StorageError("media path has no parent".to_owned()))?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(Self::fs_error)?;
        let temp = tempfile::NamedTempFile::new_in(parent).map_err(Self::fs_error)?;
        tokio::fs::copy(source, temp.path())
            .await
            .map_err(Self::fs_error)?;
        temp.persist(target)
            .map_err(|error| Self::fs_error(error.error))?;
        Ok(())
    }

    fn fs_error(error: std::io::Error) -> MediaError {
        if error.kind() == std::io::ErrorKind::NotFound {
            MediaError::NotFound
        } else {
            MediaError::Io(error.to_string())
        }
    }

    /// Store an object from a byte slice.
    pub async fn put(&self, key: &str, bytes: &[u8], content_type: &str) -> Result<(), MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => {
                storage
                    .bucket
                    .put_object_with_content_type(key, bytes, content_type)
                    .await?;
                Ok(())
            }
            MediaBackend::Filesystem(storage) => {
                let path = Self::filesystem_path(storage, key)?;
                Self::atomic_filesystem_write(&path, bytes).await
            }
        }
    }

    /// Stream a file from disk into storage without loading it into RAM.
    pub async fn put_file(
        &self,
        key: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<(), MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => {
                const BUF: usize = 8 * 1024 * 1024;
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(|e| MediaError::Io(e.to_string()))?;
                let mut reader = tokio::io::BufReader::with_capacity(BUF, file);
                storage
                    .bucket
                    .put_object_stream_with_content_type(&mut reader, key, content_type)
                    .await?;
                Ok(())
            }
            MediaBackend::Filesystem(storage) => {
                let target = Self::filesystem_path(storage, key)?;
                Self::atomic_filesystem_copy(path, &target).await
            }
        }
    }

    /// Retrieve an object's bytes.
    pub async fn get(&self, key: &str) -> Result<Vec<u8>, MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => match storage.bucket.get_object(key).await {
                Ok(response) => Ok(response.to_vec()),
                Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Err(MediaError::NotFound),
                Err(e) => Err(MediaError::StorageError(e.to_string())),
            },
            MediaBackend::Filesystem(storage) => {
                tokio::fs::read(Self::filesystem_path(storage, key)?)
                    .await
                    .map_err(Self::fs_error)
            }
        }
    }

    /// Retrieve an inclusive byte range from an object.
    pub async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Vec<u8>, MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => {
                match storage.bucket.get_object_range(key, start, Some(end)).await {
                    Ok(response) => Ok(response.to_vec()),
                    Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Err(MediaError::NotFound),
                    Err(e) => Err(MediaError::StorageError(e.to_string())),
                }
            }
            MediaBackend::Filesystem(storage) => {
                let path = Self::filesystem_path(storage, key)?;
                let mut file = tokio::fs::File::open(path).await.map_err(Self::fs_error)?;
                let length = end
                    .checked_sub(start)
                    .and_then(|length| length.checked_add(1))
                    .ok_or_else(|| MediaError::StorageError("invalid media range".to_owned()))?;
                let size = file.metadata().await.map_err(Self::fs_error)?.len();
                if start >= size || end >= size {
                    return Err(MediaError::StorageError("invalid media range".to_owned()));
                }
                file.seek(std::io::SeekFrom::Start(start))
                    .await
                    .map_err(Self::fs_error)?;
                let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
                file.take(length)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(Self::fs_error)?;
                Ok(bytes)
            }
        }
    }

    /// Stream an object's bytes without buffering its full content.
    pub async fn get_stream(&self, key: &str) -> Result<ByteStream, MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => {
                let response = storage
                    .bucket
                    .get_object_stream(key)
                    .await
                    .map_err(|e| MediaError::StorageError(e.to_string()))?;
                if response.status_code == 404 {
                    return Err(MediaError::NotFound);
                }
                Ok(Box::pin(futures_util::StreamExt::map(
                    response.bytes,
                    |chunk| chunk.map_err(|e| MediaError::StorageError(e.to_string())),
                )))
            }
            MediaBackend::Filesystem(storage) => {
                let file = tokio::fs::File::open(Self::filesystem_path(storage, key)?)
                    .await
                    .map_err(Self::fs_error)?;
                Ok(Box::pin(
                    ReaderStream::new(file).map(|chunk| chunk.map_err(Self::fs_error)),
                ))
            }
        }
    }

    /// Check if an object exists. Returns false on absence.
    pub async fn head(&self, key: &str) -> Result<bool, MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => match storage.bucket.head_object(key).await {
                Ok(_) => Ok(true),
                Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Ok(false),
                Err(e) => Err(MediaError::StorageError(e.to_string())),
            },
            MediaBackend::Filesystem(storage) => {
                match tokio::fs::metadata(Self::filesystem_path(storage, key)?).await {
                    Ok(_) => Ok(true),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                    Err(error) => Err(Self::fs_error(error)),
                }
            }
        }
    }

    /// Delete an object.
    pub async fn delete(&self, key: &str) -> Result<(), MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => {
                storage
                    .bucket
                    .delete_object(key)
                    .await
                    .map_err(|e| MediaError::StorageError(e.to_string()))?;
                Ok(())
            }
            MediaBackend::Filesystem(storage) => {
                tokio::fs::remove_file(Self::filesystem_path(storage, key)?)
                    .await
                    .map_err(Self::fs_error)
            }
        }
    }

    /// HEAD with metadata — returns Content-Length (size).
    pub async fn head_with_metadata(&self, key: &str) -> Result<Option<BlobHeadMeta>, MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => match storage.bucket.head_object(key).await {
                Ok((result, _)) => Ok(Some(BlobHeadMeta {
                    size: result.content_length.unwrap_or(0) as u64,
                })),
                Err(s3::error::S3Error::HttpFailWithBody(404, _)) => Ok(None),
                Err(e) => Err(MediaError::StorageError(e.to_string())),
            },
            MediaBackend::Filesystem(storage) => {
                match tokio::fs::metadata(Self::filesystem_path(storage, key)?).await {
                    Ok(metadata) => Ok(Some(BlobHeadMeta {
                        size: metadata.len(),
                    })),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(Self::fs_error(error)),
                }
            }
        }
    }

    /// Build the community-scoped sidecar key for a given sha256 (bare hash).
    ///
    /// Raw media bytes remain shared content-addressed CAS (`{sha}.{ext}`), but
    /// the metadata sidecar is the tenant read gate. A blob in another
    /// community must never be observable through a global `_meta/{sha}.json`
    /// lookup.
    pub fn sidecar_key(community: CommunityId, sha256: &str) -> String {
        format!("_meta/{community}/{sha256}.json")
    }

    /// Build the community-scoped sidecar key from the resolved request tenant.
    pub fn ctx_sidecar_key(ctx: &TenantContext, sha256: &str) -> String {
        Self::sidecar_key(ctx.community(), sha256)
    }

    /// Read community-scoped sidecar JSON for a given sha256 (bare hash).
    pub async fn get_sidecar(
        &self,
        ctx: &TenantContext,
        sha256: &str,
    ) -> Result<BlobMeta, MediaError> {
        let key = Self::ctx_sidecar_key(ctx, sha256);
        let bytes = self.get(&key).await?;
        let meta: BlobMeta = serde_json::from_slice(&bytes)?;
        Ok(meta)
    }

    /// Write community-scoped sidecar JSON for a given sha256 (bare hash).
    ///
    /// `ctx` must be the server-resolved request tenant. Callers must never
    /// derive the community from client-supplied blob metadata, URLs, or event
    /// tags; this sidecar key is the tenant read gate for otherwise shared CAS
    /// bytes.
    pub async fn put_sidecar(
        &self,
        ctx: &TenantContext,
        sha256: &str,
        meta: &BlobMeta,
    ) -> Result<(), MediaError> {
        let key = Self::ctx_sidecar_key(ctx, sha256);
        let meta_json = serde_json::to_vec(meta)?;
        self.put(&key, &meta_json, "application/json").await
    }

    /// Convenience: read just the MIME type from the community sidecar.
    ///
    /// Returns `None` for both absent sidecars and storage read failures. Public
    /// read handlers intentionally collapse that distinction to 404 so an
    /// A-bound request cannot distinguish a B-only blob from a missing blob.
    pub async fn read_sidecar_mime(&self, ctx: &TenantContext, sha256_ext: &str) -> Option<String> {
        let sha256 = sha256_ext.split('.').next().unwrap_or(sha256_ext);
        self.get_sidecar(ctx, sha256)
            .await
            .ok()
            .map(|m| m.mime_type)
    }

    /// One page of a full-bucket listing.
    pub async fn list_page(
        &self,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<crate::bucket_index::Page, MediaError> {
        match &self.backend {
            MediaBackend::S3(storage) => {
                let (result, _) = storage
                    .bucket
                    .list_page(
                        String::new(),
                        None,
                        continuation_token,
                        None,
                        Some(max_keys),
                    )
                    .await?;
                Ok(crate::bucket_index::Page {
                    objects: result
                        .contents
                        .into_iter()
                        .map(|obj| (obj.key, obj.size))
                        .collect(),
                    next_continuation_token: result.next_continuation_token,
                    is_truncated: result.is_truncated,
                })
            }
            MediaBackend::Filesystem(storage) => {
                let root = storage.root.clone();
                let mut files = tokio::task::spawn_blocking(move || {
                    fn walk(
                        root: &Path,
                        base: &Path,
                        out: &mut Vec<(String, u64)>,
                    ) -> std::io::Result<()> {
                        for entry in std::fs::read_dir(root)? {
                            let entry = entry?;
                            let path = entry.path();
                            if path.is_dir() {
                                walk(&path, base, out)?;
                            } else {
                                let key = path
                                    .strip_prefix(base)
                                    .unwrap()
                                    .to_string_lossy()
                                    .replace('\\', "/");
                                if let Some(key) = key.strip_prefix("_aux/") {
                                    out.push((key.to_owned(), entry.metadata()?.len()));
                                } else if let Some((_, _, key)) =
                                    key.split_once('/').and_then(|(a, rest)| {
                                        rest.split_once('/').map(|(b, c)| (a, b, c))
                                    })
                                {
                                    out.push((key.to_owned(), entry.metadata()?.len()));
                                }
                            }
                        }
                        Ok(())
                    }
                    let mut out = Vec::new();
                    if root.exists() {
                        walk(&root, &root, &mut out)?;
                    }
                    out.sort_by(|a, b| a.0.cmp(&b.0));
                    Ok::<_, std::io::Error>(out)
                })
                .await
                .map_err(|e| MediaError::StorageError(e.to_string()))?
                .map_err(Self::fs_error)?;
                if max_keys == 0 {
                    return Err(MediaError::StorageError(
                        "list page size must be nonzero".to_owned(),
                    ));
                }
                let start = match continuation_token {
                    Some(token) => files
                        .iter()
                        .position(|(key, _)| key == &token)
                        .map(|n| n + 1)
                        .ok_or_else(|| {
                            MediaError::StorageError("unknown list continuation token".to_owned())
                        })?,
                    None => 0,
                };
                let remaining = files.split_off(start);
                let is_truncated = remaining.len() > max_keys;
                let objects: Vec<_> = remaining.into_iter().take(max_keys).collect();
                let next_continuation_token = is_truncated
                    .then(|| objects.last().expect("nonempty truncated page").0.clone());
                Ok(crate::bucket_index::Page {
                    objects,
                    next_continuation_token,
                    is_truncated,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn tenant(n: u128) -> TenantContext {
        TenantContext::resolved(
            CommunityId::from_uuid(uuid::Uuid::from_u128(n)),
            "media.example",
        )
    }

    fn storage_config(access: &str, secret: &str) -> crate::config::MediaConfig {
        crate::config::MediaConfig {
            s3_endpoint: "http://localhost:9000".to_string(),
            s3_access_key: access.to_string(),
            s3_secret_key: secret.to_string(),
            s3_bucket: "buzz-media".to_string(),
            s3_region: "us-west-2".to_string(),
            s3_addressing_style: S3AddressingStyle::Path,
            max_image_bytes: 50 * 1024 * 1024,
            max_gif_bytes: 10 * 1024 * 1024,
            max_video_bytes: 524_288_000,
            max_file_bytes: 104_857_600,
            public_base_url: "http://localhost:3000/media".to_string(),
            upload_records_enabled: false,
            upload_ip_header: None,
            upload_port_header: None,
        }
    }

    /// Static keys present: builds a client without touching the AWS
    /// credential chain (no env/metadata access), and the signing region
    /// comes from config rather than a hardcoded "us-east-1".
    #[test]
    fn static_keys_build_client_with_configured_region() {
        let storage = MediaStorage::new(&storage_config("buzz_dev", "buzz_dev_secret"))
            .expect("static creds should build a client");
        match &storage.s3().bucket.region {
            Region::Custom { region, .. } => assert_eq!(region, "us-west-2"),
            other => panic!("expected Custom region, got {other:?}"),
        }
    }

    #[test]
    fn client_constructor_applies_both_addressing_styles() {
        let path = MediaStorage::new(&storage_config("buzz_dev", "buzz_dev_secret"))
            .expect("path-style client");
        assert!(path.s3().bucket.is_path_style());
        assert_eq!(path.s3().bucket.url(), "http://localhost:9000/buzz-media");

        let mut virtual_config = storage_config("buzz_dev", "buzz_dev_secret");
        virtual_config.s3_addressing_style = S3AddressingStyle::Virtual;
        let virtual_hosted = MediaStorage::new(&virtual_config).expect("virtual-hosted client");
        assert!(virtual_hosted.s3().bucket.is_subdomain_style());
        assert_eq!(
            virtual_hosted.s3().bucket.url(),
            "http://buzz-media.localhost:9000"
        );
    }

    #[test]
    fn partial_static_keys_are_rejected() {
        let err = match MediaStorage::new(&storage_config("buzz_dev", "")) {
            Ok(_) => panic!("partial static creds must not silently use credential chain"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("must be configured together"),
            "unexpected error: {err}"
        );

        let err = match MediaStorage::new(&storage_config("", "buzz_dev_secret")) {
            Ok(_) => panic!("partial static creds must not silently use credential chain"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("must be configured together"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn filesystem_keys_reject_traversal_and_windows_separators() {
        let storage = MediaStorage::filesystem(tempfile::tempdir().unwrap().path());
        let filesystem = match &storage.backend {
            MediaBackend::Filesystem(storage) => storage,
            MediaBackend::S3(_) => unreachable!(),
        };
        for key in [
            "",
            "/absolute",
            "nested//key",
            "nested/./key",
            "nested/../key",
            r"nested\\key",
            r"..\\outside",
            r"C:\\outside",
            r"\\\\server\\share",
        ] {
            assert!(
                MediaStorage::filesystem_path(filesystem, key).is_err(),
                "{key}"
            );
        }
    }

    #[tokio::test]
    async fn filesystem_backend_roundtrips_cas_sidecars_ranges_and_pages() {
        let dir = tempfile::tempdir().unwrap();
        let storage = MediaStorage::filesystem(dir.path());
        let sha = "a".repeat(64);
        let key = format!("{sha}.bin");
        storage
            .put(&key, b"hello", "application/octet-stream")
            .await
            .unwrap();
        assert_eq!(storage.get(&key).await.unwrap(), b"hello");
        assert_eq!(storage.get_range(&key, 1, 3).await.unwrap(), b"ell");
        assert_eq!(
            storage
                .head_with_metadata(&key)
                .await
                .unwrap()
                .unwrap()
                .size,
            5
        );
        assert!(dir.path().join("aa").join("aa").join(&key).exists());

        let a = tenant(1);
        let b = tenant(2);
        storage
            .put_sidecar(
                &a,
                &sha,
                &BlobMeta {
                    mime_type: "text/plain".to_owned(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            storage.read_sidecar_mime(&a, &key).await.as_deref(),
            Some("text/plain")
        );
        assert_eq!(storage.read_sidecar_mime(&b, &key).await, None);
        let page = storage.list_page(None, 1).await.unwrap();
        assert_eq!(page.objects.len(), 1);
        assert!(page.is_truncated);
        storage.delete(&key).await.unwrap();
        assert!(!storage.head(&key).await.unwrap());
    }

    #[tokio::test]
    async fn filesystem_listing_rejects_zero_page_size_and_unknown_token() {
        let dir = tempfile::tempdir().unwrap();
        let storage = MediaStorage::filesystem(dir.path());
        let key = format!("{}.bin", "b".repeat(64));
        storage
            .put(&key, b"x", "application/octet-stream")
            .await
            .unwrap();
        assert!(storage
            .list_page(None, 0)
            .await
            .unwrap_err()
            .to_string()
            .contains("nonzero"));
        assert!(storage
            .list_page(Some("missing".to_owned()), 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("unknown list continuation token"));
    }

    #[test]
    fn sidecar_keys_are_community_scoped() {
        let a = tenant(1);
        let b = tenant(2);
        let sha = "f".repeat(64);

        assert_eq!(
            MediaStorage::ctx_sidecar_key(&a, &sha),
            format!("_meta/{}/{sha}.json", a.community())
        );
        assert_ne!(
            MediaStorage::ctx_sidecar_key(&a, &sha),
            MediaStorage::ctx_sidecar_key(&b, &sha)
        );
        assert_ne!(
            MediaStorage::ctx_sidecar_key(&a, &sha),
            format!("_meta/{sha}.json")
        );
    }

    /// Mutate-bite shape for the media substrate: same CAS bytes/hash can be
    /// known in A and B, but the sidecar is the read/existence gate. If the
    /// community segment is dropped from `sidecar_key`, B's metadata overwrites
    /// A's in this map and A observes B's MIME (wrong answer, not absence).
    #[test]
    fn same_sha_sidecars_do_not_bleed_between_communities() {
        let a = tenant(1);
        let b = tenant(2);
        let sha = "a".repeat(64);
        let mut sidecars = HashMap::new();

        sidecars.insert(MediaStorage::ctx_sidecar_key(&a, &sha), "image/png");
        sidecars.insert(MediaStorage::ctx_sidecar_key(&b, &sha), "video/mp4");

        assert_eq!(
            sidecars[&MediaStorage::ctx_sidecar_key(&a, &sha)],
            "image/png"
        );
        assert_eq!(
            sidecars[&MediaStorage::ctx_sidecar_key(&b, &sha)],
            "video/mp4"
        );
    }
}

/// Metadata returned by HEAD — just enough for BUD-01 response headers.
pub struct BlobHeadMeta {
    pub size: u64,
}

/// Full blob metadata — stored as sidecar JSON in `_meta/{community}/{sha256}.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BlobMeta {
    /// Pixel dimensions ("WxH").
    pub dim: String,
    /// Blurhash string.
    pub blurhash: String,
    /// Full URL to thumbnail.
    pub thumb_url: String,
    /// File extension (e.g. "jpg").
    pub ext: String,
    /// MIME type (e.g. "image/jpeg").
    pub mime_type: String,
    /// File size in bytes.
    pub size: u64,
    /// Unix timestamp when the blob was first uploaded.
    #[serde(default)]
    pub uploaded_at: i64,
    /// Video duration in seconds. `None` for non-video blobs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
}
