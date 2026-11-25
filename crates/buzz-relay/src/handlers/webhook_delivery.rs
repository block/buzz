//! NIP-WH delivery intake (kind 20701).
//!
//! A configured ingress gateway posts each delivery to `POST /events` on its
//! community's host. The relay keeps no integration records: it trusts the
//! gateway to route each delivery, and queues it in `webhook_deliveries`. A
//! delivery never enters `events`, so no read path can serve it.
//!
//! The HTTP bridge sends deliveries here instead of through relay membership
//! and [`super::ingest::ingest_event`], which refuses them, as does the
//! WebSocket handler.

use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use nostr::{Event, PublicKey};
use serde::Deserialize;
use uuid::Uuid;

use super::ingest::{IngestError, IngestResult};
use crate::state::AppState;

/// Answer to a delivery signed by a key that is not a configured gateway.
pub(crate) const NOT_INGRESS: &str = "restricted: not an authorized webhook ingress";

/// Largest delivery content, the same cap as every other event.
const MAX_CONTENT_BYTES: usize = 256 * 1024;
/// Furthest a delivery's `created_at` may be ahead of the relay clock.
const MAX_FUTURE_SKEW_SECS: u64 = 900;
/// Longest provider delivery ID.
const MAX_DELIVERY_ID_BYTES: usize = 256;
/// Longest provider name.
const MAX_PROVIDER_BYTES: usize = 64;

/// The tag values a validated delivery is queued under.
#[derive(Debug, PartialEq, Eq)]
struct Delivery {
    integration: Uuid,
    provider: String,
}

/// The parts of the content the relay checks. Other fields are allowed, so a
/// gateway can add fields without a relay release.
#[derive(Deserialize)]
struct Content<'a> {
    version: u64,
    #[serde(rename = "type", borrow)]
    event_type: std::borrow::Cow<'a, str>,
    #[serde(borrow)]
    delivery_id: std::borrow::Cow<'a, str>,
    #[serde(rename = "payload", borrow)]
    _payload: &'a serde_json::value::RawValue,
}

/// Whether `signer` may submit deliveries. Intake is off when no key is
/// configured.
pub(crate) fn authorize(state: &AppState, signer: &PublicKey) -> Result<(), IngestError> {
    if state.config.webhook.is_ingress(signer) {
        Ok(())
    } else {
        Err(IngestError::AuthFailed(NOT_INGRESS.into()))
    }
}

/// Validate a delivery from an [`authorize`]d `signer` and queue it.
///
/// Resending a queued event succeeds with NIP-01's duplicate answer, so the
/// gateway can retry the same signed event until it gets a 200.
pub(crate) async fn intake(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    event: Event,
    signer: &PublicKey,
) -> Result<IngestResult, IngestError> {
    super::ingest::map_serving_fence_state(
        buzz_deletion::store(&state.db)
            .is_serving_active(tenant.community())
            .await,
    )?;
    if event.pubkey != *signer {
        return Err(IngestError::AuthFailed(
            "invalid: event pubkey does not match authenticated identity".into(),
        ));
    }
    let event = Arc::new(event);
    let to_verify = Arc::clone(&event);
    tokio::task::spawn_blocking(move || buzz_core::verification::verify_event(&to_verify))
        .await
        .map_err(|e| IngestError::Internal(format!("error: verification task failed: {e}")))?
        .map_err(|e| IngestError::Rejected(format!("invalid: {e}")))?;
    let delivery = validate(
        &event,
        nostr::Timestamp::now().as_secs(),
        state.config.webhook.delivery_max_age_secs,
    )
    .map_err(|reason| IngestError::Rejected(format!("invalid: {reason}")))?;
    let enqueued = state
        .db
        .enqueue_webhook_delivery(
            tenant.community(),
            &event,
            delivery.integration,
            &delivery.provider,
        )
        .await
        .map_err(|e| IngestError::Internal(format!("error: queueing delivery: {e}")))?;
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: match enqueued {
            buzz_db::webhook::Enqueued::New => String::new(),
            buzz_db::webhook::Enqueued::Duplicate => "duplicate: already have this event".into(),
        },
    })
}

/// Check a delivery's age, size, tags and content. `now` is in seconds.
fn validate(event: &Event, now: u64, max_age_secs: u64) -> Result<Delivery, &'static str> {
    let created_at = event.created_at.as_secs();
    if created_at > now.saturating_add(MAX_FUTURE_SKEW_SECS) {
        return Err("event timestamp too far from server time");
    }
    if created_at < now.saturating_sub(max_age_secs) {
        return Err("delivery too old");
    }
    if event.content.len() > MAX_CONTENT_BYTES {
        return Err("delivery too large");
    }

    const TAGS: &str = "delivery needs one integration, t and delivery tag";
    let (mut integration, mut provider, mut delivery_id) = (None, None, None);
    for tag in event.tags.iter().map(nostr::Tag::as_slice) {
        let slot = match tag.first().map(String::as_str) {
            Some("integration") => &mut integration,
            Some("t") => &mut provider,
            Some("delivery") => &mut delivery_id,
            _ => continue,
        };
        let [_, value] = tag else { return Err(TAGS) };
        if slot.replace(value.as_str()).is_some() {
            return Err(TAGS);
        }
    }
    let (Some(integration), Some(provider), Some(delivery_id)) =
        (integration, provider, delivery_id)
    else {
        return Err(TAGS);
    };
    let integration = buzz_core::artifact::canonical_uuid(integration)
        .map_err(|_| "integration must be a canonical lowercase UUID")?;
    let provider = provider
        .strip_prefix("provider:")
        .filter(|p| {
            (1..=MAX_PROVIDER_BYTES).contains(&p.len())
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
        .ok_or("t tag must be provider:<name>, lowercase letters, digits, _ or -")?;
    if delivery_id.is_empty() || delivery_id.len() > MAX_DELIVERY_ID_BYTES {
        return Err("delivery tag must be 1 to 256 bytes");
    }

    let content: Content<'_> =
        serde_json::from_str(&event.content).map_err(|_| "delivery content is not valid")?;
    if content.version != 1 {
        return Err("unsupported delivery content version");
    }
    if content.event_type.is_empty() || content.delivery_id != delivery_id {
        return Err("delivery content is not valid");
    }
    Ok(Delivery {
        integration,
        provider: provider.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    const INTEGRATION: &str = "0f4c1a52-7d3e-4b8a-9c61-2e5f8d7a1b30";
    const NOW: u64 = 1_800_000_000;
    const MAX_AGE: u64 = 72 * 60 * 60;

    fn event(tags: &[&[&str]], content: &str, created_at: u64) -> Event {
        EventBuilder::new(Kind::Custom(20701), content)
            .tags(tags.iter().map(|t| Tag::parse(t.iter().copied()).unwrap()))
            .custom_created_at(Timestamp::from_secs(created_at))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    fn content(delivery_id: &str) -> String {
        serde_json::json!({
            "version": 1, "type": "pull_request", "delivery_id": delivery_id,
            "payload": {"action": "opened"},
        })
        .to_string()
    }

    const GOOD: &[&[&str]] = &[
        &["integration", INTEGRATION],
        &["t", "provider:github"],
        &["delivery", "d-1"],
    ];

    #[test]
    fn accepts_what_the_gateway_sends() {
        let id = Uuid::parse_str(INTEGRATION).unwrap();
        assert_eq!(
            validate(&gateway_event(id), NOW, MAX_AGE),
            Ok(Delivery {
                integration: id,
                provider: "github".into()
            })
        );
    }

    /// The tags and content `buzz-ingress-gateway`'s forwarder builds, with an
    /// extra content field a later gateway may add.
    fn gateway_event(integration: Uuid) -> Event {
        let content = r#"{"version":1,"type":"pull_request","delivery_id":"d-1","payload":{"n":1},"later":true}"#;
        EventBuilder::new(Kind::Custom(20701), content)
            .tag(Tag::custom(
                nostr::TagKind::custom("integration"),
                [integration.to_string()],
            ))
            .tag(Tag::hashtag("provider:github"))
            .tag(Tag::custom(nostr::TagKind::custom("delivery"), ["d-1"]))
            .custom_created_at(Timestamp::from_secs(NOW - MAX_AGE))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn refuses_malformed_deliveries() {
        let good = content("d-1");
        let big = format!(r#"{{"pad":"{}"}}"#, "x".repeat(MAX_CONTENT_BYTES));
        let uppercase = INTEGRATION.to_uppercase();
        let cases: &[(&[&[&str]], &str, u64, &str)] = &[
            (GOOD, &good, NOW - MAX_AGE - 1, "delivery too old"),
            (GOOD, &good, NOW + 901, "event timestamp too far"),
            (GOOD, &big, NOW, "delivery too large"),
            (&GOOD[1..], &good, NOW, "needs one integration"),
            (
                &[GOOD[0], GOOD[0], GOOD[1], GOOD[2]],
                &good,
                NOW,
                "needs one integration",
            ),
            (
                &[GOOD[0], GOOD[1], &["t", "provider:gitlab"], GOOD[2]],
                &good,
                NOW,
                "needs one integration",
            ),
            (
                &[GOOD[0], GOOD[1], &["delivery", "d-1", "extra"]],
                &good,
                NOW,
                "needs one integration",
            ),
            (
                &[&["integration", &uppercase], GOOD[1], GOOD[2]],
                &good,
                NOW,
                "canonical",
            ),
            (
                &[GOOD[0], &["t", "github"], GOOD[2]],
                &good,
                NOW,
                "provider:<name>",
            ),
            (
                &[GOOD[0], &["t", "provider:Git Hub"], GOOD[2]],
                &good,
                NOW,
                "provider:<name>",
            ),
            (
                &[GOOD[0], GOOD[1], &["delivery", ""]],
                &good,
                NOW,
                "1 to 256 bytes",
            ),
            (GOOD, &content("d-2"), NOW, "content is not valid"),
            (
                GOOD,
                r#"{"version":1,"type":"x","delivery_id":"d-1"}"#,
                NOW,
                "content is not valid",
            ),
            (
                GOOD,
                r#"{"version":2,"type":"x","delivery_id":"d-1","payload":{}}"#,
                NOW,
                "version",
            ),
            (GOOD, "not json", NOW, "content is not valid"),
        ];
        for (tags, content, created_at, reason) in cases {
            let refused =
                validate(&event(tags, content, *created_at), NOW, MAX_AGE).expect_err(reason);
            assert!(refused.contains(reason), "{reason}: {refused}");
        }
        // Unrelated tags are ignored.
        let mut tags = GOOD.to_vec();
        tags.push(&["client", "x"]);
        assert!(validate(&event(&tags, &good, NOW), NOW, MAX_AGE).is_ok());
    }
}
