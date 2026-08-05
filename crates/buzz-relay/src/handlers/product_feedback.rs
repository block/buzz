//! Product-feedback event validation and sidecar persistence.

use std::sync::Arc;

use buzz_core::tenant::TenantContext;
use buzz_db::product_feedback::NewProductFeedback;
use nostr::Event;
use sha2::{Digest, Sha256};

use crate::state::AppState;

const CATEGORIES: &[&str] = &["bug", "praise", "needs-work"];
const MAX_BODY_BYTES: usize = 32 * 1024;
const MAX_TAGS_BYTES: usize = 64 * 1024;

/// Validate and persist a product-feedback event outside ordinary event storage.
pub async fn handle(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
) -> Result<(), String> {
    let (category, tags, event_created_at) = validate(tenant, event, state).await?;
    state
        .db
        .insert_product_feedback(
            tenant.community(),
            NewProductFeedback {
                event_id: event.id.as_bytes(),
                submitter_pubkey: &event.pubkey.to_bytes(),
                category,
                body: &event.content,
                tags: &tags,
                event_created_at,
            },
        )
        .await
        .map_err(|e| format!("error: database error inserting product feedback: {e}"))?;

    Ok(())
}

/// Validate and persist feedback at the transaction-owned protected commit
/// boundary. A retry observes the original receipt and never writes twice.
pub async fn handle_enforced(
    tenant: &TenantContext,
    event: &Event,
    state: &Arc<AppState>,
    protected: &crate::authorization_runtime::transport::ProtectedAuthorization,
) -> Result<(), String> {
    let (category, tags, event_created_at) = validate(tenant, event, state).await?;
    let operation_id = crate::authorization_runtime::executor::ProtectedOperationId::derive(
        tenant.community(),
        "product.feedback.v1",
        event.id.as_bytes(),
    )
    .map_err(|error| format!("error: {error}"))?;
    let mut request = Sha256::new();
    request.update(b"buzz-product-feedback-request-v1");
    request.update(event.id.as_bytes());
    let permit = protected
        .seal_postgres_mutation(
            operation_id,
            "product.feedback.v1",
            request.finalize().into(),
        )
        .map_err(|_| "restricted: protected authorization denied".to_string())?
        .ok_or_else(|| "restricted: protected authorization denied".to_string())?;
    match crate::authorization_runtime::executor::begin_authorized_operation(state, permit)
        .await
        .map_err(|error| format!("restricted: {error}"))?
    {
        crate::authorization_runtime::executor::AuthorizedOperationStart::Replay(payload) => {
            if payload.as_slice() != b"accepted" {
                return Err("error: protected feedback receipt is invalid".to_string());
            }
        }
        crate::authorization_runtime::executor::AuthorizedOperationStart::Execute(
            mut operation,
        ) => {
            buzz_db::product_feedback::insert_tx(
                operation.transaction(),
                tenant.community(),
                NewProductFeedback {
                    event_id: event.id.as_bytes(),
                    submitter_pubkey: &event.pubkey.to_bytes(),
                    category,
                    body: &event.content,
                    tags: &tags,
                    event_created_at,
                },
            )
            .await
            .map_err(|error| {
                format!("error: database error inserting product feedback: {error}")
            })?;
            operation
                .commit(b"accepted")
                .await
                .map_err(|error| format!("restricted: {error}"))?;
        }
    }
    Ok(())
}

async fn validate<'a>(
    tenant: &TenantContext,
    event: &'a Event,
    state: &Arc<AppState>,
) -> Result<
    (
        Option<&'a str>,
        serde_json::Value,
        chrono::DateTime<chrono::Utc>,
    ),
    String,
> {
    let category = parse_category(event)?;
    validate_body(&event.content)?;
    let imeta_tags = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "imeta")
        .map(|tag| tag.as_slice().iter().map(ToString::to_string).collect())
        .collect::<Vec<Vec<String>>>();
    if !imeta_tags.is_empty() {
        let media_base =
            crate::api::media::media_base_url_for_tenant(&state.config.relay_url, tenant.host());
        crate::api::validate_imeta_tags(&imeta_tags, &media_base)?;
        crate::api::verify_imeta_blobs(tenant, &imeta_tags, &state.media_storage).await?;
    }

    let tags = serialize_tags(event)?;
    let event_created_at =
        chrono::DateTime::from_timestamp(event.created_at.as_secs() as i64, 0)
            .ok_or_else(|| "invalid: feedback timestamp is out of range".to_string())?;

    Ok((category, tags, event_created_at))
}

fn serialize_tags(event: &Event) -> Result<serde_json::Value, String> {
    let tags_bytes = serde_json::to_vec(&event.tags)
        .map_err(|e| format!("error: failed to serialize feedback tags: {e}"))?;
    if tags_bytes.len() > MAX_TAGS_BYTES {
        return Err(format!(
            "invalid: feedback tags exceed maximum size of {MAX_TAGS_BYTES} bytes"
        ));
    }
    serde_json::from_slice(&tags_bytes)
        .map_err(|e| format!("error: failed to deserialize feedback tags: {e}"))
}

fn parse_category(event: &Event) -> Result<Option<&str>, String> {
    let values: Vec<&str> = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "category")
        .filter_map(|tag| tag.content())
        .collect();
    match values.as_slice() {
        [] => Ok(None),
        [category] if CATEGORIES.contains(category) => Ok(Some(category)),
        [_] => Err("invalid: unsupported feedback category".to_string()),
        _ => Err("invalid: feedback must include at most one category tag".to_string()),
    }
}

fn validate_body(body: &str) -> Result<(), String> {
    if body.trim().is_empty() {
        return Err("invalid: feedback body must not be empty".to_string());
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(format!(
            "invalid: feedback body exceeds maximum size of {MAX_BODY_BYTES} bytes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nostr::{Event, EventBuilder, Keys, Kind, Tag};

    use super::{parse_category, serialize_tags, validate_body};
    use buzz_core::kind::KIND_PRODUCT_FEEDBACK;

    fn feedback(tags: Vec<Tag>) -> Event {
        EventBuilder::new(
            Kind::Custom(KIND_PRODUCT_FEEDBACK as u16),
            "Useful feedback",
        )
        .tags(tags)
        .sign_with_keys(&Keys::generate())
        .expect("sign feedback")
    }

    #[test]
    fn accepts_supported_or_absent_category() {
        assert_eq!(parse_category(&feedback(vec![])).unwrap(), None);
        let event = feedback(vec![Tag::parse(["category", "bug"]).unwrap()]);
        assert_eq!(parse_category(&event).unwrap(), Some("bug"));
    }

    #[test]
    fn rejects_unknown_or_duplicate_category() {
        let unknown = feedback(vec![Tag::parse(["category", "idea"]).unwrap()]);
        assert!(parse_category(&unknown)
            .unwrap_err()
            .contains("unsupported"));
        let duplicate = feedback(vec![
            Tag::parse(["category", "bug"]).unwrap(),
            Tag::parse(["category", "praise"]).unwrap(),
        ]);
        assert!(parse_category(&duplicate)
            .unwrap_err()
            .contains("at most one"));
    }

    #[test]
    fn body_must_be_nonempty_and_bounded() {
        assert!(validate_body(" \n ").is_err());
        assert!(validate_body("works").is_ok());
        assert!(validate_body(&"x".repeat(32 * 1024 + 1)).is_err());
    }

    #[test]
    fn tags_are_serialized_and_bounded() {
        let small = feedback(vec![Tag::parse([
            "imeta",
            "url https://example.test/a.png",
        ])
        .unwrap()]);
        assert!(serialize_tags(&small).is_ok());

        let oversized = feedback(vec![
            Tag::parse(["diagnostics", &"x".repeat(64 * 1024)]).unwrap()
        ]);
        assert!(serialize_tags(&oversized)
            .unwrap_err()
            .contains("tags exceed maximum size"));
    }
}
