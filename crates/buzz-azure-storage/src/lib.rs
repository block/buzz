//! Azure Blob Storage primitives required by Buzz media and git storage.
//!
//! The adapter deliberately exposes conditional writes as a semantic outcome:
//! losing an optimistic-concurrency race is expected, not a transport error.
//! Production construction uses the Azure credential environment, which lets
//! AKS workload identity provide short-lived credentials without storage keys.

#![deny(unsafe_code)]

use std::ops::Range;
use std::path::Path as FilePath;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;
use futures_core::Stream;
use futures_util::TryStreamExt;
use object_store::azure::{MicrosoftAzure, MicrosoftAzureBuilder};
use object_store::list::{PaginatedListOptions, PaginatedListStore};
use object_store::path::Path;
use object_store::{
    Attribute, Attributes, Error as ObjectStoreError, ObjectMeta, ObjectStore, ObjectStoreExt,
    PutMode, PutMultipartOptions, PutOptions, PutResult, UpdateVersion, WriteMultipart,
};
use tokio::io::AsyncReadExt;

/// A streaming Azure Blob response suitable for an HTTP response body.
pub type ByteStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, AzureStorageError>> + Send + 'static>>;

/// A blob body and the exact version metadata observed by the same GET.
#[derive(Debug)]
pub struct VersionedObject {
    /// Object bytes.
    pub bytes: Bytes,
    /// Version to supply to a subsequent compare-and-swap write.
    pub version: BlobVersion,
    /// Object attributes returned by Azure, including content type when set.
    pub attributes: Attributes,
}

/// Result of an atomic conditional write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionalWrite {
    /// The write committed and returned the new object version.
    Won(BlobVersion),
    /// Another writer won the precondition race.
    LostRace,
}

/// Opaque Azure object version suitable for a later compare-and-swap write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobVersion {
    /// Strong ETag returned by the same read or successful write.
    pub etag: String,
    /// Optional Azure version identifier when account versioning is enabled.
    pub version: Option<String>,
}

/// Backend-neutral object metadata used by Buzz media and sweep paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobObjectMetadata {
    /// Full object key within the configured container.
    pub key: String,
    /// Object size in bytes.
    pub size: u64,
    /// Strong ETag when returned by Azure.
    pub etag: Option<String>,
}

/// One bounded listing page and an opaque continuation token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobListPage {
    /// Objects returned in this page.
    pub objects: Vec<BlobObjectMetadata>,
    /// Token to pass to the next request, or `None` for the final page.
    pub continuation_token: Option<String>,
}

/// Azure Blob adapter failures.
#[derive(Debug, thiserror::Error)]
pub enum AzureStorageError {
    /// Object key could not be represented as an Azure blob path.
    #[error("invalid Azure Blob Storage object key: {0}")]
    InvalidPath(#[from] object_store::path::Error),
    /// Azure or transport failure.
    #[error("Azure Blob Storage error: {0}")]
    Backend(#[from] ObjectStoreError),
    /// A successful write or read omitted the ETag needed for Buzz CAS.
    #[error("Azure Blob Storage response for '{key}' did not include an ETag")]
    MissingEtag {
        /// Object key whose response was incomplete.
        key: String,
    },
}

impl AzureStorageError {
    /// Whether Azure reported that the requested object does not exist.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Backend(ObjectStoreError::NotFound { .. }))
    }
}

/// Azure Blob Storage implementation of the object operations Buzz requires.
#[derive(Clone, Debug)]
pub struct AzureBlobStore {
    inner: Arc<MicrosoftAzure>,
}

impl AzureBlobStore {
    /// Build a production client from the Azure credential environment.
    ///
    /// In AKS, set `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, and
    /// `AZURE_FEDERATED_TOKEN_FILE`; `object_store` will use workload identity.
    /// A managed identity is used when no more-specific credential is present.
    pub fn from_env(account: &str, container: &str) -> Result<Self, AzureStorageError> {
        let inner = MicrosoftAzureBuilder::from_env()
            .with_account(account)
            .with_container_name(container)
            .build()?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Build a client for the local Azurite emulator.
    pub fn for_azurite(container: &str) -> Result<Self, AzureStorageError> {
        let inner = MicrosoftAzureBuilder::new()
            .with_container_name(container)
            .with_use_emulator(true)
            .build()?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Atomically create an object only when its key is absent.
    pub async fn create(
        &self,
        key: &str,
        bytes: Bytes,
        content_type: &str,
    ) -> Result<ConditionalWrite, AzureStorageError> {
        self.conditional_put(key, bytes, content_type, PutMode::Create)
            .await
    }

    /// Atomically replace an object only when its version still matches.
    pub async fn update(
        &self,
        key: &str,
        bytes: Bytes,
        content_type: &str,
        version: BlobVersion,
    ) -> Result<ConditionalWrite, AzureStorageError> {
        self.conditional_put(key, bytes, content_type, PutMode::Update(version.into()))
            .await
    }

    /// Put an object, replacing an existing value when present.
    pub async fn put(
        &self,
        key: &str,
        bytes: Bytes,
        content_type: &str,
    ) -> Result<BlobVersion, AzureStorageError> {
        let path = object_path(key)?;
        let result = self
            .inner
            .put_opts(
                &path,
                bytes.into(),
                put_options(content_type, PutMode::Overwrite),
            )
            .await?;
        require_etag(key, result)
    }

    /// Stream a file to Azure using bounded multipart buffering.
    pub async fn put_file(
        &self,
        key: &str,
        file_path: &FilePath,
        content_type: &str,
    ) -> Result<BlobVersion, AzureStorageError> {
        const READ_BUFFER_BYTES: usize = 1024 * 1024;
        const UPLOAD_CHUNK_BYTES: usize = 8 * 1024 * 1024;
        const MAX_IN_FLIGHT_PARTS: usize = 2;

        let path = object_path(key)?;
        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, content_type.to_string().into());
        let upload = self
            .inner
            .put_multipart_opts(
                &path,
                PutMultipartOptions {
                    attributes,
                    ..Default::default()
                },
            )
            .await?;
        let mut writer = WriteMultipart::new_with_chunk_size(upload, UPLOAD_CHUNK_BYTES);
        let mut file =
            tokio::fs::File::open(file_path)
                .await
                .map_err(|source| ObjectStoreError::Generic {
                    store: "MicrosoftAzure",
                    source: Box::new(source),
                })?;
        let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
        loop {
            let read =
                file.read(&mut buffer)
                    .await
                    .map_err(|source| ObjectStoreError::Generic {
                        store: "MicrosoftAzure",
                        source: Box::new(source),
                    })?;
            if read == 0 {
                break;
            }
            writer.wait_for_capacity(MAX_IN_FLIGHT_PARTS).await?;
            writer.write(&buffer[..read]);
        }
        let result = writer.finish().await?;
        require_etag(key, result)
    }

    /// Read an object's bytes and CAS version from one GET response.
    pub async fn get(&self, key: &str) -> Result<VersionedObject, AzureStorageError> {
        let path = object_path(key)?;
        let result = self.inner.get(&path).await?;
        let version = version_from_meta(key, &result.meta)?;
        let attributes = result.attributes.clone();
        let bytes = result.bytes().await?;
        Ok(VersionedObject {
            bytes,
            version,
            attributes,
        })
    }

    /// Stream an object's bytes without buffering the full body.
    pub async fn get_stream(&self, key: &str) -> Result<ByteStream, AzureStorageError> {
        let path = object_path(key)?;
        let result = self.inner.get(&path).await?;
        Ok(Box::pin(
            result.into_stream().map_err(AzureStorageError::from),
        ))
    }

    /// Read a half-open byte range from an object.
    pub async fn get_range(
        &self,
        key: &str,
        range: Range<u64>,
    ) -> Result<Bytes, AzureStorageError> {
        let path = object_path(key)?;
        Ok(self.inner.get_range(&path, range).await?)
    }

    /// Return object metadata, or `None` when the key is absent.
    pub async fn head(&self, key: &str) -> Result<Option<BlobObjectMetadata>, AzureStorageError> {
        let path = object_path(key)?;
        match self.inner.head(&path).await {
            Ok(meta) => Ok(Some(metadata(meta))),
            Err(ObjectStoreError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Delete an object. Azure treats deleting an absent blob as not found.
    pub async fn delete(&self, key: &str) -> Result<(), AzureStorageError> {
        let path = object_path(key)?;
        self.inner.delete(&path).await?;
        Ok(())
    }

    /// Delete an object while treating an absent key as idempotent success.
    pub async fn delete_if_exists(&self, key: &str) -> Result<(), AzureStorageError> {
        match self.delete(key).await {
            Ok(()) | Err(AzureStorageError::Backend(ObjectStoreError::NotFound { .. })) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// List all objects under a prefix, following Azure continuation pages.
    pub async fn list_prefix(
        &self,
        prefix: &str,
    ) -> Result<Vec<BlobObjectMetadata>, AzureStorageError> {
        let path = object_path(prefix)?;
        Ok(self
            .inner
            .list(Some(&path))
            .map_ok(metadata)
            .try_collect::<Vec<_>>()
            .await?)
    }

    /// List one bounded Azure page using Azure's native continuation token.
    pub async fn list_page(
        &self,
        prefix: Option<&str>,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<BlobListPage, AzureStorageError> {
        let page = self
            .inner
            .list_paginated(
                prefix,
                PaginatedListOptions {
                    max_keys: Some(max_keys),
                    page_token: continuation_token,
                    ..Default::default()
                },
            )
            .await?;
        Ok(BlobListPage {
            objects: page.result.objects.into_iter().map(metadata).collect(),
            continuation_token: page.page_token,
        })
    }

    async fn conditional_put(
        &self,
        key: &str,
        bytes: Bytes,
        content_type: &str,
        mode: PutMode,
    ) -> Result<ConditionalWrite, AzureStorageError> {
        let path = object_path(key)?;
        match self
            .inner
            .put_opts(&path, bytes.into(), put_options(content_type, mode))
            .await
        {
            Ok(result) => Ok(ConditionalWrite::Won(require_etag(key, result)?)),
            Err(ObjectStoreError::AlreadyExists { .. } | ObjectStoreError::Precondition { .. }) => {
                Ok(ConditionalWrite::LostRace)
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn object_path(key: &str) -> Result<Path, AzureStorageError> {
    Ok(Path::parse(key)?)
}

fn put_options(content_type: &str, mode: PutMode) -> PutOptions {
    let mut attributes = Attributes::new();
    attributes.insert(Attribute::ContentType, content_type.to_string().into());
    PutOptions {
        mode,
        attributes,
        ..Default::default()
    }
}

fn require_etag(key: &str, result: PutResult) -> Result<BlobVersion, AzureStorageError> {
    let etag = result.e_tag.ok_or_else(|| AzureStorageError::MissingEtag {
        key: key.to_string(),
    })?;
    Ok(BlobVersion {
        etag,
        version: result.version,
    })
}

fn version_from_meta(key: &str, meta: &ObjectMeta) -> Result<BlobVersion, AzureStorageError> {
    let etag = meta
        .e_tag
        .clone()
        .ok_or_else(|| AzureStorageError::MissingEtag {
            key: key.to_string(),
        })?;
    Ok(BlobVersion {
        etag,
        version: meta.version.clone(),
    })
}

fn metadata(meta: ObjectMeta) -> BlobObjectMetadata {
    BlobObjectMetadata {
        key: meta.location.to_string(),
        size: meta.size,
        etag: meta.e_tag,
    }
}

impl From<BlobVersion> for UpdateVersion {
    fn from(value: BlobVersion) -> Self {
        Self {
            e_tag: Some(value.etag),
            version: value.version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn retries_an_azure_throttle_response_before_surfacing_an_error() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind throttle test server");
        let address = listener.local_addr().expect("throttle test address");
        let requests = Arc::new(AtomicUsize::new(0));
        let observed_requests = Arc::clone(&requests);
        let server = tokio::spawn(async move {
            for expected_request in 1..=2 {
                let (mut stream, _) = listener.accept().await.expect("accept request");
                let mut request = [0_u8; 4096];
                let read = stream.read(&mut request).await.expect("read request");
                assert!(
                    String::from_utf8_lossy(&request[..read]).starts_with("HEAD "),
                    "adapter should retry the original Azure metadata operation"
                );
                observed_requests.fetch_add(1, Ordering::SeqCst);
                let response = if expected_request == 1 {
                    "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                };
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
        });
        let inner = MicrosoftAzureBuilder::new()
            .with_account("buzzthrottletest")
            .with_container_name("buzz-conformance")
            .with_endpoint(format!("http://{address}"))
            .with_allow_http(true)
            .with_skip_signature(true)
            .build()
            .expect("build unsigned loopback Azure client");
        let store = AzureBlobStore {
            inner: Arc::new(inner),
        };

        let object = store
            .head("throttle/probe")
            .await
            .expect("429 should be retried and the second response should be classified");
        assert!(object.is_none());
        server.await.expect("throttle test server completes");
        assert_eq!(requests.load(Ordering::SeqCst), 2);
    }
}
