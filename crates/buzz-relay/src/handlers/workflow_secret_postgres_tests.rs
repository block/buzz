//! Postgres-backed tests for kind:46021: the owner of a webhook workflow
//! replaces its stored secret, and the secret-bearing event is never stored.

use std::sync::Arc;

use buzz_core::kind::{KIND_WORKFLOW_DEF, KIND_WORKFLOW_SECRET_SET};
use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{community, create_channel, http_auth, state};
use crate::handlers::ingest::{ingest_event, IngestError, IngestResult};
use crate::state::AppState;

const HMAC_WEBHOOK_YAML: &str = "name: coplan-wake\ntrigger:\n  on: webhook\n  verify: hmac_sha256\n  signature_header: X-CoPlan-Signature\n  signature_prefix: 'sha256='\nsteps:\n  - id: hold\n    action: delay\n    duration: 1m\n";
const MESSAGE_YAML: &str = "name: echo\ntrigger:\n  on: message_posted\nsteps:\n  - id: hold\n    action: delay\n    duration: 1m\n";

async fn save_workflow(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    channel_id: Uuid,
    workflow_id: Uuid,
    yaml: &str,
) {
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF as u16), yaml)
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["d", &workflow_id.to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .expect("sign workflow def");
    ingest_event(state, tenant, event, http_auth(keys))
        .await
        .expect("save workflow");
}

async fn set_secret(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    workflow_id: Uuid,
    secret: &str,
) -> Result<IngestResult, IngestError> {
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_SECRET_SET as u16), secret)
        .tags([Tag::parse(["d", &workflow_id.to_string()]).unwrap()])
        .sign_with_keys(keys)
        .expect("sign secret set");
    ingest_event(state, tenant, event, http_auth(keys)).await
}

async fn stored_secret(state: &AppState, tenant: &TenantContext, workflow_id: Uuid) -> String {
    let workflow = state
        .db
        .get_workflow(tenant.community(), workflow_id)
        .await
        .expect("workflow");
    crate::webhook_secret::extract_secret(&workflow.definition).expect("stored secret")
}

fn rejection(result: Result<IngestResult, IngestError>) -> String {
    match result {
        Err(IngestError::Rejected(message)) => message,
        Err(other) => panic!("expected Rejected, got {other:?}"),
        Ok(ok) => panic!("expected rejection, got {:?}", ok.message),
    }
}

/// Mutations: skip the owner check → the stranger's secret lands → RED;
/// persist the event like other command kinds → stored 46021 row → RED;
/// accept non-webhook workflows → message workflow gains a secret → RED;
/// regenerate on re-save → secret changes after the YAML update → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn owner_replaces_webhook_secret_without_storing_the_event() {
    let state = state().await;
    let tenant = community(&state, "workflow-secret").await;
    let owner = Keys::generate();
    let stranger = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let webhook_workflow = Uuid::new_v4();
    save_workflow(
        &state,
        &tenant,
        &owner,
        channel_id,
        webhook_workflow,
        HMAC_WEBHOOK_YAML,
    )
    .await;
    let generated = stored_secret(&state, &tenant, webhook_workflow).await;

    let message =
        rejection(set_secret(&state, &tenant, &stranger, webhook_workflow, "stolen").await);
    assert!(message.contains("workflow not found"), "{message}");
    assert_eq!(
        stored_secret(&state, &tenant, webhook_workflow).await,
        generated
    );

    let message = rejection(set_secret(&state, &tenant, &owner, webhook_workflow, "").await);
    assert!(message.contains("secret must be"), "{message}");

    let result = set_secret(
        &state,
        &tenant,
        &owner,
        webhook_workflow,
        "coplan-wake-secret",
    )
    .await
    .expect("owner sets secret");
    assert!(result.accepted);
    assert_eq!(
        stored_secret(&state, &tenant, webhook_workflow).await,
        "coplan-wake-secret"
    );

    let stored = state
        .db
        .query_events(&EventQuery {
            kinds: Some(vec![KIND_WORKFLOW_SECRET_SET as i32]),
            ..EventQuery::for_community(tenant.community())
        })
        .await
        .expect("query secret-set events");
    assert!(
        stored.is_empty(),
        "the secret-bearing event must not be stored"
    );

    save_workflow(
        &state,
        &tenant,
        &owner,
        channel_id,
        webhook_workflow,
        HMAC_WEBHOOK_YAML,
    )
    .await;
    assert_eq!(
        stored_secret(&state, &tenant, webhook_workflow).await,
        "coplan-wake-secret",
        "re-saving the YAML keeps the caller-supplied secret"
    );

    let message_workflow = Uuid::new_v4();
    save_workflow(
        &state,
        &tenant,
        &owner,
        channel_id,
        message_workflow,
        MESSAGE_YAML,
    )
    .await;
    let message = rejection(set_secret(&state, &tenant, &owner, message_workflow, "s").await);
    assert!(message.contains("webhook trigger"), "{message}");
}
