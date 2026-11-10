use buzz_core::CommunityId;
use buzz_db::Db;
use buzz_media::{storage::UploadAdmission, MediaError};
use std::{future::Future, pin::Pin};

/// Relay upload admission shared by every media read mode. Recording before
/// storage prevents an upload from becoming invisible between storage and SQL.
pub struct UploadClaims(pub Db);

impl UploadAdmission for UploadClaims {
    fn admit<'a>(
        &'a self,
        community: CommunityId,
        hash: &'a str,
        uploader: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), MediaError>> + Send + 'a>> {
        Box::pin(async move {
            self.0
                .begin_media_upload(community, hash, uploader)
                .await
                .map_err(|_| MediaError::AuthorizationUnavailable)
        })
    }
}
