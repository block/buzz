//! Postgres-backed tests for the landing channel: the kind:9002 `landing` and
//! `welcome` tags with their exact receipts, the kind:39000 projection, the
//! protections, the NIP-11 advertisement and provisioning's adoption.

use std::sync::Arc;

use buzz_core::TenantContext;
use nostr::{EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{
    community, create_channel, edit_channel, group_state_tags, http_auth, state,
};
use crate::handlers::ingest::ingest_event;
use crate::state::AppState;

const PROTECTED: &str =
    "invalid: this is the community's landing channel; make another channel the landing channel first";

async fn relay_member(state: &AppState, tenant: &TenantContext, role: &str) -> Keys {
    let keys = Keys::generate();
    state
        .db
        .add_relay_member(
            tenant.community(),
            &keys.public_key().to_hex(),
            role,
            None,
            &Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .expect("seed relay member");
    keys
}

/// The landing projection on `channel_id`'s kind:39000: whether it carries
/// `["landing"]`, and its `welcome` text.
async fn projection(
    state: &AppState,
    tenant: &TenantContext,
    channel_id: Uuid,
) -> (bool, Option<String>) {
    let tags = group_state_tags(state, tenant, channel_id, 39000).await;
    let landing = tags.iter().any(|tag| tag.len() == 1 && tag[0] == "landing");
    let welcome = tags
        .iter()
        .find(|tag| tag.first().map(String::as_str) == Some("welcome"))
        .map(|tag| tag.get(1).cloned().unwrap_or_default());
    (landing, welcome)
}

/// NIP-11 `landing_channel` for the tenant's host.
async fn advertised(state: &AppState, tenant: &TenantContext) -> Option<String> {
    match crate::nip11::nip11_document(state, tenant.host()).await {
        Ok(info) => info.landing_channel,
        Err(response) => panic!("NIP-11 refused: {}", response.status()),
    }
}

/// Only a relay owner or admin may send `landing` or `welcome`, only on a
/// stream, with valid values; every refusal carries the contract's exact
/// receipt text and designates nothing.
///
/// Mutations: drop `landing`/`welcome` from the relay-role check → the
/// channel owner's edits are accepted (RED); drop the stream check → the
/// forum is designated (RED).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn landing_tags_need_a_relay_owner_or_admin_and_a_stream() {
    let state = state().await;
    let tenant = community(&state, "landing-authz").await;
    let owner = Keys::generate();
    let admin = relay_member(&state, &tenant, "admin").await;
    let member = relay_member(&state, &tenant, "member").await;
    let stream = create_channel(&state, &tenant, &owner, &[]).await;
    let forum = create_channel(&state, &tenant, &owner, &[&["channel_type", "forum"]]).await;

    let not_authorized =
        "invalid: actor not authorized: landing and welcome require a relay owner or admin";
    let cases: &[(&Keys, Uuid, &[&[&str]], &str)] = &[
        (&owner, stream, &[&["landing", "true"]], not_authorized),
        (&owner, stream, &[&["welcome", "Hi"]], not_authorized),
        (
            &member,
            stream,
            &[&["landing", "true"], &["welcome", ""]],
            not_authorized,
        ),
        (
            &admin,
            forum,
            &[&["landing", "true"]],
            "invalid: landing is only available on stream channels",
        ),
        (
            &admin,
            stream,
            &[&["landing", "false"]],
            "invalid: landing cannot be turned off; make another channel the landing channel",
        ),
        (
            &admin,
            stream,
            &[&["landing", "1"]],
            "invalid: invalid landing value: 1 (must be \"true\")",
        ),
        (
            &admin,
            stream,
            &[&["welcome", "   "]],
            "invalid: welcome message must not be blank",
        ),
        (
            &admin,
            stream,
            &[&["landing", "true"], &["welcome", "a\u{7}b"]],
            "invalid: welcome message must be plain text (line breaks only)",
        ),
        (
            &admin,
            stream,
            &[&["welcome", "Hi"]],
            "invalid: welcome can only be set on the landing channel",
        ),
    ];
    for (keys, channel, tags, expected) in cases {
        assert_eq!(
            edit_channel(&state, &tenant, keys, *channel, tags)
                .await
                .expect_err("refused"),
            *expected,
            "tags {tags:?}"
        );
    }
    assert_eq!(
        state
            .db
            .get_community_landing(tenant.community())
            .await
            .expect("landing"),
        buzz_db::landing::CommunityLanding::default(),
        "nothing is designated"
    );
    assert_eq!(advertised(&state, &tenant).await, None);
}

/// The client's full-state form designates a channel, is idempotent on the
/// current landing channel, and moves the designation. kind:39000 carries
/// `["landing"]` and the custom `welcome` on the landing channel only, the
/// previous landing channel is re-signed without them, and NIP-11 follows.
/// While designated, archive, private, TTL and kind:9008 delete are refused.
///
/// Mutations: drop the previous-landing re-emission → `a` keeps its tags
/// (RED); emit the tags for every channel → `b` carries them early (RED);
/// drop the 9008 pre-check → the delete is accepted (RED).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn landing_designation_is_projected_advertised_and_protected() {
    let state = state().await;
    let tenant = community(&state, "landing-move").await;
    let owner = Keys::generate();
    let admin = relay_member(&state, &tenant, "admin").await;
    let a = create_channel(&state, &tenant, &owner, &[]).await;
    let b = create_channel(&state, &tenant, &owner, &[]).await;
    let id = |channel: Uuid| Some(channel.hyphenated().to_string());

    edit_channel(
        &state,
        &tenant,
        &admin,
        a,
        &[&["landing", "true"], &["welcome", "Hi all!\nSay hello."]],
    )
    .await
    .expect("designate a");
    assert_eq!(
        projection(&state, &tenant, a).await,
        (true, Some("Hi all!\nSay hello.".into()))
    );
    assert_eq!(projection(&state, &tenant, b).await, (false, None));
    assert_eq!(advertised(&state, &tenant).await, id(a));

    edit_channel(
        &state,
        &tenant,
        &admin,
        a,
        &[&["landing", "true"], &["welcome", ""]],
    )
    .await
    .expect("the full-state form on the landing channel");
    assert_eq!(
        projection(&state, &tenant, a).await,
        (true, None),
        "an empty welcome restores the default"
    );
    assert_eq!(advertised(&state, &tenant).await, id(a));

    edit_channel(
        &state,
        &tenant,
        &admin,
        b,
        &[&["landing", "true"], &["welcome", "Moved"]],
    )
    .await
    .expect("move to b");
    assert_eq!(
        projection(&state, &tenant, b).await,
        (true, Some("Moved".into()))
    );
    assert_eq!(
        projection(&state, &tenant, a).await,
        (false, None),
        "the previous landing channel is re-signed without its tags"
    );
    assert_eq!(advertised(&state, &tenant).await, id(b));

    let protected: [&[&[&str]]; 3] = [
        &[&["archived", "true"]],
        &[&["visibility", "private"]],
        &[&["ttl", "3600"]],
    ];
    for tags in protected {
        assert_eq!(
            edit_channel(&state, &tenant, &owner, b, tags)
                .await
                .expect_err("protected"),
            PROTECTED,
            "tags {tags:?}"
        );
    }
    let delete = EventBuilder::new(Kind::Custom(9008), "")
        .tags([Tag::parse(["h", &b.to_string()]).expect("h tag")])
        .sign_with_keys(&owner)
        .expect("sign 9008");
    let result = ingest_event(&state, &tenant, delete, http_auth(&owner))
        .await
        .expect("ingest 9008");
    assert!(!result.accepted, "the landing channel is not deleted");
    assert_eq!(result.message, PROTECTED);
    let after = state
        .db
        .get_channel(tenant.community(), b)
        .await
        .expect("b");
    assert!(after.deleted_at.is_none() && after.archived_at.is_none());
    assert_eq!(after.visibility, "open");

    // The previous landing channel lost its protections.
    edit_channel(&state, &tenant, &owner, a, &[&["archived", "true"]])
        .await
        .expect("archive the previous landing channel");
}

/// Operator provisioning of a community with an owner creates its landing
/// `general`, owned by that owner, and NIP-11 advertises it.
///
/// Mutation: drop the `converge_landing_channel` call in the `create_only`
/// branch → no designation (RED).
#[tokio::test]
#[ignore = "requires Postgres"]
async fn provisioning_creates_the_landing_general() {
    let operator = Keys::generate();
    let mut base = (*state().await).clone();
    let mut config = (*base.config).clone();
    config.relay_operator_pubkeys = vec![operator.public_key().to_hex()];
    base.config = Arc::new(config);
    let state = Arc::new(base);
    let owner = Keys::generate();
    let host = format!("landing-provision-{}.test", Uuid::new_v4().simple());

    let response = crate::handlers::community_provisioning::provision_community(
        &state,
        &operator.public_key(),
        crate::handlers::community_provisioning::ProvisionCommunityRequest {
            host: host.clone(),
            initial_owner_pubkey: Some(owner.public_key().to_hex()),
            create_only: true,
            initial_requires_nip_fi: None,
        },
    )
    .await
    .expect("provision");
    let community =
        buzz_core::CommunityId::from_uuid(response.community_id.parse().expect("community id"));
    let tenant = TenantContext::resolved(community, &host);

    let landing = state
        .db
        .get_community_landing(community)
        .await
        .expect("landing")
        .channel_id
        .expect("provisioning designates a landing channel");
    let general = state
        .db
        .get_channel(community, landing)
        .await
        .expect("general");
    assert_eq!(general.name, "general");
    assert_eq!(general.created_by, owner.public_key().to_bytes().to_vec());
    assert_eq!(
        state
            .db
            .get_member_role(community, landing, &owner.public_key().to_bytes())
            .await
            .expect("owner role")
            .as_deref(),
        Some("owner")
    );
    assert_eq!(projection(&state, &tenant, landing).await, (true, None));
    assert_eq!(
        advertised(&state, &tenant).await,
        Some(landing.hyphenated().to_string())
    );
}
