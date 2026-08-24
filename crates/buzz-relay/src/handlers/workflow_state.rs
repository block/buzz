use nostr::{Event, EventBuilder, EventId, Keys, Kind, Tag};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use buzz_core::kind::KIND_WORKFLOW_STATE;
use buzz_core::{CommunityId, StoredEvent};

/// Lifecycle value carried by a relay-signed workflow state projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowStateStatus {
    /// The workflow is present and may execute when otherwise enabled.
    Active,
    /// The workflow was deleted; clients must not fall back to legacy state.
    Deleted,
}

impl WorkflowStateStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deleted => "deleted",
        }
    }
}

/// Build the relay-authored current-state projection for a workflow.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_workflow_state(
    relay_keys: &Keys,
    workflow_id: Uuid,
    channel_id: Uuid,
    owner: &[u8],
    actor: &[u8],
    request_id: EventId,
    yaml: &str,
    status: WorkflowStateStatus,
) -> anyhow::Result<Event> {
    if owner.len() != 32 || actor.len() != 32 {
        return Err(anyhow::anyhow!(
            "workflow owner and actor must be 32-byte pubkeys"
        ));
    }
    let workflow_id = workflow_id.to_string();
    let channel_id = channel_id.to_string();
    let owner = hex::encode(owner);
    let actor = hex::encode(actor);
    let request_id = request_id.to_hex();
    let tags = vec![
        Tag::parse(["d", workflow_id.as_str()])?,
        Tag::parse(["h", channel_id.as_str()])?,
        Tag::parse(["owner", owner.as_str()])?,
        Tag::parse(["actor", actor.as_str()])?,
        Tag::parse(["e", request_id.as_str(), "", "request"])?,
        Tag::parse(["status", status.as_str()])?,
    ];
    let content = match status {
        WorkflowStateStatus::Active => yaml,
        WorkflowStateStatus::Deleted => "",
    };
    EventBuilder::new(Kind::Custom(KIND_WORKFLOW_STATE as u16), content)
        .tags(tags)
        .sign_with_keys(relay_keys)
        .map_err(Into::into)
}

/// Replace the relay-authored state for one workflow inside the caller's
/// command transaction. The workflow row CAS serializes competing writers, so
/// replacement order here is the accepted request order rather than NIP-33
/// timestamps supplied by clients.
pub(crate) async fn store_workflow_state(
    tx: &mut Transaction<'_, Postgres>,
    community_id: CommunityId,
    event: &Event,
    channel_id: Uuid,
) -> anyhow::Result<StoredEvent> {
    let workflow_id = event
        .tags
        .iter()
        .find_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("d"))
                .then(|| parts.get(1).map(ToString::to_string))
                .flatten()
        })
        .ok_or_else(|| anyhow::anyhow!("workflow state is missing its d tag"))?;
    let created_at_secs = event.created_at.as_secs() as i64;
    let created_at = chrono::DateTime::from_timestamp(created_at_secs, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid workflow state timestamp"))?;
    let pubkey = event.pubkey.to_bytes();
    let signature = event.sig.serialize();
    let tags = serde_json::to_value(&event.tags)?;
    let received_at = chrono::Utc::now();

    // NIP-33 normally keys replacement by author, but this relay-owned
    // projection must keep one coordinate across relay signing-key rotation.
    // Serialize every writer for (community, kind, d) before retiring the old
    // head and inserting the new one. A hash collision only adds contention;
    // it cannot weaken correctness.
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(workflow_state_lock_key(community_id, &workflow_id))
        .execute(&mut **tx)
        .await?;

    sqlx::query(
        "UPDATE events SET deleted_at = NOW() \
         WHERE community_id = $1 AND kind = $2 AND d_tag = $3 \
         AND deleted_at IS NULL",
    )
    .bind(community_id.as_uuid())
    .bind(KIND_WORKFLOW_STATE as i32)
    .bind(&workflow_id)
    .execute(&mut **tx)
    .await?;

    sqlx::query(
        "INSERT INTO events \
         (community_id, id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id, d_tag) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(community_id.as_uuid())
    .bind(event.id.as_bytes().as_slice())
    .bind(pubkey.as_slice())
    .bind(created_at)
    .bind(KIND_WORKFLOW_STATE as i32)
    .bind(tags)
    .bind(&event.content)
    .bind(signature.as_slice())
    .bind(received_at)
    .bind(channel_id)
    .bind(&workflow_id)
    .execute(&mut **tx)
    .await?;

    Ok(StoredEvent::with_received_at(
        event.clone(),
        received_at,
        Some(channel_id),
        true,
    ))
}

fn workflow_state_lock_key(community_id: CommunityId, workflow_id: &str) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in community_id
        .as_uuid()
        .as_bytes()
        .iter()
        .chain((KIND_WORKFLOW_STATE as i32).to_le_bytes().iter())
        .chain(workflow_id.as_bytes())
    {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::Keys;
    use uuid::Uuid;

    fn tag_values(event: &nostr::Event, name: &str) -> Vec<String> {
        event
            .tags
            .iter()
            .filter_map(|tag| {
                let parts = tag.as_slice();
                (parts.first().map(String::as_str) == Some(name))
                    .then(|| parts.get(1).map(ToString::to_string))
                    .flatten()
            })
            .collect()
    }

    #[test]
    fn state_lock_coordinate_does_not_depend_on_relay_key() {
        let community = CommunityId::from_uuid(Uuid::new_v4());
        let workflow = Uuid::new_v4().to_string();
        assert_eq!(
            workflow_state_lock_key(community, &workflow),
            workflow_state_lock_key(community, &workflow)
        );
        assert_ne!(
            workflow_state_lock_key(community, &workflow),
            workflow_state_lock_key(community, &Uuid::new_v4().to_string())
        );
    }

    #[test]
    fn active_state_is_relay_authored_and_preserves_owner_and_actor() {
        let relay = Keys::generate();
        let workflow_id = Uuid::new_v4();
        let channel_id = Uuid::new_v4();
        let owner = [0x11; 32];
        let actor = [0x22; 32];
        let request = nostr::EventId::from_slice(&[0x33; 32]).expect("request id");
        let yaml = "name: managed\ntrigger:\n  on: message_posted\nsteps: []\n";

        let state = build_workflow_state(
            &relay,
            workflow_id,
            channel_id,
            &owner,
            &actor,
            request,
            yaml,
            WorkflowStateStatus::Active,
        )
        .expect("build active workflow state");

        assert_eq!(
            state.kind.as_u16(),
            buzz_core::kind::KIND_WORKFLOW_STATE as u16
        );
        assert_eq!(state.pubkey, relay.public_key());
        assert_eq!(state.content, yaml);
        assert_eq!(tag_values(&state, "d"), vec![workflow_id.to_string()]);
        assert_eq!(tag_values(&state, "h"), vec![channel_id.to_string()]);
        assert_eq!(tag_values(&state, "owner"), vec![hex::encode(owner)]);
        assert_eq!(tag_values(&state, "actor"), vec![hex::encode(actor)]);
        assert_eq!(tag_values(&state, "e"), vec![request.to_hex()]);
        assert_eq!(tag_values(&state, "status"), vec!["active".to_string()]);
    }

    #[test]
    fn deleted_state_has_empty_content_and_deleted_status() {
        let relay = Keys::generate();
        let state = build_workflow_state(
            &relay,
            Uuid::new_v4(),
            Uuid::new_v4(),
            &[0x11; 32],
            &[0x22; 32],
            nostr::EventId::from_slice(&[0x33; 32]).expect("request id"),
            "ignored",
            WorkflowStateStatus::Deleted,
        )
        .expect("build deleted workflow state");

        assert!(state.content.is_empty());
        assert_eq!(tag_values(&state, "status"), vec!["deleted".to_string()]);
    }
}
