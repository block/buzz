//! Azure Blob adaptation for the media storage contract.

use std::path::Path;

use buzz_azure_storage::AzureBlobStore;
use bytes::Bytes;

use crate::bucket_index::Page;
use crate::error::MediaError;

use super::{BulkDeleteOutcome, ByteStream};

/// Azure implementation details kept outside the upstream S3 media semantics.
#[derive(Clone)]
pub(super) struct AzureMediaStore {
    store: AzureBlobStore,
}

impl AzureMediaStore {
    /// Construct through the Azure credential environment.
    pub(super) fn from_env(account: &str, container: &str) -> Result<Self, MediaError> {
        Ok(Self {
            store: AzureBlobStore::from_env(account, container)?,
        })
    }

    /// Store an in-memory object with its content type.
    pub(super) async fn put(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(), MediaError> {
        self.store
            .put(key, Bytes::copy_from_slice(bytes), content_type)
            .await?;
        Ok(())
    }

    /// Stream a file into Azure without buffering the complete object.
    pub(super) async fn put_file(
        &self,
        key: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<(), MediaError> {
        self.store.put_file(key, path, content_type).await?;
        Ok(())
    }

    /// Read an object and translate Azure errors into the media contract.
    pub(super) async fn get(&self, key: &str) -> Result<Vec<u8>, MediaError> {
        Ok(self.store.get(key).await?.bytes.to_vec())
    }

    /// Read the media contract's inclusive byte range from Azure.
    pub(super) async fn get_range_inclusive(
        &self,
        key: &str,
        start: u64,
        end: u64,
    ) -> Result<Vec<u8>, MediaError> {
        let end_exclusive = end
            .checked_add(1)
            .ok_or_else(|| MediaError::StorageError("invalid inclusive range end".to_string()))?;
        Ok(self
            .store
            .get_range(key, start..end_exclusive)
            .await?
            .to_vec())
    }

    /// Stream an object while translating chunk failures into media errors.
    pub(super) async fn get_stream(&self, key: &str) -> Result<ByteStream, MediaError> {
        let stream = self.store.get_stream(key).await?;
        Ok(Box::pin(futures_util::StreamExt::map(stream, |chunk| {
            chunk.map_err(MediaError::from)
        })))
    }

    /// Return whether an object exists.
    pub(super) async fn exists(&self, key: &str) -> Result<bool, MediaError> {
        Ok(self.store.head(key).await?.is_some())
    }

    /// Delete an object while treating absence as success.
    pub(super) async fn delete_if_exists(&self, key: &str) -> Result<(), MediaError> {
        self.store.delete_if_exists(key).await?;
        Ok(())
    }

    /// Return object size, or `None` when absent.
    pub(super) async fn size(&self, key: &str) -> Result<Option<u64>, MediaError> {
        Ok(self.store.head(key).await?.map(|metadata| metadata.size))
    }

    /// Probe whether Azure Blob versioning is enabled, then remove the probe.
    pub(super) async fn versioning_detected(&self, key: &str) -> Result<bool, MediaError> {
        self.store
            .put(
                key,
                Bytes::from_static(b"buzz deletion versioning probe"),
                "text/plain",
            )
            .await?;
        let inspected = self.store.get(key).await;
        let removed = self.store.delete_if_exists(key).await;
        let versioned = inspected?.version.version.is_some();
        removed?;
        Ok(versioned)
    }

    /// Delete a bounded manifest chunk while preserving per-key outcomes.
    pub(super) async fn delete_objects(
        &self,
        keys: &[String],
    ) -> Result<BulkDeleteOutcome, MediaError> {
        let mut outcome = BulkDeleteOutcome::default();
        for key in keys {
            match self.store.delete_if_exists(key).await {
                Ok(()) => outcome.deleted += 1,
                Err(error) => {
                    outcome
                        .failed
                        .push((key.clone(), "AzureDelete".to_string(), error.to_string()))
                }
            }
        }
        Ok(outcome)
    }

    /// Return one bounded Azure listing page under an exact key prefix.
    pub(super) async fn list_prefix_page(
        &self,
        prefix: &str,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<Page, MediaError> {
        let page = self
            .store
            .list_page(
                (!prefix.is_empty()).then_some(prefix),
                continuation_token,
                max_keys,
            )
            .await?;
        Ok(Page {
            is_truncated: page.continuation_token.is_some(),
            objects: page
                .objects
                .into_iter()
                .map(|object| (object.key, object.size))
                .collect(),
            next_continuation_token: page.continuation_token,
        })
    }
}
