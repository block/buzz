//! Shared native-client fold for reserved binding bootstrap/status frames.

use std::fmt;

use nostr::{Event, EventId, PublicKey};
use serde::Serialize;
use serde_json::Value;

use crate::{
    client_binding_bootstrap::{
        validate_client_binding_bootstrap_event, ClientBindingEpoch,
        CLIENT_BINDING_BOOTSTRAP_SUB_ID, CLIENT_BINDING_STATUS_SUB_ID,
    },
    client_binding_status::{ClientBindingStatusTracker, ClientBindingStatusUpdate},
    verify_event,
};

/// Current-only data permitted to cross a native client IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentProjection {
    /// Canonical lowercase Nostr event-author public key.
    pub event_author_pubkey: String,
    /// Exclusive Unix-seconds freshness bound.
    pub fresh_until: u64,
}

/// One change produced by the serialized native fold.
pub enum ProjectionUpdate {
    /// Reserved input did not change the trusted projection.
    Unchanged,
    /// Clear any existing projection.
    Clear,
    /// Replace the projection with a fresh current value.
    Current(CurrentProjection),
}

struct ReservedEvent {
    event: Result<Event, ()>,
    exact_outer_shape: bool,
}

enum ReservedFrame {
    Bootstrap(ReservedEvent),
    Status(ReservedEvent),
}

/// Connection-scoped wrapper around the authenticated status tracker.
pub struct ClientBindingStatusSession {
    trusted_relay_pubkey: PublicKey,
    expected_event_author_pubkey: PublicKey,
    connection_epoch: ClientBindingEpoch,
    bootstrap_event_id: Option<EventId>,
    bootstrap_latched_invalid: bool,
    tracker: Option<ClientBindingStatusTracker>,
    projected_fresh_until: Option<u64>,
}

impl fmt::Debug for ClientBindingStatusSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClientBindingStatusSession")
            .field("trusted_relay_pubkey", &"[redacted]")
            .field("expected_event_author_pubkey", &"[redacted]")
            .field("connection_epoch", &"[redacted]")
            .field(
                "bootstrap_event_id",
                &self.bootstrap_event_id.map(|_| "[redacted]"),
            )
            .field("bootstrap_latched_invalid", &self.bootstrap_latched_invalid)
            .field("tracker", &self.tracker.as_ref().map(|_| "[redacted]"))
            .field("projected_fresh_until", &"[redacted]")
            .finish()
    }
}

impl ClientBindingStatusSession {
    /// Start an empty connection-scoped consumer.
    pub fn new(
        trusted_relay_pubkey: PublicKey,
        expected_event_author_pubkey: PublicKey,
        connection_epoch: ClientBindingEpoch,
    ) -> Self {
        Self {
            trusted_relay_pubkey,
            expected_event_author_pubkey,
            connection_epoch,
            bootstrap_event_id: None,
            bootstrap_latched_invalid: false,
            tracker: None,
            projected_fresh_until: None,
        }
    }

    /// Exact authenticated connection epoch.
    pub fn connection_epoch(&self) -> &ClientBindingEpoch {
        &self.connection_epoch
    }

    /// Swallow and fold an exact reserved EVENT frame.
    ///
    /// `None` identifies ordinary non-reserved traffic, which callers must
    /// deliver unchanged. Any malformed or unauthenticated reserved frame is
    /// fail-closed and clears an existing projection.
    pub fn consume_text(&mut self, text: &str, now: u64) -> Option<ProjectionUpdate> {
        let frame = reserved_frame(text.as_bytes())?;
        Some(match frame {
            ReservedFrame::Bootstrap(event) => self.accept_bootstrap(event, now),
            ReservedFrame::Status(event) => self.accept_status(event, now),
        })
    }

    /// Currently projected freshness bound, if any.
    pub const fn projected_fresh_until(&self) -> Option<u64> {
        self.projected_fresh_until
    }

    /// Clear presentation exactly at or after the exclusive freshness bound.
    pub fn expire(&mut self, now: u64) -> ProjectionUpdate {
        let expired = self
            .projected_fresh_until
            .is_some_and(|fresh_until| now >= fresh_until);
        if !expired {
            return ProjectionUpdate::Unchanged;
        }
        if let Some(tracker) = self.tracker.as_mut() {
            let _ = tracker.current_presentation(now);
        }
        self.projected_fresh_until = None;
        ProjectionUpdate::Clear
    }

    /// Clear presentation on disconnect while retaining the local floor.
    pub fn disconnect(&mut self) -> ProjectionUpdate {
        if let Some(tracker) = self.tracker.as_mut() {
            tracker.on_disconnect();
        }
        self.projected_fresh_until = None;
        ProjectionUpdate::Clear
    }

    fn accept_bootstrap(&mut self, reserved: ReservedEvent, now: u64) -> ProjectionUpdate {
        let Ok(event) = reserved.event else {
            return self.clear_trusted_invalid();
        };
        if verify_event(&event).is_err() || event.pubkey != self.trusted_relay_pubkey {
            return self.clear_trusted_invalid();
        }
        if !reserved.exact_outer_shape || self.bootstrap_latched_invalid {
            self.bootstrap_latched_invalid = true;
            return self.clear_trusted_invalid();
        }
        let bootstrap = match validate_client_binding_bootstrap_event(
            &event,
            &self.trusted_relay_pubkey,
            &self.connection_epoch,
            &self.expected_event_author_pubkey,
            now,
        ) {
            Ok(bootstrap) => bootstrap,
            Err(_) => {
                self.bootstrap_latched_invalid = true;
                return self.clear_trusted_invalid();
            }
        };
        if let Some(event_id) = self.bootstrap_event_id {
            return if event_id == event.id {
                ProjectionUpdate::Unchanged
            } else {
                self.bootstrap_latched_invalid = true;
                self.clear_trusted_invalid()
            };
        }
        self.bootstrap_event_id = Some(event.id);
        self.tracker = Some(ClientBindingStatusTracker::new(
            self.trusted_relay_pubkey,
            bootstrap.authorization_domain(),
            self.expected_event_author_pubkey,
        ));
        ProjectionUpdate::Unchanged
    }

    fn accept_status(&mut self, reserved: ReservedEvent, now: u64) -> ProjectionUpdate {
        let Ok(event) = reserved.event else {
            return self.clear_trusted_invalid();
        };
        if verify_event(&event).is_err() || event.pubkey != self.trusted_relay_pubkey {
            return self.clear_trusted_invalid();
        }
        if self.bootstrap_latched_invalid {
            return self.clear_trusted_invalid();
        }
        if !reserved.exact_outer_shape {
            if let Some(tracker) = self.tracker.as_mut() {
                if tracker.accept(&event, now).is_err() {
                    tracker.retain_trusted_invalid_high_water(&event);
                }
                tracker.on_disconnect();
            } else {
                self.bootstrap_latched_invalid = true;
            }
            return self.clear_trusted_invalid();
        }
        let Some(tracker) = self.tracker.as_mut() else {
            self.bootstrap_latched_invalid = true;
            return self.clear_trusted_invalid();
        };
        match tracker.accept(&event, now) {
            Ok(ClientBindingStatusUpdate::Duplicate) => ProjectionUpdate::Unchanged,
            Ok(ClientBindingStatusUpdate::Accepted) => {
                let Some(status) = tracker.current_presentation(now) else {
                    self.projected_fresh_until = None;
                    return ProjectionUpdate::Clear;
                };
                self.projected_fresh_until = Some(status.fresh_until());
                ProjectionUpdate::Current(CurrentProjection {
                    event_author_pubkey: self.expected_event_author_pubkey.to_hex(),
                    fresh_until: status.fresh_until(),
                })
            }
            Err(_) => {
                tracker.retain_trusted_invalid_high_water(&event);
                self.clear_trusted_invalid()
            }
        }
    }

    fn clear_trusted_invalid(&mut self) -> ProjectionUpdate {
        self.projected_fresh_until = None;
        ProjectionUpdate::Clear
    }
}

fn reserved_frame(bytes: &[u8]) -> Option<ReservedFrame> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let values = value.as_array()?;
    if values.first().and_then(Value::as_str) != Some("EVENT") {
        return None;
    }
    let reserved = match values.get(1).and_then(Value::as_str) {
        Some(CLIENT_BINDING_BOOTSTRAP_SUB_ID) => ReservedFrame::Bootstrap,
        Some(CLIENT_BINDING_STATUS_SUB_ID) => ReservedFrame::Status,
        _ => return None,
    };
    let event = values
        .get(2)
        .cloned()
        .ok_or(())
        .and_then(|value| serde_json::from_value(value).map_err(|_| ()));
    Some(reserved(ReservedEvent {
        event,
        exact_outer_shape: values.len() == 3,
    }))
}

/// Identify reserved status/bootstrap text without consuming ordinary frames.
pub fn is_reserved_text(text: &str) -> bool {
    reserved_frame(text.as_bytes()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client_binding_bootstrap::ClientBindingBootstrapInputV1,
        client_binding_status::{ClientBindingStatusDisposition, ClientBindingStatusInputV1},
        CommunityId,
    };
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Timestamp};
    use serde_json::json;
    use uuid::Uuid;

    const ISSUED_AT: u64 = 1_800_000_000;
    const FRESH_UNTIL: u64 = ISSUED_AT + 120;

    fn domain() -> CommunityId {
        CommunityId::from_uuid(Uuid::from_u128(0x1234))
    }

    fn epoch_b() -> ClientBindingEpoch {
        ClientBindingEpoch::parse("22222222-2222-4222-8222-222222222222").unwrap()
    }

    fn bootstrap(relay: &Keys, author: &Keys, epoch: ClientBindingEpoch) -> Event {
        ClientBindingBootstrapInputV1::new(domain(), author.public_key(), epoch, ISSUED_AT)
            .unwrap()
            .sign_with_relay_keys(relay)
            .unwrap()
    }

    fn status(
        relay: &Keys,
        author: &Keys,
        revision: u64,
        disposition: ClientBindingStatusDisposition,
    ) -> Event {
        let input = match disposition {
            ClientBindingStatusDisposition::DisplayCurrent => ClientBindingStatusInputV1::current(
                domain(),
                author.public_key(),
                7,
                "policy-v1",
                revision,
                ISSUED_AT,
                FRESH_UNTIL,
            ),
            ClientBindingStatusDisposition::Withdrawn => ClientBindingStatusInputV1::withdrawn(
                domain(),
                author.public_key(),
                revision,
                ISSUED_AT,
                FRESH_UNTIL,
            ),
        }
        .unwrap();
        input.sign_with_relay_keys(relay).unwrap()
    }

    fn frame(sub_id: &str, event: &Event) -> String {
        json!(["EVENT", sub_id, event]).to_string()
    }

    fn extra_outer_value_frame(sub_id: &str, event: &Event) -> String {
        json!(["EVENT", sub_id, event, { "unexpected": true }]).to_string()
    }

    fn live_session(relay: &Keys, author: &Keys) -> ClientBindingStatusSession {
        let mut session =
            ClientBindingStatusSession::new(relay.public_key(), author.public_key(), epoch_b());
        assert!(matches!(
            session.consume_text(
                &frame(
                    CLIENT_BINDING_BOOTSTRAP_SUB_ID,
                    &bootstrap(relay, author, epoch_b()),
                ),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Unchanged)
        ));
        assert!(matches!(
            session.consume_text(
                &frame(
                    CLIENT_BINDING_STATUS_SUB_ID,
                    &status(
                        relay,
                        author,
                        1,
                        ClientBindingStatusDisposition::DisplayCurrent,
                    ),
                ),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Current(_))
        ));
        session
    }

    #[test]
    fn current_projection_serializes_as_exact_epoch_free_two_key_ipc_shape() {
        let author = Keys::generate();
        let projection = CurrentProjection {
            event_author_pubkey: author.public_key().to_hex(),
            fresh_until: FRESH_UNTIL,
        };

        let serialized = serde_json::to_value(projection).expect("projection serializes");
        assert_eq!(
            serialized,
            json!({
                "eventAuthorPubkey": author.public_key().to_hex(),
                "freshUntil": FRESH_UNTIL,
            })
        );
        let object = serialized.as_object().expect("projection is an object");
        assert_eq!(object.len(), 2);
        for forbidden in [
            "connectionEpoch",
            "connection_epoch",
            "epoch",
            "statusRevision",
            "bindingVersion",
            "policyVersion",
        ] {
            assert!(
                object.get(forbidden).is_none(),
                "leaked IPC field {forbidden}"
            );
        }
    }

    #[test]
    fn reserved_frames_are_classified_without_consuming_ordinary_traffic() {
        assert!(is_reserved_text(
            &json!(["EVENT", CLIENT_BINDING_STATUS_SUB_ID]).to_string()
        ));
        assert!(is_reserved_text(
            &json!(["EVENT", CLIENT_BINDING_BOOTSTRAP_SUB_ID, "bad"]).to_string()
        ));
        assert!(!is_reserved_text(
            &json!(["EVENT", "ordinary", {}]).to_string()
        ));
        assert!(!is_reserved_text("not-json"));
    }

    #[test]
    fn malformed_or_unauthenticated_reserved_frames_clear_live_projection() {
        let relay = Keys::generate();
        let wrong_relay = Keys::generate();
        let author = Keys::generate();
        let signed = status(
            &relay,
            &author,
            2,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let mut tampered_json: Value = serde_json::from_str(&signed.as_json()).unwrap();
        tampered_json["content"] = Value::String("{}".to_string());
        let tampered = Event::from_json(tampered_json.to_string()).unwrap();
        let wrong_signer = status(
            &wrong_relay,
            &author,
            2,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let cases = [
            json!(["EVENT", CLIENT_BINDING_STATUS_SUB_ID]).to_string(),
            json!(["EVENT", CLIENT_BINDING_STATUS_SUB_ID, "bad"]).to_string(),
            frame(CLIENT_BINDING_STATUS_SUB_ID, &tampered),
            frame(CLIENT_BINDING_STATUS_SUB_ID, &wrong_signer),
            json!(["EVENT", CLIENT_BINDING_BOOTSTRAP_SUB_ID]).to_string(),
        ];

        for reserved in cases {
            let mut session = live_session(&relay, &author);
            assert!(matches!(
                session.consume_text(&reserved, ISSUED_AT),
                Some(ProjectionUpdate::Clear)
            ));
            assert!(session.projected_fresh_until().is_none());
        }
    }

    #[test]
    fn malformed_reserved_outer_shape_consumes_high_water_before_clearing() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session =
            ClientBindingStatusSession::new(relay.public_key(), author.public_key(), epoch_b());
        assert!(matches!(
            session.consume_text(
                &frame(
                    CLIENT_BINDING_BOOTSTRAP_SUB_ID,
                    &bootstrap(&relay, &author, epoch_b()),
                ),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Unchanged)
        ));
        let revision_two = status(
            &relay,
            &author,
            2,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let revision_three = status(
            &relay,
            &author,
            3,
            ClientBindingStatusDisposition::DisplayCurrent,
        );

        assert!(matches!(
            session.consume_text(
                &extra_outer_value_frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_two),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Clear)
        ));
        assert!(matches!(
            session.consume_text(
                &frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_two),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Unchanged)
        ));
        assert!(session.projected_fresh_until().is_none());
        assert!(matches!(
            session.consume_text(
                &frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_three),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Current(_))
        ));
    }

    #[test]
    fn trusted_invalid_same_scope_revision_advances_hidden_high_water() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session = live_session(&relay, &author);
        let revision_four = status(
            &relay,
            &author,
            4,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        let mut invalid_payload: Value =
            serde_json::from_str(&revision_four.content).expect("synthetic status payload");
        invalid_payload["fresh_until"] = json!(ISSUED_AT);
        let trusted_invalid = EventBuilder::new(
            Kind::Custom(crate::kind::KIND_CLIENT_BINDING_STATUS as u16),
            invalid_payload.to_string(),
        )
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("trusted-invalid status signs");

        assert!(matches!(
            session.consume_text(
                &frame(CLIENT_BINDING_STATUS_SUB_ID, &trusted_invalid),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Clear)
        ));
        assert!(matches!(
            session.consume_text(
                &frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_four),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Clear)
        ));

        let revision_five = status(
            &relay,
            &author,
            5,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        assert!(matches!(
            session.consume_text(
                &frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_five),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Current(_))
        ));
    }

    #[test]
    fn wrong_bootstrap_epoch_latches_session_invalid() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let wrong_epoch =
            ClientBindingEpoch::parse("11111111-1111-4111-8111-111111111111").unwrap();
        let mut session =
            ClientBindingStatusSession::new(relay.public_key(), author.public_key(), epoch_b());
        assert!(matches!(
            session.consume_text(
                &frame(
                    CLIENT_BINDING_BOOTSTRAP_SUB_ID,
                    &bootstrap(&relay, &author, wrong_epoch),
                ),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Clear)
        ));

        assert!(matches!(
            session.consume_text(
                &frame(
                    CLIENT_BINDING_BOOTSTRAP_SUB_ID,
                    &bootstrap(&relay, &author, epoch_b()),
                ),
                ISSUED_AT,
            ),
            Some(ProjectionUpdate::Clear)
        ));
        let current = status(
            &relay,
            &author,
            1,
            ClientBindingStatusDisposition::DisplayCurrent,
        );
        assert!(matches!(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &current), ISSUED_AT),
            Some(ProjectionUpdate::Clear)
        ));
    }

    #[test]
    fn withdrawal_expiry_and_disconnect_clear_current_projection() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session = live_session(&relay, &author);
        let withdrawal = status(
            &relay,
            &author,
            2,
            ClientBindingStatusDisposition::Withdrawn,
        );
        assert!(matches!(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &withdrawal), ISSUED_AT,),
            Some(ProjectionUpdate::Clear)
        ));

        let mut session = live_session(&relay, &author);
        assert!(matches!(
            session.expire(FRESH_UNTIL),
            ProjectionUpdate::Clear
        ));
        let mut session = live_session(&relay, &author);
        assert!(matches!(session.disconnect(), ProjectionUpdate::Clear));
    }
}
