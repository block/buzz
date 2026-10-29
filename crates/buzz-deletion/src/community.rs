//! Community submission and preparation shared by the CLI and parent workflows.

use crate::build_inventory;
use anyhow::Result;
use buzz_core::CommunityId;
use buzz_db::deletion::{DeletionRequest, DeletionStage, DeletionStore};
use buzz_db::DbError;
use buzz_media::MediaStorage;

/// Submit operator intent and freeze inventory using the existing community
/// operations. A pinned community ID makes admission create-only: a concurrent
/// existing request is returned unchanged, including blocked or aborted states.
/// Without an ID, the CLI retains its existing same-submitter preparation retry.
/// Callers must already hold operator authority; this does not approve or run deletion.
pub async fn submit_and_prepare(
    store: &DeletionStore,
    media: &MediaStorage,
    host: Option<&str>,
    requested_by: &str,
    reason: Option<&str>,
    expected_community: Option<CommunityId>,
) -> Result<DeletionRequest> {
    let request = match store
        .submit(host, requested_by, reason, expected_community)
        .await
    {
        Ok(request) => request,
        Err(error @ DbError::DeletionSafety(_)) if expected_community.is_some() => {
            // A separate read sees a concurrent submitter's committed row.
            // Database/transport failures are never converted to success.
            if let Some(community) = expected_community {
                if let Some(existing) = store.find_for_community(community).await? {
                    if host.is_none_or(|host| existing.community_host.eq_ignore_ascii_case(host)) {
                        return Ok(existing);
                    }
                }
            }
            return Err(error.into());
        }
        Err(error) => return Err(error.into()),
    };
    let inventory = build_inventory(store, media, &request, None).await?;
    match store.freeze_inventory(request.id, &inventory).await {
        Ok(request) => Ok(request),
        Err(error @ DbError::DeletionSafety(_)) if expected_community.is_some() => {
            // An operator may have progressed the child during storage listing.
            // Preserve their committed state, never reset its approval/inventory.
            let current = store.get(request.id).await?;
            if current.stage != DeletionStage::Submitted {
                Ok(current)
            } else {
                Err(error.into())
            }
        }
        Err(error) => Err(error.into()),
    }
}
