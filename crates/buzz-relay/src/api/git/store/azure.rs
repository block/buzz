//! Azure Blob adaptation for Git-on-object-storage.

use buzz_azure_storage::{AzureBlobStore, BlobVersion, ConditionalWrite};
use bytes::Bytes;

use super::{CasOutcome, ETag, Precond, StoreError};

/// Azure operations translated into the Git store's backend-neutral contract.
#[derive(Clone)]
pub(super) struct AzureGitStore {
    store: AzureBlobStore,
}

impl AzureGitStore {
    /// Construct through the Azure credential environment.
    pub(super) fn from_env(account: &str, container: &str) -> Result<Self, StoreError> {
        Ok(Self {
            store: AzureBlobStore::from_env(account, container)?,
        })
    }

    /// Construct against Azurite for conformance coverage.
    #[cfg(test)]
    pub(super) fn for_azurite(container: &str) -> Result<Self, StoreError> {
        Ok(Self {
            store: AzureBlobStore::for_azurite(container)?,
        })
    }

    /// Create an immutable object, treating a pre-existing key as idempotent success.
    pub(super) async fn create_idempotent(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(), StoreError> {
        self.store
            .create(key, Bytes::copy_from_slice(bytes), content_type)
            .await?;
        Ok(())
    }

    /// Read an object and translate Azure not-found into the Git contract.
    pub(super) async fn get(&self, key: &str) -> Result<Bytes, StoreError> {
        match self.store.get(key).await {
            Ok(object) => Ok(object.bytes),
            Err(error) if error.is_not_found() => Err(StoreError::NotFound(key.into())),
            Err(error) => Err(StoreError::AzureBackend(error)),
        }
    }

    /// Return object size, or `None` when the key is absent.
    pub(super) async fn size(&self, key: &str) -> Result<Option<u64>, StoreError> {
        Ok(self.store.head(key).await?.map(|metadata| metadata.size))
    }

    /// Read pointer bytes and ETag from the same Azure response.
    pub(super) async fn get_pointer(&self, key: &str) -> Result<Option<(ETag, Bytes)>, StoreError> {
        match self.store.get(key).await {
            Ok(object) => Ok(Some((ETag(object.version.etag), object.bytes))),
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(StoreError::AzureBackend(error)),
        }
    }

    /// Apply the Git pointer precondition and translate Azure CAS outcomes.
    pub(super) async fn put_pointer(
        &self,
        key: &str,
        body: &[u8],
        precond: Precond,
    ) -> Result<CasOutcome, StoreError> {
        let result = match precond {
            Precond::IfNoneMatchStar => {
                self.store
                    .create(key, Bytes::copy_from_slice(body), "application/json")
                    .await?
            }
            Precond::IfMatch(ETag(etag)) => {
                self.store
                    .update(
                        key,
                        Bytes::copy_from_slice(body),
                        "application/json",
                        BlobVersion {
                            etag,
                            version: None,
                        },
                    )
                    .await?
            }
        };
        Ok(match result {
            ConditionalWrite::Won(version) => CasOutcome::Won(ETag(version.etag)),
            ConditionalWrite::LostRace => CasOutcome::LostRace,
        })
    }

    /// Expose create-only result classification to the conformance race probe.
    pub(super) async fn put_immutable_raw(
        &self,
        key: &str,
        bytes: &[u8],
    ) -> Result<u16, StoreError> {
        Ok(
            match self
                .store
                .create(
                    key,
                    Bytes::copy_from_slice(bytes),
                    "application/octet-stream",
                )
                .await?
            {
                ConditionalWrite::Won(_) => 201,
                ConditionalWrite::LostRace => 412,
            },
        )
    }

    /// Delete a probe object while treating absence as success.
    pub(super) async fn delete_if_exists(&self, key: &str) -> Result<(), StoreError> {
        self.store.delete_if_exists(key).await?;
        Ok(())
    }
}
