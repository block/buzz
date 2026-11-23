//! The record of governed reads. Every door that shows a governed
//! repository's content or state picks what it will show, then records
//! whom it is showing it to, then shows exactly that
//! (`ReadsAreGovernedAndRecorded`, docs/spec/GitGovernance.tla). A row
//! proves an authorized read attempt, not delivery: a read refused by
//! authorization leaves no row, while one that fails after recording
//! (say, a malformed request) keeps it. Reads of an ungoverned repository
//! are not recorded, and since governance only grows, a repository found
//! ungoverned was ungoverned when the door picked its state. A governed
//! read whose record cannot be written is not served.
use buzz_audit::{AuditAction, AuditError, AuditService, NewAuditEntry};
use buzz_core::{governance::RepoCoordinate, CommunityId};

/// What a door shows.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Door {
    /// The repository's refs and objects.
    Fetch,
    /// Its governance state: policy in force, receipts, documents.
    State,
}

/// Record that `reader` was shown `repo` through `door`, if it is governed.
pub(crate) async fn record(
    db: &buzz_db::Db,
    community: CommunityId,
    repo: &RepoCoordinate,
    reader: &[u8],
    door: Door,
) -> Result<(), AuditError> {
    let mut tx = db.begin_event_write_transaction(community).await?;
    if !buzz_db::governance::governed(&mut tx, repo).await? {
        tx.rollback().await?;
        #[cfg(test)]
        crate::nip_fi_test_hooks::after_git_read_decided(community).await;
        return Ok(());
    }
    let door = match door {
        Door::Fetch => "fetch",
        Door::State => "state",
    };
    let entry = NewAuditEntry {
        community_id: community,
        action: AuditAction::GitRead,
        actor_pubkey: Some(reader.to_vec()),
        object_id: Some(repo.to_string()),
        detail: serde_json::json!({ "door": door }),
    };
    AuditService::log_in_transaction(&mut tx, entry).await?;
    tx.commit().await?;
    Ok(())
}
