//! Native-only relay binding status validation and projection.

use std::fmt;

use buzz_core_pkg::{
    client_binding_bootstrap::{
        validate_client_binding_bootstrap_event, ClientBindingEpoch,
        CLIENT_BINDING_BOOTSTRAP_SUB_ID, CLIENT_BINDING_STATUS_SUB_ID,
    },
    client_binding_status::{ClientBindingStatusTracker, ClientBindingStatusUpdate},
    verify_event,
};
use nostr::{Event, EventId, PublicKey};
use serde::Serialize;
use serde_json::Value;

/// Current-only data permitted to cross the native IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurrentProjection {
    pub(crate) event_author_pubkey: String,
    pub(crate) fresh_until: u64,
    pub(crate) connection_epoch: String,
}

/// One change produced by the serialized native fold.
pub(crate) enum ProjectionUpdate {
    Unchanged,
    Clear,
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

/// Connection-scoped wrapper around the shared authenticated status tracker.
pub(crate) struct ClientBindingStatusSession {
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
    pub(crate) fn new(
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

    pub(crate) fn connection_epoch(&self) -> &ClientBindingEpoch {
        &self.connection_epoch
    }

    /// Swallow and fold an exact reserved EVENT frame. All other frames return
    /// `None` and must be delivered to the webview unchanged.
    pub(crate) fn consume_text(&mut self, text: &str, now: u64) -> Option<ProjectionUpdate> {
        let frame = reserved_frame(text.as_bytes())?;
        Some(match frame {
            ReservedFrame::Bootstrap(event) => self.accept_bootstrap(event, now),
            ReservedFrame::Status(event) => self.accept_status(event, now),
        })
    }

    #[cfg(test)]
    pub(crate) fn projected_fresh_until(&self) -> Option<u64> {
        self.projected_fresh_until
    }

    pub(crate) fn expire(&mut self, now: u64) -> ProjectionUpdate {
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

    pub(crate) fn disconnect(&mut self) -> ProjectionUpdate {
        if let Some(tracker) = self.tracker.as_mut() {
            tracker.on_disconnect();
        }
        self.projected_fresh_until = None;
        ProjectionUpdate::Clear
    }

    fn accept_bootstrap(&mut self, reserved: ReservedEvent, now: u64) -> ProjectionUpdate {
        let Ok(event) = reserved.event else {
            return ProjectionUpdate::Unchanged;
        };
        if verify_event(&event).is_err() || event.pubkey != self.trusted_relay_pubkey {
            return ProjectionUpdate::Unchanged;
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
            return ProjectionUpdate::Unchanged;
        };
        if verify_event(&event).is_err() || event.pubkey != self.trusted_relay_pubkey {
            return ProjectionUpdate::Unchanged;
        }
        if self.bootstrap_latched_invalid {
            return self.clear_trusted_invalid();
        }
        if !reserved.exact_outer_shape {
            if let Some(tracker) = self.tracker.as_mut() {
                // The outer frame is trusted-invalid, but a valid inner status
                // must still consume its revision so replaying the identical
                // event later in an exact array cannot restore presentation.
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
                    connection_epoch: self.connection_epoch.as_str().to_owned(),
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

/// Classify the reserved exact-connection channels without requiring an
/// eligible presentation session. Every native socket uses this to prevent
/// bootstrap and status frames from reaching raw browser delivery.
pub(crate) fn is_reserved_text(text: &str) -> bool {
    reserved_frame(text.as_bytes()).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core_pkg::{
        client_binding_bootstrap::{
            ClientBindingBootstrapInputV1, CLIENT_BINDING_BOOTSTRAP_SUB_ID,
            CLIENT_BINDING_STATUS_SUB_ID,
        },
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

    fn epoch() -> ClientBindingEpoch {
        ClientBindingEpoch::parse("11111111-1111-4111-8111-111111111111").expect("synthetic epoch")
    }

    fn session(relay: &Keys, author: &Keys) -> ClientBindingStatusSession {
        ClientBindingStatusSession::new(relay.public_key(), author.public_key(), epoch())
    }

    fn bootstrap(relay: &Keys, author: &Keys) -> Event {
        ClientBindingBootstrapInputV1::new(domain(), author.public_key(), epoch(), ISSUED_AT)
            .expect("synthetic bootstrap input")
            .sign_with_relay_keys(relay)
            .expect("synthetic bootstrap signs")
    }

    fn status(
        relay: &Keys,
        author: &Keys,
        revision: u64,
        disposition: ClientBindingStatusDisposition,
        policy: &str,
    ) -> Event {
        let input = match disposition {
            ClientBindingStatusDisposition::DisplayCurrent => ClientBindingStatusInputV1::current(
                domain(),
                author.public_key(),
                7,
                policy,
                revision,
                ISSUED_AT,
                FRESH_UNTIL,
                None,
            ),
            ClientBindingStatusDisposition::Withdrawn => ClientBindingStatusInputV1::withdrawn(
                domain(),
                author.public_key(),
                revision,
                ISSUED_AT,
                FRESH_UNTIL,
            ),
        }
        .expect("synthetic status input");
        input
            .sign_with_relay_keys(relay)
            .expect("synthetic status signs")
    }

    fn frame(sub_id: &str, event: &Event) -> String {
        json!(["EVENT", sub_id, event]).to_string()
    }

    fn extra_outer_value_frame(sub_id: &str, event: &Event) -> String {
        json!(["EVENT", sub_id, event, "unexpected"]).to_string()
    }

    fn assert_unchanged(update: Option<ProjectionUpdate>) {
        assert!(matches!(update, Some(ProjectionUpdate::Unchanged)));
    }

    fn assert_clear(update: Option<ProjectionUpdate>) {
        assert!(matches!(update, Some(ProjectionUpdate::Clear)));
    }

    fn assert_current(update: Option<ProjectionUpdate>, author: &Keys, revision: u64) {
        let Some(ProjectionUpdate::Current(current)) = update else {
            panic!("expected a current projection");
        };
        assert_eq!(current.event_author_pubkey, author.public_key().to_hex());
        assert_eq!(current.fresh_until, FRESH_UNTIL);
        assert_eq!(current.connection_epoch, epoch().as_str());
        assert!(revision > 0, "call site documents the accepted revision");
    }

    #[test]
    fn reserved_channels_are_classified_before_event_decoding() {
        assert!(is_reserved_text(
            &json!(["EVENT", CLIENT_BINDING_BOOTSTRAP_SUB_ID]).to_string()
        ));
        assert!(is_reserved_text(
            &json!(["EVENT", CLIENT_BINDING_STATUS_SUB_ID, "not-an-event"]).to_string()
        ));
        assert!(!is_reserved_text(
            &json!(["EVENT", "ordinary-subscription", {}]).to_string()
        ));
        assert!(!is_reserved_text("not-json"));
    }

    #[test]
    fn status_before_bootstrap_fails_closed_and_latches_the_session() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session = session(&relay, &author);
        let current = status(
            &relay,
            &author,
            1,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v1",
        );

        assert_clear(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &current), ISSUED_AT),
        );
        assert_clear(session.consume_text(
            &frame(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap(&relay, &author)),
            ISSUED_AT,
        ));
        assert!(session.projected_fresh_until().is_none());
    }

    #[test]
    fn unauthenticated_or_wrong_signer_noise_has_no_effect() {
        let relay = Keys::generate();
        let wrong_relay = Keys::generate();
        let author = Keys::generate();
        let mut session = session(&relay, &author);
        let wrong_signer = status(
            &wrong_relay,
            &author,
            1,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v1",
        );
        assert_unchanged(session.consume_text(
            &frame(CLIENT_BINDING_STATUS_SUB_ID, &wrong_signer),
            ISSUED_AT,
        ));

        let signed = status(
            &relay,
            &author,
            1,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v1",
        );
        let mut tampered_json: Value =
            serde_json::from_str(&signed.as_json()).expect("status event parses");
        tampered_json["content"] = Value::String("{}".to_string());
        let tampered = Event::from_json(tampered_json.to_string()).expect("tampered event parses");
        assert_unchanged(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &tampered), ISSUED_AT),
        );

        assert_unchanged(session.consume_text(
            &frame(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap(&relay, &author)),
            ISSUED_AT,
        ));
        assert_current(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &signed), ISSUED_AT),
            &author,
            1,
        );
    }

    #[test]
    fn trusted_invalid_status_clears_duplicate_cannot_restore_and_newer_can() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session = session(&relay, &author);
        let bootstrap = bootstrap(&relay, &author);
        let current = status(
            &relay,
            &author,
            1,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v1",
        );
        let conflicting_equal = status(
            &relay,
            &author,
            1,
            ClientBindingStatusDisposition::DisplayCurrent,
            "different-policy",
        );
        let newer = status(
            &relay,
            &author,
            2,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v2",
        );

        assert_unchanged(session.consume_text(
            &frame(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap),
            ISSUED_AT,
        ));
        assert_current(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &current), ISSUED_AT),
            &author,
            1,
        );
        assert_clear(session.consume_text(
            &frame(CLIENT_BINDING_STATUS_SUB_ID, &conflicting_equal),
            ISSUED_AT,
        ));
        assert_unchanged(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &current), ISSUED_AT),
        );
        assert!(session.projected_fresh_until().is_none());
        assert_current(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &newer), ISSUED_AT),
            &author,
            2,
        );
    }

    #[test]
    fn malformed_reserved_outer_shape_consumes_high_water_before_clearing() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session = session(&relay, &author);
        let revision_two = status(
            &relay,
            &author,
            2,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v2",
        );
        let revision_three = status(
            &relay,
            &author,
            3,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v3",
        );
        assert_unchanged(session.consume_text(
            &frame(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap(&relay, &author)),
            ISSUED_AT,
        ));

        assert_clear(session.consume_text(
            &extra_outer_value_frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_two),
            ISSUED_AT,
        ));
        assert_unchanged(session.consume_text(
            &frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_two),
            ISSUED_AT,
        ));
        assert!(session.projected_fresh_until().is_none());
        assert_current(
            session.consume_text(
                &frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_three),
                ISSUED_AT,
            ),
            &author,
            3,
        );
    }

    #[test]
    fn trusted_invalid_parseable_revision_advances_hidden_high_water() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session = session(&relay, &author);
        assert_unchanged(session.consume_text(
            &frame(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap(&relay, &author)),
            ISSUED_AT,
        ));
        let first = status(
            &relay,
            &author,
            1,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v1",
        );
        assert_current(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &first), ISSUED_AT),
            &author,
            1,
        );

        let revision_four = status(
            &relay,
            &author,
            4,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v4",
        );
        let mut invalid_payload: Value =
            serde_json::from_str(&revision_four.content).expect("synthetic status payload");
        invalid_payload["authorization_domain"] = json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
        let trusted_invalid = EventBuilder::new(
            Kind::Custom(buzz_core_pkg::kind::KIND_CLIENT_BINDING_STATUS as u16),
            invalid_payload.to_string(),
        )
        .tags([])
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("trusted-invalid status signs");
        assert_clear(session.consume_text(
            &frame(CLIENT_BINDING_STATUS_SUB_ID, &trusted_invalid),
            ISSUED_AT,
        ));
        assert_clear(session.consume_text(
            &frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_four),
            ISSUED_AT,
        ));

        let revision_five = status(
            &relay,
            &author,
            5,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v5",
        );
        assert_current(
            session.consume_text(
                &frame(CLIENT_BINDING_STATUS_SUB_ID, &revision_five),
                ISSUED_AT,
            ),
            &author,
            5,
        );
    }

    #[test]
    fn withdrawal_and_passive_expiry_clear_current_projection() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session = session(&relay, &author);
        assert_unchanged(session.consume_text(
            &frame(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap(&relay, &author)),
            ISSUED_AT,
        ));
        let current = status(
            &relay,
            &author,
            1,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v1",
        );
        assert_current(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &current), ISSUED_AT),
            &author,
            1,
        );
        let withdrawn = status(
            &relay,
            &author,
            2,
            ClientBindingStatusDisposition::Withdrawn,
            "unused",
        );
        assert_clear(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &withdrawn), ISSUED_AT),
        );

        let newer = status(
            &relay,
            &author,
            3,
            ClientBindingStatusDisposition::DisplayCurrent,
            "policy-v3",
        );
        assert_current(
            session.consume_text(&frame(CLIENT_BINDING_STATUS_SUB_ID, &newer), ISSUED_AT),
            &author,
            3,
        );
        assert!(matches!(
            session.expire(FRESH_UNTIL - 1),
            ProjectionUpdate::Unchanged
        ));
        assert!(matches!(
            session.expire(FRESH_UNTIL),
            ProjectionUpdate::Clear
        ));
        assert!(session.projected_fresh_until().is_none());
    }

    #[test]
    fn expected_signer_invalid_bootstrap_latches_exact_origin() {
        let relay = Keys::generate();
        let author = Keys::generate();
        let mut session = session(&relay, &author);
        let invalid = EventBuilder::new(
            Kind::Custom(buzz_core_pkg::kind::KIND_CLIENT_BINDING_BOOTSTRAP as u16),
            "{}",
        )
        .tags([])
        .custom_created_at(Timestamp::from(ISSUED_AT))
        .sign_with_keys(&relay)
        .expect("trusted-invalid bootstrap signs");

        assert_clear(
            session.consume_text(&frame(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &invalid), ISSUED_AT),
        );
        assert_clear(session.consume_text(
            &frame(CLIENT_BINDING_BOOTSTRAP_SUB_ID, &bootstrap(&relay, &author)),
            ISSUED_AT,
        ));
    }
}
