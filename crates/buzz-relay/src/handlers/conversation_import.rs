//! NIP-CI conversation import: session control events (kinds 42110 to 42113)
//! and the delivery of imported history and session snapshots.
//!
//! Control events and imported events reach this module only after ingest's
//! channel gates (token scope, membership, archived channel) have passed.

use std::sync::Arc;

use buzz_core::import::{
    self, reason, ImportCompleteContent, ImportConsentContent, ImportProposalContent,
    ImportWithdrawContent, ADMIN_CONSENT_SOURCE_USER, IMPORT_PAYLOAD_VERSION, MANIFEST_TAG,
    MAX_PROPOSAL_CONTENT_BYTES, MAX_WITHDRAW_REASON_CHARS,
};
use buzz_core::kind::{
    KIND_IMPORT_COMPLETE, KIND_IMPORT_CONSENT, KIND_IMPORT_PROPOSAL, KIND_IMPORT_WITHDRAW,
};
use buzz_core::{StoredEvent, TenantContext};
use buzz_db::import_session::ImportControlOutcome;
use nostr::Event;
use uuid::Uuid;

use super::ingest::{IngestError, IngestResult};
use crate::state::AppState;

/// Whether an agent-policy row marks its user as an agent: a user with an
/// `agent_owner_pubkey`. Live fan-out and every pull path share this test.
pub(crate) fn is_agent(policy: Option<&buzz_db::user::AgentChannelPolicy>) -> bool {
    policy.is_some_and(|(_, owner)| owner.is_some())
}

/// Hides NIP-CI imported events from a reader that is an agent, on pull
/// paths (REQ, search, HTTP query and the bridge reads). The agent lookup
/// runs once, on the first imported event, and a failed lookup hides
/// imported events.
pub(crate) struct ImportReadGate<'a> {
    state: &'a AppState,
    community: buzz_core::CommunityId,
    reader: &'a [u8],
    hides: Option<bool>,
}

impl<'a> ImportReadGate<'a> {
    pub(crate) fn new(
        state: &'a AppState,
        community: buzz_core::CommunityId,
        reader: &'a [u8],
    ) -> Self {
        Self {
            state,
            community,
            reader,
            hides: None,
        }
    }

    /// Whether `event` must be left out of this reader's results.
    pub(crate) async fn hides(&mut self, event: &Event) -> bool {
        self.hides_if(import::is_import_event(event)).await
    }

    /// [`ImportReadGate::hides`] for an event already serialized to JSON.
    pub(crate) async fn hides_json(&mut self, event: &serde_json::Value) -> bool {
        let is_import = event["tags"].as_array().is_some_and(|tags| {
            tags.iter().any(|tag| {
                tag.get(0).and_then(serde_json::Value::as_str) == Some(import::IMPORT_TAG)
            })
        });
        self.hides_if(is_import).await
    }

    /// Keep the JSON events this reader may see, in order.
    pub(crate) async fn retain_json(
        &mut self,
        events: Vec<serde_json::Value>,
    ) -> Vec<serde_json::Value> {
        let mut kept = Vec::with_capacity(events.len());
        for event in events {
            if !self.hides_json(&event).await {
                kept.push(event);
            }
        }
        kept
    }

    async fn hides_if(&mut self, is_import: bool) -> bool {
        if !is_import {
            return false;
        }
        if let Some(hides) = self.hides {
            return hides;
        }
        let hides = match self
            .state
            .db
            .get_agent_channel_policies(self.community, &[self.reader.to_vec()])
            .await
        {
            Ok(policies) => is_agent(policies.get(self.reader)),
            Err(e) => {
                tracing::warn!("import read gate: agent lookup failed: {e}");
                true
            }
        };
        self.hides = Some(hides);
        hides
    }
}

fn malformed(detail: &str) -> IngestError {
    IngestError::Rejected(format!("{}{detail}", reason::MALFORMED_PREFIX))
}

/// Client-facing reason for a failed import session write.
const STORAGE_FAILED: &str = "error: import session storage failed";

/// Client-facing reason for a failed import session read.
const READ_FAILED: &str = "error: import session read failed";

/// Map a failed import session write to a fixed client reason; the database
/// detail goes to the log only.
pub(crate) fn storage_failed(e: buzz_db::DbError) -> IngestError {
    tracing::warn!(error = %e, "import session write failed");
    IngestError::Internal(STORAGE_FAILED.into())
}

/// Map a failed import session read to a fixed client reason; the database
/// detail goes to the log only.
pub(crate) fn read_failed(e: buzz_db::DbError) -> IngestError {
    tracing::warn!(error = %e, "import session read failed");
    IngestError::Internal(READ_FAILED.into())
}

/// The value of the single tag named `name`. Missing or repeated tags are
/// malformed.
fn single_tag<'a>(event: &'a Event, name: &str) -> Result<&'a str, IngestError> {
    let mut values = event.tags.iter().filter_map(|tag| {
        let slice = tag.as_slice();
        (slice.first().map(String::as_str) == Some(name)).then(|| slice.get(1))
    });
    match (values.next(), values.next()) {
        (Some(Some(value)), None) => Ok(value.as_str()),
        _ => Err(malformed(&format!("expected exactly one {name} tag"))),
    }
}

/// Proposal id bytes from the single `e` tag of a 42111 to 42113.
fn proposal_id(event: &Event) -> Result<Vec<u8>, IngestError> {
    let id = single_tag(event, "e")?;
    if !import::is_lower_hex_64(id) {
        return Err(malformed("e tag must be a 64 lowercase hex proposal id"));
    }
    hex::decode(id).map_err(|_| malformed("e tag must be a 64 lowercase hex proposal id"))
}

fn decode<T: serde::de::DeserializeOwned>(event: &Event, what: &str) -> Result<T, IngestError> {
    serde_json::from_str(&event.content).map_err(|_| malformed(&format!("{what} content")))
}

fn check_version(v: u32) -> Result<(), IngestError> {
    if v == IMPORT_PAYLOAD_VERSION {
        Ok(())
    } else {
        Err(malformed("unsupported version"))
    }
}

/// Apply one import control event to its session in `channel_id`.
pub(crate) async fn accept_control(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    kind: u32,
    channel_id: Uuid,
) -> Result<IngestResult, IngestError> {
    let community = tenant.community();
    let keys = &state.relay_keypair;
    if let Some(name) = import::first_unexpected_control_tag(event, kind) {
        return Err(malformed(&format!("unexpected tag {name}")));
    }
    let outcome = match kind {
        KIND_IMPORT_PROPOSAL => {
            if event.content.len() > MAX_PROPOSAL_CONTENT_BYTES {
                return Err(malformed("proposal content exceeds 16 KiB"));
            }
            let content: ImportProposalContent = decode(event, "proposal")?;
            let now = chrono::Utc::now().timestamp();
            import::validate_proposal_content(&content, now, state.config.import_limits)
                .map_err(|rejection| IngestError::Rejected(rejection.reason()))?;
            import::validate_proposal_tags(event, &content).map_err(malformed)?;
            let expires_at = import::cap_expires_at(
                content.expires_at,
                now,
                state.config.import_session_max_days,
            );
            state
                .db
                .create_import_session(
                    community,
                    event,
                    &content,
                    channel_id,
                    expires_at,
                    state.config.import_limits,
                    keys,
                )
                .await
                .map_err(storage_failed)?
        }
        KIND_IMPORT_CONSENT => {
            let proposal = proposal_id(event)?;
            let manifest = single_tag(event, MANIFEST_TAG)?;
            let content: ImportConsentContent = decode(event, "consent")?;
            check_version(content.v)?;
            if content.source_user != ADMIN_CONSENT_SOURCE_USER
                && !import::is_lower_hex_64(&content.source_user)
            {
                return Err(malformed("source_user must be 64 lowercase hex or admin"));
            }
            state
                .db
                .record_import_consent(
                    community,
                    event,
                    &proposal,
                    channel_id,
                    manifest,
                    content.decision,
                    &content.source_user,
                    keys,
                )
                .await
                .map_err(storage_failed)?
        }
        KIND_IMPORT_WITHDRAW => {
            let proposal = proposal_id(event)?;
            if !event.content.is_empty() {
                let content: ImportWithdrawContent = decode(event, "withdraw")?;
                check_version(content.v)?;
                if content
                    .reason
                    .as_deref()
                    .is_some_and(|r| r.chars().count() > MAX_WITHDRAW_REASON_CHARS)
                {
                    return Err(malformed("withdraw reason exceeds 200 characters"));
                }
            }
            state
                .db
                .withdraw_import_session(community, event, &proposal, channel_id, keys)
                .await
                .map_err(storage_failed)?
        }
        KIND_IMPORT_COMPLETE => {
            let proposal = proposal_id(event)?;
            let content: ImportCompleteContent = decode(event, "complete")?;
            check_version(content.v)?;
            state
                .db
                .complete_import_participant(community, event, &proposal, channel_id, keys)
                .await
                .map_err(storage_failed)?
        }
        _ => return Err(IngestError::Rejected(reason::KIND_NOT_ALLOWED.into())),
    };

    let event_id = event.id.to_hex();
    match outcome {
        ImportControlOutcome::Rejected(reason) => Err(IngestError::Rejected(reason.into())),
        ImportControlOutcome::RejectedAfterTransition { reason, snapshot } => {
            publish_snapshot(tenant, state, &snapshot).await;
            Err(IngestError::Rejected(reason.into()))
        }
        ImportControlOutcome::Duplicate => Ok(IngestResult {
            event_id,
            accepted: true,
            message: "duplicate:".into(),
        }),
        ImportControlOutcome::Applied {
            stored_event,
            snapshot,
        } => {
            let pubkey_hex = event.pubkey.to_hex();
            super::event::dispatch_persistent_event(
                tenant,
                state,
                &stored_event,
                kind,
                &pubkey_hex,
                None,
            )
            .await;
            publish_snapshot(tenant, state, &snapshot).await;
            Ok(IngestResult {
                event_id,
                accepted: true,
                message: String::new(),
            })
        }
    }
}

/// Deliver a relay-signed kind:30623 snapshot to live channel subscribers.
///
/// The snapshot is stored with the session's channel, so reads and fan-out
/// apply the channel's own access gate. Like other relay-maintained state it
/// carries no audit or workflow side effects.
pub(crate) async fn publish_snapshot(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    snapshot: &StoredEvent,
) {
    let Some(channel) = snapshot.channel_id else {
        return;
    };
    super::artifact_sync::publish_reliably(
        state,
        tenant,
        buzz_pubsub::EventTopic::Channel(channel),
        snapshot,
    )
    .await;
    super::event::fan_out_event_to_local_subscribers(state, tenant.community(), snapshot).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn event_with_tags(tags: Vec<Vec<&str>>) -> Event {
        let tags = tags
            .into_iter()
            .map(|t| Tag::parse(t).expect("tag"))
            .collect::<Vec<_>>();
        EventBuilder::new(Kind::Custom(42111), "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    #[test]
    fn proposal_id_requires_one_lowercase_hex_e_tag() {
        let id = "ab".repeat(32);
        assert_eq!(
            proposal_id(&event_with_tags(vec![vec!["e", &id]])).ok(),
            Some(hex::decode(&id).expect("hex"))
        );
        for tags in [
            vec![],
            vec![vec!["e", &id], vec!["e", &id]],
            vec![vec!["e", "AB"]],
        ] {
            assert!(proposal_id(&event_with_tags(tags)).is_err());
        }
        let upper = id.to_uppercase();
        assert!(proposal_id(&event_with_tags(vec![vec!["e", &upper]])).is_err());
    }

    #[test]
    fn malformed_reasons_carry_the_import_prefix() {
        let IngestError::Rejected(message) = malformed("detail") else {
            panic!("expected a rejection");
        };
        assert_eq!(message, "invalid: import event malformed: detail");
    }
}
