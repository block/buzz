//! Exercise the real operator command, including aggregate failure status.
use buzz_core::{
    channel::{ChannelType, ChannelVisibility},
    TenantContext,
};
use buzz_db::{Db, DbConfig};
use nostr::{EventBuilder, Keys, Kind};
use uuid::Uuid;

async fn fixture() -> (Db, TenantContext, Keys) {
    let db = Db::new(&DbConfig {
        database_url: std::env::var("BUZZ_TEST_DATABASE_URL")
            .expect("isolated postgres-ci database"),
        ..DbConfig::default()
    })
    .await
    .unwrap();
    let host = format!("repair-{}.example", Uuid::new_v4());
    let community = db.ensure_configured_community(&host).await.unwrap().id;
    (
        db,
        TenantContext::resolved(community, host),
        Keys::generate(),
    )
}

async fn create(db: &Db, tenant: &TenantContext, id: Uuid, keys: &Keys) {
    db.create_channel_with_id(
        tenant.community(),
        id,
        "repair",
        ChannelType::Stream,
        ChannelVisibility::Open,
        None,
        keys.public_key().as_bytes(),
        None,
    )
    .await
    .unwrap();
}

async fn run(tenant: &TenantContext, keys: &Keys, channel: Option<Uuid>) -> std::process::Output {
    let binary = std::env::var_os("NEXTEST_BIN_EXE_buzz_admin")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_buzz-admin").into());
    let mut command = tokio::process::Command::new(binary);
    command
        .arg("reconcile-channels")
        .env_clear()
        .env(
            "DATABASE_URL",
            std::env::var("BUZZ_TEST_DATABASE_URL").unwrap(),
        )
        .env("RELAY_URL", format!("wss://{}", tenant.host()))
        .env("BUZZ_RELAY_PRIVATE_KEY", keys.secret_key().to_secret_hex())
        .kill_on_drop(true);
    if let Some(id) = channel {
        command.args(["--channel", &id.to_string()]);
    }
    tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
        .await
        .unwrap()
        .unwrap()
}

async fn discovery(db: &Db, tenant: &TenantContext, channel: Uuid) -> Vec<(i32, Vec<u8>)> {
    sqlx::query_as("SELECT kind, id FROM events WHERE community_id=$1 AND channel_id=$2 AND kind IN (39000,39001,39002) AND deleted_at IS NULL ORDER BY kind")
        .bind(tenant.community().as_uuid()).bind(channel).fetch_all(db.pool()).await.unwrap()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn current_metadata_does_not_skip_missing_auxiliary_discovery() {
    let (db, tenant, keys) = fixture().await;
    let channel = Uuid::new_v4();
    create(&db, &tenant, channel, &keys).await;
    let mut write = db
        .begin_channel_metadata_write(tenant.community(), channel, keys.public_key())
        .await
        .unwrap();
    let event = EventBuilder::new(Kind::Custom(39000), "")
        .tags(write.snapshot_tags().await.unwrap())
        .custom_created_at(write.snapshot_timestamp().unwrap())
        .sign_with_keys(&keys)
        .unwrap();
    write.store_snapshot(&event, 512 * 1024).await.unwrap();
    write.commit().await.unwrap();
    // Both missing, then each missing independently. Preserve the current head.
    for missing in [None, Some(39001), Some(39002)] {
        if let Some(kind) = missing {
            // Include retired heads, even when repair happens in the same second.
            sqlx::query("UPDATE events SET deleted_at=NOW() WHERE community_id=$1 AND channel_id=$2 AND kind=$3")
                .bind(tenant.community().as_uuid())
                .bind(channel)
                .bind(kind)
                .execute(db.pool())
                .await
                .unwrap();
        }
        let output = run(&tenant, &keys, None).await;
        assert!(output.status.success(), "{:?}", output);
        let rows = discovery(&db, &tenant, channel).await;
        assert_eq!(
            rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            [39000, 39001, 39002]
        );
        assert_eq!(rows[0].1, event.id.as_bytes());
    }
    let before = discovery(&db, &tenant, channel).await;
    let output = run(&tenant, &keys, Some(channel)).await;
    assert!(output.status.success(), "{:?}", output);
    let after = discovery(&db, &tenant, channel).await;
    assert_eq!(&before[..2], &after[..2], "targeted repair is roster-only");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repair_finishes_scan_but_exits_unsuccessfully_after_a_channel_error() {
    let (db, tenant, keys) = fixture().await;
    let broken = Uuid::from_u128(1);
    let healthy = Uuid::from_u128(2);
    for id in [broken, healthy] {
        create(&db, &tenant, id, &keys).await;
    }
    sqlx::query(
        "UPDATE channels SET labels=ARRAY['INVALID']::text[] WHERE community_id=$1 AND id=$2",
    )
    .bind(tenant.community().as_uuid())
    .bind(broken)
    .execute(db.pool())
    .await
    .unwrap();
    let output = run(&tenant, &keys, None).await;
    assert!(!output.status.success(), "must report incomplete repair");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 failed"),
        "{:?}",
        output
    );
    assert_eq!(
        discovery(&db, &tenant, healthy).await.len(),
        3,
        "later channels still repaired"
    );
    assert!(discovery(&db, &tenant, broken).await.is_empty());
}

// Same public-coordinate encoding as the production replacement protocol. The
// test takes its lock externally to stop the real binary before roster capture.
fn replacement_key(tenant: &TenantContext, channel: Uuid, keys: &Keys, kind: i32) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in tenant
        .community()
        .as_uuid()
        .as_bytes()
        .iter()
        .chain(kind.to_le_bytes().iter())
        .chain(keys.public_key().as_bytes().iter())
        .chain(channel.as_bytes().iter())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as i64
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repair_captures_rosters_after_waiting_for_the_replacement_lock() {
    let (db, tenant, keys) = fixture().await;
    for kind in [39001, 39002] {
        let channel = Uuid::new_v4();
        create(&db, &tenant, channel, &keys).await;
        assert!(run(&tenant, &keys, None).await.status.success());
        // Full repair must refill 39001; targeted repair always replaces 39002.
        if kind == 39001 {
            sqlx::query("UPDATE events SET deleted_at=now() WHERE community_id=$1 AND channel_id=$2 AND kind=$3")
                .bind(tenant.community().as_uuid()).bind(channel).bind(kind)
                .execute(db.pool()).await.unwrap();
        }
        let mut holder = db.pool().begin().await.unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(replacement_key(&tenant, channel, &keys, kind))
            .execute(&mut *holder)
            .await
            .unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *holder)
            .await
            .unwrap();
        let newcomer = Keys::generate();
        let change = async {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    let blocked: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)))")
                        .bind(pid).fetch_one(db.pool()).await.unwrap();
                    if blocked { break; }
                    tokio::task::yield_now().await;
                }
            }).await.expect("actual operator must wait behind replacement lock");
            db.add_member(
                tenant.community(),
                channel,
                newcomer.public_key().as_bytes(),
                buzz_core::channel::MemberRole::Admin,
                Some(keys.public_key().as_bytes()),
            )
            .await
            .unwrap();
            holder.rollback().await.unwrap();
        };
        let target = (kind == 39002).then_some(channel);
        let (output, ()) = tokio::join!(run(&tenant, &keys, target), change);
        assert!(output.status.success(), "{output:?}");
        let tags: serde_json::Value = sqlx::query_scalar("SELECT tags FROM events WHERE community_id=$1 AND channel_id=$2 AND kind=$3 AND deleted_at IS NULL")
            .bind(tenant.community().as_uuid()).bind(channel).bind(kind).fetch_one(db.pool()).await.unwrap();
        for member in [keys.public_key(), newcomer.public_key()] {
            assert!(
                tags.as_array()
                    .unwrap()
                    .iter()
                    .any(|tag| tag[0] == "p" && tag[1] == member.to_hex()),
                "kind {kind} must include fresh roster and signer self-membership: {tags}"
            );
        }
    }
}
