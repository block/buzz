use std::future::Future;

use nostr::{Event, Keys};
use tauri::State;

use crate::{
    app_state::AppState,
    events::{self, ArtifactRevisionRef},
    models::SendChannelMessageResponse,
    relay::{
        assert_expected_relay_scope, assert_expected_signer, query_relay_at_with_keys,
        submit_signed_event_classified, SubmitFailure,
    },
};

use super::thread_ref::provided_thread_ref;

/// Whether `held` is byte-for-byte the wake `built` by this command: same ID,
/// author, kind, timestamp, tags, and content, with a valid signature. A
/// duplicate or ambiguous acknowledgement is only ever resolved by this check,
/// never by building a second wake from the same timestamp.
fn is_exact_wake(held: &Event, built: &Event) -> bool {
    held.verify().is_ok()
        && held.id == built.id
        && held.pubkey == built.pubkey
        && held.kind == built.kind
        && held.created_at == built.created_at
        && held.tags == built.tags
        && held.content == built.content
}

/// Ask the relay whether it already holds exactly this wake event.
async fn relay_holds_exact_wake(
    state: &AppState,
    api_base_url: &str,
    keys: &Keys,
    built: &Event,
) -> bool {
    let filter = serde_json::json!({ "ids": [built.id.to_hex()], "limit": 1 });
    query_relay_at_with_keys(state, api_base_url, &[filter], keys, None)
        .await
        .is_ok_and(|events| events.iter().any(|held| is_exact_wake(held, built)))
}

/// Submit one built wake. A failure whose outcome is unknown is settled by
/// reading the relay for that exact event: held means delivered.
async fn attempt_wake<S, SF, H, HF>(
    event: &Event,
    submit: &S,
    holds: &H,
) -> Result<(), SubmitFailure>
where
    S: Fn(Event) -> SF,
    SF: Future<Output = Result<(), SubmitFailure>>,
    H: Fn(Event) -> HF,
    HF: Future<Output = bool>,
{
    match submit(event.clone()).await {
        Ok(()) => Ok(()),
        Err(failure) => {
            if holds(event.clone()).await {
                Ok(())
            } else {
                Err(failure)
            }
        }
    }
}

/// Deliver one wake and return the event the relay was given.
///
/// The wake is first built from the frozen `created_at`, so a prompt retry is
/// the byte-identical event. Only when the relay explicitly refuses that
/// timestamp as stale (it checks the freshness window before storing anything,
/// so the event is certainly absent) is the wake rebuilt once with `now`: the
/// identical content and tags, hence the identical feedback binding, under a
/// fresh timestamp and therefore a new event ID. That is safe only because the
/// relay keeps one wake per `(author, feedback revision)`: a wake it already
/// recorded for this feedback (an earlier refresh whose acknowledgement was
/// lost) answers `duplicate:` and nothing is stored twice. A refresh is never
/// attempted for any other failure, and never when no time has passed.
async fn deliver_wake<B, S, SF, H, HF>(
    created_at: u64,
    now: u64,
    build: B,
    submit: S,
    holds: H,
) -> Result<Event, String>
where
    B: Fn(u64) -> Result<Event, String>,
    S: Fn(Event) -> SF,
    SF: Future<Output = Result<(), SubmitFailure>>,
    H: Fn(Event) -> HF,
    HF: Future<Output = bool>,
{
    let first = build(created_at)?;
    match attempt_wake(&first, &submit, &holds).await {
        Ok(()) => return Ok(first),
        Err(failure) if failure.is_stale_timestamp() && now != created_at => {}
        Err(failure) => return Err(failure.into_message()),
    }
    let refreshed = build(now)?;
    attempt_wake(&refreshed, &submit, &holds)
        .await
        .map(|()| refreshed)
        .map_err(SubmitFailure::into_message)
}

/// Publish the kind-9 reply that wakes the executive agent for one already
/// published `synaxis.artifact-feedback` revision.
///
/// The renderer freezes `created_at` (and every other input) with its draft, so
/// the first attempt and a prompt retry rebuild the identical event rather than
/// minting a second wake. If the submission fails or its acknowledgement is
/// ambiguous, the relay is read back and the wake counts as sent only when it
/// holds that exact event. When the relay's freshness window has passed and it
/// refuses the frozen timestamp, the wake is rebuilt once with a fresh timestamp
/// (see [`deliver_wake`]); the relay deduplicates wakes by feedback revision, so
/// the retry can never create a second one. The send fails closed when the
/// active community or identity changed since the caller captured them.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn send_artifact_feedback_message(
    channel_id: String,
    content: String,
    parent_event_id: String,
    root_event_id: String,
    executive_agent_pubkey: String,
    feedback_artifact_id: String,
    feedback_revision_event_id: String,
    reviewed_artifact_id: String,
    reviewed_revision_event_id: String,
    created_at: u64,
    expected_relay_url: String,
    expected_signer_pubkey: String,
    state: State<'_, AppState>,
) -> Result<SendChannelMessageResponse, String> {
    let channel_uuid = uuid::Uuid::parse_str(&channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    if expected_relay_url.trim().is_empty() || expected_signer_pubkey.trim().is_empty() {
        return Err("feedback notification needs its captured relay and signer".to_string());
    }
    let relay_base = crate::relay::relay_api_base_url_with_override(&state);
    assert_expected_relay_scope(Some(&expected_relay_url), &relay_base)?;
    let signing_keys = state.signing_keys()?;
    assert_expected_signer(
        Some(&expected_signer_pubkey),
        &signing_keys.public_key().to_hex(),
    )?;

    let thread_ref = provided_thread_ref(&root_event_id, &parent_event_id)?;
    let depth = if root_event_id == parent_event_id { 1 } else { 2 };
    let app: &AppState = &state;
    let relay_base = relay_base.as_str();
    let keys = &signing_keys;

    let sent = deliver_wake(
        created_at,
        nostr::Timestamp::now().as_secs(),
        |at| {
            events::build_artifact_feedback_message(
                channel_uuid,
                &content,
                &thread_ref,
                &executive_agent_pubkey,
                &ArtifactRevisionRef {
                    artifact_id: &feedback_artifact_id,
                    revision_event_id: &feedback_revision_event_id,
                },
                &ArtifactRevisionRef {
                    artifact_id: &reviewed_artifact_id,
                    revision_event_id: &reviewed_revision_event_id,
                },
                at,
            )?
            .sign_with_keys(keys)
            .map_err(|error| format!("failed to sign event: {error}"))
        },
        |event: Event| async move {
            submit_signed_event_classified(&event, app, relay_base, keys)
                .await
                .map(|_| ())
        },
        |event: Event| async move { relay_holds_exact_wake(app, relay_base, keys, &event).await },
    )
    .await?;

    Ok(SendChannelMessageResponse {
        event_id: sent.id.to_hex(),
        root_event_id: Some(root_event_id),
        parent_event_id: Some(parent_event_id),
        depth,
        created_at: sent.created_at.as_secs() as i64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag};
    use std::future::ready;
    use std::cell::RefCell;

    fn wake(content: &str, created_at: u64, keys: &Keys) -> Event {
        EventBuilder::new(Kind::Custom(9), content)
            .tags([Tag::parse(["h", "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50"]).unwrap()])
            .custom_created_at(nostr::Timestamp::from(created_at))
            .sign_with_keys(keys)
            .unwrap()
    }

    #[test]
    fn only_the_byte_identical_wake_resolves_an_ambiguous_acknowledgement() {
        let keys = Keys::generate();
        let built = wake("feedback", 1_700_000_000, &keys);
        // Re-signing yields a different Schnorr signature but the same event ID.
        assert!(is_exact_wake(&wake("feedback", 1_700_000_000, &keys), &built));

        for other in [
            wake("feedback ", 1_700_000_000, &keys),
            wake("feedback", 1_700_000_001, &keys),
            wake("feedback", 1_700_000_000, &Keys::generate()),
        ] {
            assert!(!is_exact_wake(&other, &built));
        }

        // A held event whose body was altered after signing fails verification.
        let mut value = serde_json::to_value(&built).unwrap();
        value["content"] = serde_json::json!("tampered");
        let tampered: Event = serde_json::from_value(value).unwrap();
        assert!(!is_exact_wake(&tampered, &built));
    }

    const FIRST: u64 = 1_700_000_000;
    /// Longer than the relay's 900 s freshness window after `FIRST`.
    const LATER: u64 = FIRST + 1_000;

    fn stale() -> SubmitFailure {
        SubmitFailure::Refused {
            status: 400,
            message: format!(
                "relay returned 400 Bad Request: {}",
                buzz_core_pkg::artifact::STALE_EVENT_TIMESTAMP_REJECTION
            ),
        }
    }

    /// What the fake relay saw: every submission, in order.
    #[derive(Default)]
    struct Relay {
        submitted: RefCell<Vec<(nostr::EventId, u64)>>,
    }

    impl Relay {
        fn submitted_at(&self) -> Vec<u64> {
            self.submitted.borrow().iter().map(|e| e.1).collect()
        }
    }

    fn build_with(keys: &Keys) -> impl Fn(u64) -> Result<Event, String> + '_ {
        move |at| Ok(wake("feedback", at, keys))
    }

    #[tokio::test]
    async fn a_fresh_wake_is_sent_once_with_its_frozen_timestamp() {
        let keys = Keys::generate();
        let relay = Relay::default();
        let sent = deliver_wake(
            FIRST,
            FIRST + 5,
            build_with(&keys),
            |event: Event| {
                relay
                    .submitted
                    .borrow_mut()
                    .push((event.id, event.created_at.as_secs()));
                ready(Ok(()))
            },
            |_| ready(false),
        )
        .await
        .unwrap();
        assert_eq!(sent.created_at.as_secs(), FIRST);
        assert_eq!(relay.submitted_at(), vec![FIRST]);
    }

    #[tokio::test]
    async fn a_stale_timestamp_is_refreshed_once_and_the_relay_dedupes_the_wake() {
        let keys = Keys::generate();
        let relay = Relay::default();
        let sent = deliver_wake(
            FIRST,
            LATER,
            build_with(&keys),
            |event: Event| {
                let at = event.created_at.as_secs();
                relay.submitted.borrow_mut().push((event.id, at));
                // The relay refuses only the frozen timestamp.
                ready(if at == FIRST { Err(stale()) } else { Ok(()) })
            },
            |_| ready(false),
        )
        .await
        .unwrap();
        assert_eq!(relay.submitted_at(), vec![FIRST, LATER]);
        assert_eq!(sent.created_at.as_secs(), LATER);
        let first = wake("feedback", FIRST, &keys);
        assert_ne!(sent.id, first.id, "a fresh timestamp is a new event ID");
        assert_eq!(sent.content, first.content);
        assert_eq!(sent.tags, first.tags, "the feedback binding is unchanged");
    }

    #[tokio::test]
    async fn a_stale_refusal_of_a_wake_the_relay_already_holds_is_not_refreshed() {
        let keys = Keys::generate();
        let relay = Relay::default();
        let sent = deliver_wake(
            FIRST,
            LATER,
            build_with(&keys),
            |event: Event| {
                relay
                    .submitted
                    .borrow_mut()
                    .push((event.id, event.created_at.as_secs()));
                ready(Err(stale()))
            },
            // The lost-acknowledgement case: the exact event is stored.
            |_| ready(true),
        )
        .await
        .unwrap();
        assert_eq!(relay.submitted_at(), vec![FIRST]);
        assert_eq!(sent.created_at.as_secs(), FIRST);
    }

    #[tokio::test]
    async fn only_a_stale_timestamp_refusal_earns_a_refresh() {
        let keys = Keys::generate();
        for failure in [
            SubmitFailure::Unknown("relay unreachable: request timed out".into()),
            SubmitFailure::Refused {
                status: 403,
                message: "relay returned 403 Forbidden: restricted: not a member".into(),
            },
            // The same words under a different status are not the freshness
            // refusal.
            SubmitFailure::Refused {
                status: 500,
                message: format!(
                    "relay returned 500: {}",
                    buzz_core_pkg::artifact::STALE_EVENT_TIMESTAMP_REJECTION
                ),
            },
        ] {
            let relay = Relay::default();
            let expected = failure.clone().into_message();
            let error = deliver_wake(
                FIRST,
                LATER,
                build_with(&keys),
                |event: Event| {
                    relay
                        .submitted
                        .borrow_mut()
                        .push((event.id, event.created_at.as_secs()));
                    ready(Err(failure.clone()))
                },
                |_| ready(false),
            )
            .await
            .unwrap_err();
            assert_eq!(error, expected);
            assert_eq!(relay.submitted_at(), vec![FIRST], "{expected}");
        }
    }

    #[tokio::test]
    async fn a_refresh_is_attempted_once_and_never_when_no_time_passed() {
        let keys = Keys::generate();

        // Refused both times: one refresh, then the relay's reason surfaces.
        let relay = Relay::default();
        let error = deliver_wake(
            FIRST,
            LATER,
            build_with(&keys),
            |event: Event| {
                relay
                    .submitted
                    .borrow_mut()
                    .push((event.id, event.created_at.as_secs()));
                ready(Err(stale()))
            },
            |_| ready(false),
        )
        .await
        .unwrap_err();
        assert!(error.contains("event timestamp too far"));
        assert_eq!(relay.submitted_at(), vec![FIRST, LATER]);

        // The clock has not moved, so a refresh would rebuild the same event.
        let relay = Relay::default();
        deliver_wake(
            FIRST,
            FIRST,
            build_with(&keys),
            |event: Event| {
                relay
                    .submitted
                    .borrow_mut()
                    .push((event.id, event.created_at.as_secs()));
                ready(Err(stale()))
            },
            |_| ready(false),
        )
        .await
        .unwrap_err();
        assert_eq!(relay.submitted_at(), vec![FIRST]);
    }
}
