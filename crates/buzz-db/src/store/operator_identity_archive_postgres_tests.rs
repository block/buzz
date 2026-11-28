//! Operator archive of the confirmed, tenant-local NIP-IA identity set.

use buzz_core::CommunityId;
use sqlx::PgPool;
use uuid::Uuid;

use crate::operator_identity_archive::{ArchiveInventoryError, OperatorArchiveRequest};
use crate::Db;

const ROOT: [u8; 32] = [0x11; 32];
const AGENT: [u8; 32] = [0x22; 32];

async fn add_agent(pool: &PgPool, community: CommunityId, key: &[u8; 32], owner: &[u8; 32]) {
    sqlx::query("INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)")
        .bind(community.as_uuid())
        .bind(key.as_slice())
        .bind(owner.as_slice())
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn archive_confirms_direct_agents_and_ignores_workflow_changes() {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let db = Db::from_pool(pool.clone());
    let c = db
        .ensure_configured_community(&format!("archive-{}.example", Uuid::new_v4().simple()))
        .await
        .unwrap()
        .id;
    let original = db
        .admin_identity_inventory(c, &ROOT)
        .await
        .unwrap()
        .inventory_version
        .unwrap();
    add_agent(&pool, c, &AGENT, &ROOT).await;
    let actor = "a".repeat(64);
    let request = |confirmed_version| OperatorArchiveRequest {
        community: c,
        target: &ROOT,
        actor: &actor,
        confirmed_version,
        reason: None,
        replaced_by: None,
        configured_operators: &[],
    };
    assert!(matches!(
        db.archive_operator_identities(request(&original)).await,
        Err(ArchiveInventoryError::Changed)
    ));
    assert!(db.list_archived(c).await.unwrap().is_empty());

    let inventory = db.admin_identity_inventory(c, &ROOT).await.unwrap();
    let confirmed = inventory.inventory_version.unwrap();
    add_agent(&pool, c, &[0x33; 32], &AGENT).await;
    sqlx::query(
        "INSERT INTO workflows (community_id, owner_pubkey, name, definition, definition_hash) \
         SELECT $1, $2, 'example', '{}', $2 FROM generate_series(1,201)",
    )
    .bind(c.as_uuid())
    .bind(AGENT.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    let inventory = db.admin_identity_inventory(c, &ROOT).await.unwrap();
    assert!(inventory.workflows_truncated);
    assert_eq!(
        inventory.inventory_version.as_deref(),
        Some(confirmed.as_str())
    );

    let root = hex::encode(ROOT);
    let agent = hex::encode(AGENT);
    db.archive(c, &agent, "owner", &root, None, None, &"b".repeat(64))
        .await
        .unwrap();
    let replacement = "c".repeat(64);
    let changes = db
        .archive_operator_identities(OperatorArchiveRequest {
            reason: Some("duplicate"),
            replaced_by: Some(&replacement),
            ..request(&confirmed)
        })
        .await
        .unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].pubkey, root);
    assert!(changes[0].changed);
    assert_eq!(changes[1].pubkey, agent);
    assert!(!changes[1].changed);
    let rows = db.list_archived(c).await.unwrap();
    let root_row = rows.iter().find(|row| row.pubkey == root).unwrap();
    assert_eq!(root_row.consent_path, "relay");
    assert_eq!(root_row.actor, actor);
    assert_eq!(root_row.replaced_by.as_deref(), Some(replacement.as_str()));
    assert!(root_row.request_event_id.is_empty());
    let agent_row = rows.iter().find(|row| row.pubkey == agent).unwrap();
    assert_eq!(agent_row.consent_path, "owner");
    assert!(agent_row.replaced_by.is_none());
    assert!(!db.is_archived(c, &hex::encode([0x33; 32])).await.unwrap());

    sqlx::query(
        "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) \
         SELECT $1, decode(lpad(to_hex(i), 64, '0'), 'hex'), $2 FROM generate_series(1, 199) i",
    )
    .bind(c.as_uuid())
    .bind(ROOT.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    let inventory = db.admin_identity_inventory(c, &ROOT).await.unwrap();
    assert!(inventory.identities_truncated);
    assert!(inventory.inventory_version.is_none());
    assert!(matches!(
        db.archive_operator_identities(request(&confirmed)).await,
        Err(ArchiveInventoryError::TooLarge)
    ));
    assert_eq!(db.list_archived(c).await.unwrap().len(), 2);
}
