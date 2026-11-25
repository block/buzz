use super::*;

async fn add_agent_owner(f: &media_fixture::Fixture, community: Uuid, human: &str) -> nostr::Keys {
    let keys = nostr::Keys::generate();
    let agent = keys.public_key().to_hex();
    sqlx::query("INSERT INTO users(community_id,pubkey,agent_owner_pubkey) VALUES($1,decode($2,'hex'),decode($3,'hex'))")
        .bind(community).bind(&agent).bind(human).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'owner')")
        .bind(community)
        .bind(&agent)
        .execute(&f.pool)
        .await
        .unwrap();
    keys
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn agent_owned_communities_are_listed_once_and_need_separate_approval() {
    let f = media_fixture::fixture().await;
    let subject = nostr::Keys::generate().public_key().to_hex();
    let users = f.options().db.user_deletion_store();
    let communities = [Uuid::from_u128(11), Uuid::from_u128(12)];
    // Only the first community has an ordinary membership for the human.
    sqlx::query("INSERT INTO relay_members(community_id,pubkey,role) VALUES($1,$2,'member')")
        .bind(communities[0])
        .bind(&subject)
        .execute(&f.pool)
        .await
        .unwrap();
    for id in communities {
        add_agent_owner(&f, id, &subject).await;
    }
    // The same agent key owns a third community where the human has no ownership
    // record. Authority in one community must never select another community.
    let unrelated = Uuid::new_v4();
    sqlx::query("INSERT INTO communities(id,host) VALUES($1,'unrelated.agent-owner.example')")
        .bind(unrelated)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO relay_members(community_id,pubkey,role)
        SELECT $1,pubkey,'owner' FROM relay_members WHERE community_id=$2 AND role='owner'",
    )
    .bind(unrelated)
    .bind(communities[0])
    .execute(&f.pool)
    .await
    .unwrap();
    let preview = users.generate_inventory(&subject).await.unwrap();
    let rows = preview["communities"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for (id, row) in communities.iter().zip(rows) {
        assert_eq!(row["community_id"], id.to_string());
        assert_eq!(row["relationship"], "owner");
        assert_eq!(row["owned_by_agent"], true);
    }
    for delete in [false, true] {
        let request = users
            .submit_for_mode(&subject, "operator", None, !delete)
            .await
            .unwrap();
        assert_eq!(request.inventory_manifest.as_ref(), Some(&preview));
        users
            .approve(
                request.id,
                "approver",
                request.inventory_digest.as_deref().unwrap(),
            )
            .await
            .unwrap();
        run(
            &f.store,
            request.id,
            "worker",
            &RunOptions {
                delete,
                ..f.options()
            },
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        let saved = f.store.inspect(request.id).await.unwrap();
        let stages: Vec<_> = saved
            .stage_history
            .iter()
            .filter(|row| row.target_community_id.is_some())
            .collect();
        assert_eq!(stages.len(), 2);
        assert!(stages
            .iter()
            .all(|row| row.stage == "delete_owned_community"));
        for id in communities {
            let child = f
                .options()
                .db
                .deletion_store()
                .find_for_community(buzz_core::CommunityId::from_uuid(id))
                .await
                .unwrap();
            if delete {
                assert_eq!(
                    child.unwrap().stage,
                    buzz_db::deletion::DeletionStage::Inventoried
                );
            } else {
                assert!(child.is_none());
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn agent_ownership_change_blocks_media_and_preserves_the_message() {
    assert_stale_participant_stops(false).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn agent_ownership_change_after_media_preserves_the_message() {
    assert_stale_participant_stops(true).await;
}

async fn assert_stale_participant_stops(after_media: bool) {
    let f = media_fixture::fixture().await;
    if after_media {
        let options = RunOptions {
            next_step: true,
            ..f.options()
        };
        run(
            &f.store,
            f.id,
            "worker",
            &options,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    }
    let human = f.store.get(f.id).await.unwrap().target;
    let community = Uuid::from_u128(11);
    let agent = add_agent_owner(&f, community, &human).await;
    let message = nostr::EventBuilder::new(nostr::Kind::Custom(9), "agent message")
        .sign_with_keys(&agent)
        .unwrap();
    f.options()
        .db
        .insert_event(buzz_core::CommunityId::from_uuid(community), &message, None)
        .await
        .unwrap();
    let error = run(
        &f.store,
        f.id,
        "worker",
        &f.options(),
        &CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("differs from the approved inventory"));
    if !after_media {
        assert_eq!(f.requests.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(f.uploader_count().await, 1);
    }
    let messages: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id=$1 AND pubkey=decode($2,'hex')",
    )
    .bind(community)
    .bind(agent.public_key().to_hex())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(messages, 1);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn loss_of_ownership_after_media_prevents_a_new_community_request() {
    let f = media_fixture::fixture_with_roles(&["owner"]).await;
    let community = Uuid::from_u128(11);
    let one = RunOptions {
        next_step: true,
        ..f.options()
    };
    run(&f.store, f.id, "worker", &one, &CancellationToken::new())
        .await
        .unwrap();
    sqlx::query("UPDATE relay_members SET role='member' WHERE community_id=$1")
        .bind(community)
        .execute(&f.pool)
        .await
        .unwrap();
    let error = run(&f.store, f.id, "worker", &one, &CancellationToken::new())
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("differs from the approved inventory"));
    assert!(one
        .db
        .deletion_store()
        .find_for_community(buzz_core::CommunityId::from_uuid(community))
        .await
        .unwrap()
        .is_none());
}
