use super::*;
use crate::{
    channel::{ChannelType, ChannelVisibility},
    user_deletion::ExecutionMode,
    Db,
};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

#[tokio::test]
#[ignore = "requires Postgres"]
async fn preview_delete_and_retry_preserve_other_users_contributions() {
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let community = db
        .ensure_configured_community("content-ownership.example")
        .await
        .unwrap()
        .id;
    let alice = Keys::generate();
    let bob = Keys::generate();
    for author in [&alice, &bob] {
        db.ensure_user(community, author.public_key().as_bytes())
            .await
            .unwrap();
    }
    let channel = db
        .create_channel(
            community,
            "content-ownership",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            alice.public_key().as_bytes(),
            None,
        )
        .await
        .unwrap()
        .id;
    let original = EventBuilder::new(Kind::Custom(9), "Alice's original")
        .tags([Tag::parse(["h", &channel.to_string()]).unwrap()])
        .sign_with_keys(&alice)
        .unwrap();
    db.insert_event(community, &original, Some(channel))
        .await
        .unwrap();
    let mut authored = vec![original.clone()];
    let mut preserved = Vec::new();
    // Each author has a reaction, edit, vote and reply targeting the same root.
    for (index, author) in [&alice, &bob].into_iter().enumerate() {
        for kind in [7, 40003, 45002, 9] {
            let event = EventBuilder::new(Kind::Custom(kind), format!("{index}:{kind}"))
                .tags([
                    Tag::parse(["h", &channel.to_string()]).unwrap(),
                    Tag::parse(["e", &original.id.to_hex()]).unwrap(),
                    Tag::parse(["p", &alice.public_key().to_hex()]).unwrap(),
                ])
                .custom_created_at(Timestamp::from(original.created_at.as_secs() + 1))
                .sign_with_keys(author)
                .unwrap();
            db.insert_event(community, &event, Some(channel))
                .await
                .unwrap();
            if kind == 7 {
                sqlx::query("INSERT INTO reactions (community_id,event_created_at,event_id,pubkey,emoji,reaction_event_id)
                    VALUES ($1,to_timestamp($2),$3,$4,'+',$5)")
                    .bind(community.as_uuid()).bind(original.created_at.as_secs() as f64)
                    .bind(original.id.as_bytes().as_slice()).bind(author.public_key().as_bytes().as_slice())
                    .bind(event.id.as_bytes().as_slice()).execute(&pool).await.unwrap();
            }
            if index == 0 {
                authored.push(event);
            } else {
                preserved.push(event);
            }
        }
    }
    // These historical rows remain independently identifiable by their owner,
    // even when there is no signed reaction event to select.
    sqlx::query(
        "INSERT INTO reactions (community_id,event_created_at,event_id,pubkey,emoji)
        VALUES ($1,to_timestamp($2),$3,$4,'+')",
    )
    .bind(community.as_uuid())
    .bind(original.created_at.as_secs() as f64)
    .bind(preserved.last().unwrap().id.as_bytes().as_slice())
    .bind(alice.public_key().as_bytes().as_slice())
    .execute(&pool)
    .await
    .unwrap();
    for (name, author) in [("alice-sub", &alice), ("bob-sub", &bob)] {
        sqlx::query("INSERT INTO subscriptions (community_id,id,owner_pubkey) VALUES ($1,$2,$3)")
            .bind(community.as_uuid())
            .bind(name)
            .bind(author.public_key().as_bytes().as_slice())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO delivery_log (community_id,subscription_id,event_id) VALUES ($1,$2,$3)",
        )
        .bind(community.as_uuid())
        .bind(name)
        .bind(original.id.as_bytes().as_slice())
        .execute(&pool)
        .await
        .unwrap();
    }
    for event in std::iter::once(&original)
        .chain(authored.iter().skip(1))
        .chain(&preserved)
    {
        if event.kind != Kind::Custom(9) {
            continue;
        }
        let is_root = event.id == original.id;
        sqlx::query("INSERT INTO thread_metadata (community_id,event_created_at,event_id,channel_id,parent_event_id,root_event_id,depth)
            VALUES ($1,to_timestamp($2),$3,$4,$5,$5,$6)")
            .bind(community.as_uuid()).bind(event.created_at.as_secs() as f64)
            .bind(event.id.as_bytes().as_slice()).bind(channel)
            .bind((!is_root).then(|| original.id.as_bytes().to_vec()))
            .bind(if is_root { 0_i32 } else { 1 }).execute(&pool).await.unwrap();
    }
    // Even an owned workflow can contain someone else's approval. Neither its
    // ownership nor Alice's trigger message authorizes deleting that history.
    for author in [&alice, &bob] {
        let workflow = Uuid::new_v4();
        let run = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO workflows (community_id,id,name,owner_pubkey,definition,definition_hash)
            VALUES ($1,$2,'retained workflow',$3,'{}',$4)",
        )
        .bind(community.as_uuid())
        .bind(workflow)
        .bind(author.public_key().as_bytes().as_slice())
        .bind(vec![0_u8; 32])
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO workflow_runs (community_id,id,workflow_id,status,trigger_event_id,trigger_context)
            VALUES ($1,$2,$3,'completed',$4,$5)")
            .bind(community.as_uuid()).bind(run).bind(workflow).bind(original.id.as_bytes().as_slice())
            .bind(json!({"author": alice.public_key().to_hex(), "message_id": original.id.to_hex()}))
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workflow_approvals (community_id,token,workflow_id,run_id,step_id,step_index,approver_spec,status,approver_pubkey,expires_at)
            VALUES ($1,$2,$3,$4,'review',0,'reviewer','granted',$5,now()+interval '1 day')")
            .bind(community.as_uuid()).bind(run.as_bytes().as_slice()).bind(workflow).bind(run)
            .bind(bob.public_key().as_bytes().as_slice()).execute(&pool).await.unwrap();
    }
    let options = ContentBatchOptions {
        community: *community.as_uuid(),
        pubkey: alice.public_key().to_hex(),
        relay_pubkey: None,
        batch_size: 1,
        max_items: 1000,
    };
    let store = db.user_deletion_store();
    let mut preview = Vec::new();
    store
        .run_message_batches(&options, ExecutionMode::ReadOnly, None, &mut preview)
        .await
        .unwrap();
    let records: Vec<Value> = std::str::from_utf8(&preview)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let event_ids: HashSet<String> = records
        .iter()
        .filter_map(|record| record["items"].as_array())
        .flatten()
        .filter(|item| item["table"] == "events")
        .map(|item| item["key"]["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        event_ids,
        authored.iter().map(|event| event.id.to_hex()).collect()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1")
        .bind(community.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count as usize, authored.len() + preserved.len());

    store
        .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    let remaining: Vec<Vec<u8>> = sqlx::query_scalar("SELECT id FROM events WHERE community_id=$1")
        .bind(community.as_uuid())
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(
        remaining.into_iter().collect::<HashSet<_>>(),
        preserved
            .iter()
            .map(|event| event.id.as_bytes().to_vec())
            .collect()
    );
    for event in &preserved {
        let (pubkey, tags, content, sig): (Vec<u8>, Value, String, Vec<u8>) = sqlx::query_as(
            "SELECT pubkey,tags,content,sig FROM events WHERE community_id=$1 AND id=$2",
        )
        .bind(community.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pubkey, event.pubkey.as_bytes().to_vec());
        assert_eq!(tags, serde_json::to_value(&event.tags).unwrap());
        assert_eq!(content, event.content);
        assert_eq!(sig, event.sig.serialize().to_vec());
    }
    let reaction_authors: Vec<Vec<u8>> =
        sqlx::query_scalar("SELECT pubkey FROM reactions WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(reaction_authors, vec![bob.public_key().as_bytes().to_vec()]);
    let deliveries: Vec<String> =
        sqlx::query_scalar("SELECT subscription_id FROM delivery_log WHERE community_id=$1")
            .bind(community.as_uuid())
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(deliveries, vec!["bob-sub"]);
    let reply_count: i32 = sqlx::query_scalar(
        "SELECT reply_count FROM thread_metadata WHERE community_id=$1 AND event_id=$2",
    )
    .bind(community.as_uuid())
    .bind(original.id.as_bytes().as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reply_count, 1, "retain ancestry for Bob's surviving reply");
    for table in ["workflows", "workflow_runs", "workflow_approvals"] {
        let query = format!("SELECT count(*) FROM {table} WHERE community_id=$1");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(query))
            .bind(community.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2, "{table} must be retained");
    }
    let retry = store
        .run_message_batches(&options, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(retry.items, 0);
}
