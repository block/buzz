use super::scrub_postgres_tests::{options, setup};
use super::*;
use crate::user_deletion::ExecutionMode;
use nostr::{EventBuilder, Keys, Kind, Tag};

async fn state(pool: &sqlx::PgPool, community: Uuid, table: &str) -> Vec<Value> {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT to_jsonb(t) FROM {table} t WHERE community_id=$1 ORDER BY to_jsonb(t)"
    )))
    .bind(community)
    .fetch_all(pool)
    .await
    .unwrap()
}

pub(super) struct InterruptAfterCommit(pub(super) Vec<u8>);
impl Write for InterruptAfterCommit {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for byte in bytes {
            if *byte == b'\n' {
                let value: Value = serde_json::from_slice(&self.0).unwrap();
                self.0.clear();
                if value["record"] == "batch_committed" {
                    return Err(std::io::Error::other("simulated lost output after commit"));
                }
            } else {
                self.0.push(*byte);
            }
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn private_dm_snapshot_cleanup_checks_subject_signer_and_resumes_without_shared_deletion() {
    let (db, community, channel, alice, bob) = setup().await;
    let relay = Keys::generate();
    let agent = Keys::generate();
    db.ensure_user(community, agent.public_key().as_bytes())
        .await
        .unwrap();
    sqlx::query("UPDATE users SET agent_owner_pubkey=$2 WHERE community_id=$1 AND pubkey=$3")
        .bind(community.as_uuid())
        .bind(alice.public_key().as_bytes().as_slice())
        .bind(agent.public_key().as_bytes().as_slice())
        .execute(db.pool())
        .await
        .unwrap();
    let other = db
        .ensure_configured_community(&format!("private-state-{}.example", Uuid::new_v4()))
        .await
        .unwrap()
        .id;
    let a = alice.public_key().to_hex();
    let b = bob.public_key().to_hex();
    let g = agent.public_key().to_hex();
    let mut selected = HashSet::new();
    let cases = [
        (&relay, vec![("d", &a), ("p", &a)], None, true),
        (&relay, vec![("d", &g), ("p", &g)], None, true),
        (&relay, vec![("d", &b), ("p", &b)], None, false),
        (&bob, vec![("d", &a), ("p", &a)], None, false),
        (&alice, vec![("d", &a), ("p", &a)], None, false),
        (&relay, vec![("d", &a), ("p", &b)], None, false),
        (&relay, vec![("d", &b), ("p", &a)], None, false),
        (&relay, vec![("d", &a), ("p", &a), ("p", &b)], None, false),
        (&relay, vec![("d", &a), ("d", &b), ("p", &a)], None, false),
        (&relay, vec![("d", &a), ("actor", &a)], None, false),
        (&relay, vec![("p", &a)], None, false),
        (&relay, vec![("d", &a), ("p", &a)], Some(channel), false),
    ];
    for (index, (signer, subject, scoped_channel, delete)) in cases.into_iter().enumerate() {
        let mut tags: Vec<Tag> = subject
            .into_iter()
            .map(|(name, value)| Tag::parse([name, value.as_str()]).unwrap())
            .collect();
        tags.push(Tag::parse(["h", &Uuid::new_v4().to_string()]).unwrap());
        let event = EventBuilder::new(Kind::Custom(30622), "")
            .tags(tags)
            .sign_with_keys(signer)
            .unwrap();
        // insert_event preserves historical/imported shapes; production snapshot
        // publishing uses the same d-tag extraction through replaceable storage.
        db.insert_event(community, &event, scoped_channel)
            .await
            .unwrap();
        db.insert_event(other, &event, None).await.unwrap();
        if delete {
            selected.insert(event.id.to_hex());
        }
        if index == 0 {
            sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND id=$2")
                .bind(community.as_uuid())
                .bind(event.id.as_bytes().as_slice())
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
    // Historical malformed JSON must be excluded without preventing valid
    // personal snapshots from being previewed and deleted.
    for extra in [json!(0), json!(["legacy", 0]), json!(["legacy", null])] {
        let event = EventBuilder::new(Kind::Custom(30622), Uuid::new_v4().to_string())
            .tags([
                Tag::parse(["d", &a]).unwrap(),
                Tag::parse(["p", &a]).unwrap(),
            ])
            .sign_with_keys(&relay)
            .unwrap();
        db.insert_event(community, &event, None).await.unwrap();
        sqlx::query("UPDATE events SET tags=$3 WHERE community_id=$1 AND id=$2")
            .bind(community.as_uuid())
            .bind(event.id.as_bytes().as_slice())
            .bind(json!([["d", a], ["p", a], extra]))
            .execute(db.pool())
            .await
            .unwrap();
    }
    // A target signature must not make a shared document/repository/wrapped DM
    // or another durable huddle record eligible for this private-state change.
    for kind in [40100, 45010, 30617, 30618, 1059, 48101] {
        let event = EventBuilder::new(Kind::Custom(kind), "shared content stays")
            .tags([Tag::parse(["d", &a]).unwrap()])
            .sign_with_keys(&alice)
            .unwrap();
        db.insert_event(community, &event, None).await.unwrap();
    }
    let tables = [
        "events",
        "event_mentions",
        "channels",
        "channel_members",
        "users",
    ];
    let mut before = Vec::new();
    let mut foreign = Vec::new();
    for table in tables {
        before.push(state(db.pool(), *community.as_uuid(), table).await);
        foreign.push(state(db.pool(), *other.as_uuid(), table).await);
    }
    let store = db.user_deletion_store();
    let mut opts = options(community, &alice);
    opts.batch_size = 1;
    assert_eq!(
        store
            .run_message_batches(&opts, ExecutionMode::ReadOnly, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
    opts.relay_pubkey = Some(relay.public_key().to_hex());
    let mut output = Vec::new();
    let preview = store
        .run_message_batches(&opts, ExecutionMode::ReadOnly, None, &mut output)
        .await
        .unwrap();
    assert!(preview
        .excluded_event_kinds
        .contains(&json!({"kind":30622,"count":1})));
    let records: Vec<Value> = std::str::from_utf8(&output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let selected_in_preview: HashSet<_> = records
        .iter()
        .filter_map(|r| r["items"].as_array())
        .flatten()
        .filter(|item| item["table"] == "events")
        .map(|item| item["key"]["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(selected_in_preview, selected);
    for (i, table) in tables.into_iter().enumerate() {
        assert_eq!(
            state(db.pool(), *community.as_uuid(), table).await,
            before[i]
        );
    }
    let error = store
        .run_message_batches(
            &opts,
            ExecutionMode::Delete,
            None,
            &mut InterruptAfterCommit(Vec::new()),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("simulated lost output"),
        "{error}"
    );
    let resumed = store
        .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(resumed.items + 1, preview.items);
    for (i, table) in tables.into_iter().enumerate() {
        assert_eq!(state(db.pool(), *other.as_uuid(), table).await, foreign[i]);
        let expected: Vec<Value> = before[i]
            .iter()
            .filter(|row| {
                let field = if table == "events" { "id" } else { "event_id" };
                !selected.contains(row[field].as_str().unwrap_or("").trim_start_matches("\\x"))
            })
            .cloned()
            .collect();
        assert_eq!(
            state(db.pool(), *community.as_uuid(), table).await,
            expected,
            "{table}"
        );
    }
    assert_eq!(
        store
            .run_message_batches(&opts, ExecutionMode::Delete, None, &mut Vec::new())
            .await
            .unwrap()
            .items,
        0
    );
}
