//! User erasure must reach community payloads outside the conversation store.
use super::*;
use crate::{
    artifact::ArtifactOutcome, community_artifact::CommunityArtifactPrincipal,
    user_deletion::ExecutionMode, CommunityId, Db,
};
use buzz_core::artifact::{self, ArtifactQuery, ArtifactScope, ArtifactView};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};

fn principal(keys: &Keys) -> CommunityArtifactPrincipal {
    CommunityArtifactPrincipal {
        pubkey: keys.public_key().to_bytes().to_vec(),
        verified_owner: None,
        owner_admission: false,
        require_membership: false,
    }
}

async fn revision(
    db: &Db,
    community: CommunityId,
    id: Uuid,
    author: &Keys,
    previous: Option<&Event>,
) -> Event {
    let mut tags = vec![
        vec!["ar".into(), "1".into()],
        vec!["community".into(), community.to_string()],
        vec!["d".into(), id.to_string()],
        vec!["type".into(), "example.note".into()],
        vec!["title".into(), "Personal data".into()],
        vec![
            "op".into(),
            if previous.is_some() {
                "update"
            } else {
                "create"
            }
            .into(),
        ],
    ];
    if let Some(previous) = previous {
        tags.push(vec!["prev".into(), previous.id.to_hex()]);
    }
    let event = EventBuilder::new(Kind::Custom(45010), "private authored payload")
        .tags(tags.into_iter().map(|tag| Tag::parse(tag).unwrap()))
        .sign_with_keys(author)
        .unwrap();
    assert!(matches!(
        db.accept_community_artifact(
            community,
            &principal(author),
            &event,
            &artifact::validate(&event).unwrap()
        )
        .await
        .unwrap(),
        ArtifactOutcome::Accepted(_)
    ));
    event
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn user_deletion_erases_community_artifact_payloads_without_resurrecting_heads() {
    let (db, community, _, alice, bob) = super::scrub_postgres_tests::setup().await;
    let other = db
        .ensure_configured_community(&format!("artifact-erasure-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let agent = Keys::generate();
    db.ensure_user(community, agent.public_key().as_bytes())
        .await
        .unwrap();
    db.set_agent_owner(
        community,
        agent.public_key().as_bytes(),
        alice.public_key().as_bytes(),
    )
    .await
    .unwrap();
    let shared = Uuid::new_v4();
    let erased_head = Uuid::new_v4();
    let agent_id = Uuid::new_v4();
    let alice_old = revision(&db, community, shared, &alice, None).await;
    let bob_head = revision(&db, community, shared, &bob, Some(&alice_old)).await;
    let bob_old = revision(&db, community, erased_head, &bob, None).await;
    let alice_head = revision(&db, community, erased_head, &alice, Some(&bob_old)).await;
    let agent_head = revision(&db, community, agent_id, &agent, None).await;
    let foreign = revision(&db, other, erased_head, &alice, None).await;
    // Previously moderated payloads still contain personal data and belong in
    // erasure, even though ordinary reads already suppress them.
    sqlx::query(
        "UPDATE artifact_revisions SET redacted=true WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(alice_old.id.as_bytes().as_slice())
    .execute(db.pool())
    .await
    .unwrap();
    let options = super::scrub_postgres_tests::options(community, &alice);
    let store = db.user_deletion_store();
    store
        .run_message_batches(&options, ExecutionMode::ReadOnly, None, &mut Vec::new())
        .await
        .unwrap();
    assert!(
        db.community_artifact_head(community, &principal(&bob), erased_head)
            .await
            .unwrap()
            .is_some(),
        "preview must not erase the current revision"
    );
    let summary = store
        .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    assert!(
        db.community_artifact_head(community, &principal(&bob), erased_head)
            .await
            .unwrap()
            .is_none(),
        "erasure must hide the author's current head without reviving Bob's old revision"
    );
    assert!(
        db.community_artifact_head(community, &principal(&bob), agent_id)
            .await
            .unwrap()
            .is_none(),
        "owned-agent payloads must be erased too"
    );
    assert_eq!(
        db.community_artifact_head(community, &principal(&bob), shared)
            .await
            .unwrap()
            .unwrap()
            .0
            .event
            .id,
        bob_head.id
    );
    let history = db
        .query_community_artifacts(
            community,
            &principal(&bob),
            &ArtifactQuery {
                scope: ArtifactScope::Community,
                view: ArtifactView::History,
                tags: vec![],
                limit: 100,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        history
            .iter()
            .map(|event| event.event.id)
            .collect::<HashSet<_>>(),
        [bob_head.id, bob_old.id].into_iter().collect()
    );
    assert_eq!(
        db.community_artifact_head(other, &principal(&bob), erased_head)
            .await
            .unwrap()
            .unwrap()
            .0
            .event,
        foreign,
        "erasure is confined to its selected community"
    );
    for event in [&alice_old, &alice_head, &agent_head] {
        let (payload, redacted): (Option<Value>, bool) = sqlx::query_as(
            "SELECT community_event,redacted FROM artifact_revisions WHERE community_id=$1 AND event_id=$2")
            .bind(community.as_uuid()).bind(event.id.as_bytes().as_slice())
            .fetch_one(db.pool()).await.unwrap();
        assert!(
            payload.is_none() && redacted,
            "erase payload but preserve acceptance receipt"
        );
        assert!(
            db.community_artifact_accepted(
                community,
                &principal(if event.pubkey == agent.public_key() {
                    &agent
                } else {
                    &alice
                }),
                event
            )
            .await
            .unwrap(),
            "exact accepted replay remains recognized"
        );
    }
    let current: Vec<u8> = sqlx::query_scalar(
        "SELECT event_id FROM artifact_heads WHERE community_id=$1 AND scope='community' AND artifact_id=$2")
        .bind(community.as_uuid()).bind(erased_head).fetch_one(db.pool()).await.unwrap();
    assert_eq!(
        current,
        alice_head.id.as_bytes().to_vec(),
        "the erased head remains the CAS predecessor"
    );
    assert_eq!(summary.items, 3);
    assert_eq!(
        store
            .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0,
        "erasure retry converges"
    );
}
