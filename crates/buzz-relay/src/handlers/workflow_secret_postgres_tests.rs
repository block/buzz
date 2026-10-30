//! Postgres-backed tests for kind:46021: the owner of a webhook workflow
//! replaces its stored secret, and the secret-bearing event is never stored.

use std::sync::Arc;

use buzz_auth::Scope;
use buzz_core::kind::{KIND_WORKFLOW_DEF, KIND_WORKFLOW_SECRET_SET};
use buzz_core::TenantContext;
use buzz_db::event::EventQuery;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{community, create_channel, http_auth, state};
use crate::handlers::ingest::{ingest_event, IngestAuth, IngestError, IngestResult};
use crate::state::AppState;

const HMAC_WEBHOOK_YAML: &str = "name: coplan-wake\ntrigger:\n  on: webhook\n  verify: hmac_sha256\n  signature_header: X-CoPlan-Signature\n  signature_prefix: 'sha256='\nsteps:\n  - id: hold\n    action: delay\n    duration: 1m\n";
const HMAC_WEBHOOK_YAML_RENAMED: &str = "name: coplan-wake-renamed\ntrigger:\n  on: webhook\n  verify: hmac_sha256\n  signature_header: X-CoPlan-Signature\n  signature_prefix: 'sha256='\nsteps:\n  - id: hold\n    action: delay\n    duration: 1m\n";
const MESSAGE_YAML: &str = "name: echo\ntrigger:\n  on: message_posted\nsteps:\n  - id: hold\n    action: delay\n    duration: 1m\n";

/// Saves a kind:30620. `seconds_later` keeps a re-save from being dropped as
/// a same-second duplicate or a superseded replaceable event.
async fn save_workflow(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    workflow: (Uuid, Uuid),
    yaml: &str,
    seconds_later: u64,
) -> IngestResult {
    let (channel_id, workflow_id) = workflow;
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_DEF as u16), yaml)
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).unwrap(),
            Tag::parse(["d", &workflow_id.to_string()]).unwrap(),
        ])
        .custom_created_at(Timestamp::from(Timestamp::now().as_secs() + seconds_later))
        .sign_with_keys(keys)
        .expect("sign workflow def");
    ingest_event(state, tenant, event, http_auth(keys))
        .await
        .expect("save workflow")
}

async fn set_secret(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    workflow_id: Uuid,
    secret: &str,
    auth: IngestAuth,
) -> Result<IngestResult, IngestError> {
    let event = EventBuilder::new(Kind::Custom(KIND_WORKFLOW_SECRET_SET as u16), secret)
        .tags([Tag::parse(["d", &workflow_id.to_string()]).unwrap()])
        .sign_with_keys(keys)
        .expect("sign secret set");
    ingest_event(state, tenant, event, auth).await
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
/// skip the token channel check → the other-channel token's secret lands → RED;
/// persist the event like other command kinds → stored 46021 row → RED;
/// regenerate on re-save → secret changes after the YAML update → RED;
/// accept non-webhook workflows → message workflow gains a secret → RED.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn owner_replaces_webhook_secret_without_storing_the_event() {
    let state = state().await;
    let tenant = community(&state, "workflow-secret").await;
    let owner = Keys::generate();
    let stranger = Keys::generate();
    let channel_id = create_channel(&state, &tenant, &owner, &[]).await;
    let webhook_workflow = Uuid::new_v4();
    let webhook = (channel_id, webhook_workflow);
    save_workflow(&state, &tenant, &owner, webhook, HMAC_WEBHOOK_YAML, 0).await;
    let generated = stored_secret(&state, &tenant, webhook_workflow).await;

    let denials = [
        ("stranger", &stranger, http_auth(&stranger)),
        (
            "owner token scoped to another channel",
            &owner,
            IngestAuth::Nip42 {
                pubkey: owner.public_key(),
                scopes: vec![Scope::MessagesWrite],
                channel_ids: Some(vec![Uuid::new_v4()]),
                conn_id: Uuid::new_v4(),
            },
        ),
    ];
    for (label, keys, auth) in denials {
        let message =
            rejection(set_secret(&state, &tenant, keys, webhook_workflow, "stolen", auth).await);
        assert!(message.contains("workflow not found"), "{label}: {message}");
        assert_eq!(
            stored_secret(&state, &tenant, webhook_workflow).await,
            generated,
            "{label}"
        );
    }

    let message = rejection(
        set_secret(
            &state,
            &tenant,
            &owner,
            webhook_workflow,
            "",
            http_auth(&owner),
        )
        .await,
    );
    assert!(message.contains("secret must be"), "{message}");

    let result = set_secret(
        &state,
        &tenant,
        &owner,
        webhook_workflow,
        "coplan-wake-secret",
        http_auth(&owner),
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

    let resave = save_workflow(
        &state,
        &tenant,
        &owner,
        webhook,
        HMAC_WEBHOOK_YAML_RENAMED,
        5,
    )
    .await;
    assert!(
        !resave.message.starts_with("duplicate"),
        "the re-save must reach upsert_workflow: {}",
        resave.message
    );
    let renamed = state
        .db
        .get_workflow(tenant.community(), webhook_workflow)
        .await
        .expect("workflow");
    assert_eq!(renamed.name, "coplan-wake-renamed");
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
        (channel_id, message_workflow),
        MESSAGE_YAML,
        0,
    )
    .await;
    let message = rejection(
        set_secret(
            &state,
            &tenant,
            &owner,
            message_workflow,
            "s",
            http_auth(&owner),
        )
        .await,
    );
    assert!(message.contains("webhook trigger"), "{message}");
}
