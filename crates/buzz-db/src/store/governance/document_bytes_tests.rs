use super::*;
use crate::store::audit::{self, Append};
use buzz_core::governance::{command_builder, MAX_DOCUMENT_BYTES};
use serde_json::json;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn floor_at_document_byte_limit_can_be_read_and_replaced() {
    let db = Db::new(&crate::DbConfig {
        database_url: crate::test_support::database_url(),
        ..Default::default()
    })
    .await
    .expect("connect");
    let community = CommunityId::from_uuid(Uuid::new_v4());
    let admin = Keys::generate();
    let relay = Keys::generate();
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(format!("governance-bytes-{community}.example.org"))
        .execute(db.pool())
        .await
        .expect("community");
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'admin')")
        .bind(community.as_uuid())
        .bind(admin.public_key().to_hex())
        .execute(db.pool())
        .await
        .expect("community admin");

    let document = DocumentId::Floor;
    let content = floor_at_byte_limit();
    let original = Document::parse(&content, &document).expect("valid floor at the byte limit");
    let first = apply_floor(
        &db,
        community,
        &admin,
        &relay,
        ExpectedHead::Absent,
        &content,
    )
    .await;

    let mut tx = db
        .begin_event_write_transaction(community)
        .await
        .expect("body read transaction");
    let read = body(&mut tx, &document)
        .await
        .expect("accepted floor body remains within the byte limit");
    assert!(read == Some(original), "floor body changed");
    tx.rollback().await.expect("finish body read transaction");

    let read_head = db
        .governance_head(
            community,
            &document,
            admin.public_key().as_bytes(),
            None,
            false,
        )
        .await
        .expect("read floor head")
        .expect("floor head exists");
    read_head.verify().expect("valid signed floor head");
    assert_eq!(read_head.id, first.id);
    assert!(read_head.content == content, "floor content bytes changed");

    let replacement = apply_floor(
        &db,
        community,
        &admin,
        &relay,
        ExpectedHead::Event(read_head.id.to_hex()),
        &read_head.content,
    )
    .await;
    let current = db
        .governance_head(
            community,
            &document,
            admin.public_key().as_bytes(),
            None,
            false,
        )
        .await
        .expect("read replacement floor head")
        .expect("replacement floor head exists");
    current
        .verify()
        .expect("valid signed replacement floor head");
    assert_eq!(current.id, replacement.id);
    assert_ne!(current.id, read_head.id);
    assert!(
        current.content == content,
        "replacement content bytes changed"
    );
}

fn floor_at_byte_limit() -> String {
    let segment = "x".repeat(237);
    let refs: Vec<_> = (0..32)
        .map(|index| format!("refs/heads/{segment}{index:02}"))
        .collect();
    let rules: Vec<_> = (0..8)
        .map(|index| {
            json!({
                "id": format!("rule-{index}"),
                "refs": refs,
                "labels": {},
                "require": [{"type": "review", "count": 1}]
            })
        })
        .collect();
    let mut document = json!({
        "version": 1,
        "body": {
            "type": "floor",
            "rules": rules,
            "labels": {"service": {"values": [""]}},
            "max_grant_seconds": 600
        }
    });
    // Fill the remaining bytes with a real label value so reserialization cannot trim them.
    let remaining = MAX_DOCUMENT_BYTES - document.to_string().len();
    document["body"]["labels"]["service"]["values"][0] = json!("x".repeat(remaining));
    let content = document.to_string();
    assert_eq!(content.len(), MAX_DOCUMENT_BYTES);
    content
}

async fn apply_floor(
    db: &Db,
    community: CommunityId,
    admin: &Keys,
    relay: &Keys,
    expected: ExpectedHead,
    content: &str,
) -> Event {
    let document = DocumentId::Floor;
    let command = command_builder(*community.as_uuid(), &document, &expected, content)
        .expect("build floor command")
        .sign_with_keys(admin)
        .expect("sign floor command");
    let mut tx = db
        .begin_event_write_transaction(community)
        .await
        .expect("floor write transaction");
    fence_authority_write(&mut tx, &document)
        .await
        .expect("fence floor write");
    let prepared = db
        .prepare_governance_command(&mut tx, &command, None, relay, false)
        .await
        .expect("prepare floor command");
    let (seq, prev_hash) = audit::lock_head(&mut tx)
        .await
        .expect("lock audit head")
        .map_or((1, None), |(seq, hash)| (seq + 1, Some(hash)));
    let command_id = command.id.to_hex();
    audit::append(
        &mut tx,
        Append {
            seq,
            hash_version: 2,
            hash: &seq.to_be_bytes().repeat(4),
            prev_hash: prev_hash.as_deref(),
            action: "governance_changed",
            actor: Some(admin.public_key().as_bytes()),
            object: Some(&command_id),
            detail: &json!({
                "document": document.to_string(),
                "command_id": command_id,
                "previous": prepared.previous,
                "state": prepared.state
            }),
            created_at: chrono::Utc::now(),
            valid_until: prepared.valid_until,
        },
    )
    .await
    .expect("append floor audit row");
    tx.commit().await.expect("commit floor write");
    prepared.state
}
