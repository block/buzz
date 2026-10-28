//! Read-state change ping: after a write commits, the actor's own live
//! connections learn which channels and threads to refetch. The event is
//! ephemeral, relay-signed and result-gated to its one `p` tag, so it never
//! reaches another reader and is never stored. Delivery is best effort: a
//! missed ping is recovered by the client's load on connect, reconnect and
//! wake.

use crate::state::AppState;
use buzz_core::{kind::KIND_READ_STATE_CHANGED, TenantContext};
use buzz_pubsub::EventTopic;
use nostr::{EventBuilder, Kind, Tag};
use serde_json::json;
use std::collections::BTreeSet;
use tracing::warn;
use uuid::Uuid;

/// What a committed batch may have changed. `unknown` outcomes are included:
/// they may have committed, and a needless refetch is harmless.
#[derive(Default)]
pub(super) struct Changes {
    pub(super) channels: BTreeSet<Uuid>,
    pub(super) threads: BTreeSet<(Uuid, String)>,
}

impl Changes {
    pub(super) fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }
}

/// Sign and deliver the ping on every relay pod; returns its event ID so the
/// writing client can recognize and skip its own echo.
pub(super) async fn send(
    state: &AppState,
    tenant: &TenantContext,
    actor: &nostr::PublicKey,
    changes: &Changes,
    revision: Option<(i64, i64)>,
) -> Option<String> {
    if changes.is_empty() {
        return None;
    }
    let content = json!({
        "read_revision_from": revision.map(|(from, _)| from),
        "read_revision": revision.map(|(_, last)| last),
        "channels": changes.channels,
        "threads": changes.threads.iter()
            .map(|(channel_id, root_id)| json!({"channel_id": channel_id, "root_id": root_id}))
            .collect::<Vec<_>>(),
    })
    .to_string();
    let p = Tag::parse(["p", &actor.to_hex()]).ok()?;
    // Distinct ID per ping even for identical content in the same second.
    let nonce = Tag::parse(["nonce", &hex::encode(rand::random::<[u8; 16]>())]).ok()?;
    let event = match EventBuilder::new(Kind::Custom(KIND_READ_STATE_CHANGED as u16), content)
        .tags([p, nonce])
        .sign_with_keys(&state.relay_keypair)
    {
        Ok(event) => event,
        Err(error) => {
            warn!(%error, "read-state ping: signing failed");
            return None;
        }
    };
    // Same order as relay-signed membership notices: Redis first so other
    // pods' connections receive it, then this pod's, deduplicated by ID.
    state.mark_local_event(tenant.community(), &event.id);
    if let Err(error) = state
        .pubsub
        .publish_event(tenant, EventTopic::Global, &event)
        .await
    {
        state
            .local_event_ids
            .invalidate(&(tenant.community(), event.id.to_bytes()));
        warn!(%error, "read-state ping: Redis publish failed");
    }
    let stored = buzz_core::event::StoredEvent::new(event.clone(), None);
    crate::handlers::event::fan_out_event_to_local_subscribers(state, tenant.community(), &stored)
        .await;
    Some(event.id.to_hex())
}
