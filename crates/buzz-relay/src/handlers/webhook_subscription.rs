//! NIP-WH webhook subscriptions: NIP-AR artifacts of type
//! [`buzz_core::artifact::WEBHOOK_SUBSCRIPTION_TYPE`].
//!
//! NIP-AR owns identity, revisions and reads, and the artifact store checks
//! that the signer is a channel admin. This module checks what only this type
//! defines: the `integration` tag and the content, including both CEL
//! expressions, so a mistake is refused when an admin saves it instead of
//! failing every later delivery.

use buzz_core::artifact::{ArtifactEnvelope, ArtifactOp};
use buzz_webhook_filter::CompileError;
use nostr::Event;
use serde::Deserialize;

use super::ingest::IngestError;

/// NIP-WH `SubscriptionContent`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    version: u32,
    filter: String,
    transform: Option<String>,
}

fn rejected(message: impl Into<String>) -> IngestError {
    IngestError::Rejected(message.into())
}

/// Check a revision's `integration` tag and content. A delete carries
/// neither: NIP-AR keeps only envelope tags and empty content.
pub(crate) async fn check(event: &Event, envelope: &ArtifactEnvelope) -> Result<(), IngestError> {
    if envelope.op == ArtifactOp::Delete {
        return Ok(());
    }
    let (filter, transform) = programs(event).map_err(rejected)?;
    check_cel(filter, transform).await
}

/// A revision's filter and transform sources, or why its `integration` tag or
/// content is not valid. The worker reads revisions with this too.
pub(crate) fn programs(event: &Event) -> Result<(String, Option<String>), String> {
    let mut integrations = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.first().is_some_and(|name| name == "integration"));
    match (integrations.next(), integrations.next()) {
        (Some([_, id]), None) => {
            buzz_core::artifact::canonical_uuid(id)
                .map_err(|error| format!("invalid: integration tag: {error}"))?;
        }
        _ => {
            return Err(
                "invalid: webhook subscription requires exactly one integration tag with one value"
                    .into(),
            )
        }
    }
    let content: Content = serde_json::from_str(&event.content)
        .map_err(|error| format!("invalid: webhook subscription content: {error}"))?;
    if content.version != 1 {
        return Err("invalid: unsupported webhook subscription version".into());
    }
    Ok((content.filter, content.transform))
}

/// Compile both expressions. Parsing runs on its own thread
/// (`buzz_webhook_filter`), so it never holds an async worker.
async fn check_cel(filter: String, transform: Option<String>) -> Result<(), IngestError> {
    let checked = tokio::task::spawn_blocking(move || {
        buzz_webhook_filter::compile_filter(&filter).map_err(|error| ("filter", error))?;
        if let Some(transform) = transform {
            buzz_webhook_filter::compile_transform(&transform)
                .map_err(|error| ("transform", error))?;
        }
        Ok(())
    })
    .await
    .map_err(|error| {
        IngestError::Internal(format!("error: webhook subscription check: {error}"))
    })?;
    checked.map_err(|(field, error)| match error {
        CompileError::Unavailable(_) => {
            IngestError::Internal(format!("error: webhook subscription check: {error}"))
        }
        error => rejected(format!("invalid: {field}: {error}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn event(integration: &[&[&str]], content: &str) -> Event {
        EventBuilder::new(Kind::Custom(45010), content)
            .tags(
                integration
                    .iter()
                    .map(|tag| Tag::parse(tag.iter().copied()).expect("tag")),
            )
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    #[test]
    fn refuses_malformed_tags_and_content() {
        let id = "0f4c1a52-7d3e-4b8a-9c61-2e5f8d7a1b30";
        let upper = id.to_uppercase();
        let valid = r#"{"version":1,"filter":"true"}"#;
        let one: &[&[&str]] = &[&["integration", id]];
        assert!(programs(&event(one, valid)).is_ok());
        assert!(programs(&event(
            one,
            r#"{"version":1,"filter":"true","transform":"{}"}"#
        ))
        .is_ok());
        for (tags, content, reason) in [
            (&[][..], valid, "exactly one integration tag"),
            (
                &[&["integration", id][..], &["integration", id]][..],
                valid,
                "exactly one integration tag",
            ),
            (
                &[&["integration", id, "extra"][..]][..],
                valid,
                "exactly one integration tag",
            ),
            (
                &[&["integration", &upper[..]][..]][..],
                valid,
                "invalid: integration tag:",
            ),
            (
                &[&["integration", "abc"][..]][..],
                valid,
                "invalid: integration tag:",
            ),
            (
                &[&["integration", &"ab".repeat(32)[..]][..]][..],
                valid,
                "invalid: integration tag:",
            ),
            (
                &[&["integration", "00000000-0000-0000-0000-000000000000"][..]][..],
                valid,
                "invalid: integration tag:",
            ),
            (one, "not json", "invalid: webhook subscription content:"),
            (
                one,
                r#"{"version":2,"filter":"true"}"#,
                "unsupported webhook subscription version",
            ),
            (
                one,
                r#"{"version":1}"#,
                "invalid: webhook subscription content:",
            ),
            (
                one,
                r#"{"version":1,"filter":"true","name":"x"}"#,
                "unknown field `name`",
            ),
        ] {
            let Err(message) = programs(&event(tags, content)) else {
                panic!("{content} with {tags:?} was accepted");
            };
            assert!(message.contains(reason), "{message}");
        }
    }
}
