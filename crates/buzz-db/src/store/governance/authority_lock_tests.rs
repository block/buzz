//! Commands must decide with the authority in force after a concurrent revocation.
use super::*;
use std::time::Duration;

const EMPTY_FLOOR: &str =
    r#"{"version":1,"body":{"type":"floor","rules":[],"labels":{},"max_grant_seconds":3600}}"#;

async fn tx(f: &Fixture) -> AdmittedTx {
    f.db.begin_event_write_transaction(f.community)
        .await
        .expect("begin transaction")
}

async fn prepare(
    f: &Fixture,
    mut tx: AdmittedTx,
    event: &Event,
    grant: Option<&GitGrant>,
) -> Result<Prepared> {
    let command = Command::parse(event).expect("parse command");
    fence_authority_write(&mut tx, &command.document)
        .await
        .expect("fence authority");
    let result =
        f.db.prepare_governance_command(&mut tx, event, grant, &f.owner, true)
            .await;
    tx.rollback().await.expect("roll back command");
    result
}

fn command(f: &Fixture, id: &DocumentId, content: &str) -> Event {
    buzz_core::governance::command_builder(
        *f.community.as_uuid(),
        id,
        &ExpectedHead::Absent,
        content,
    )
    .expect("build command")
    .sign_with_keys(&f.agent)
    .expect("sign command")
}

async fn assert_waits_for_writer(
    f: &Fixture,
    mut writer: AdmittedTx,
    event: &Event,
    grant: Option<&GitGrant>,
    revoked: bool,
) {
    let mut command_tx = tx(f).await;
    let pid = backend_pid(&mut command_tx).await;
    let pending = prepare(f, command_tx, event, grant);
    tokio::pin!(pending);
    tokio::select! {
        result = &mut pending => {
            panic!("command completed before its authority writer committed: {:?}", result.err());
        }
        () = wait_for_blocking_pid(&mut writer, pid) => {}
    }
    writer.commit().await.expect("commit authority change");
    let result = tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("command completes after its authority writer commits");
    if revoked {
        assert!(
            matches!(result, Err(DbError::AccessDenied(_))),
            "command must see committed revocation: {:?}",
            result.err()
        );
    } else {
        result.expect("command remains authorized");
    }
}

async fn backend_pid(tx: &mut AdmittedTx) -> i32 {
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(tx.conn())
        .await
        .expect("backend PID")
}

async fn wait_for_blocking_pid(blocker: &mut AdmittedTx, pid: i32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool =
                sqlx::query_scalar("SELECT pg_backend_pid() = ANY(pg_blocking_pids($1))")
                    .bind(pid)
                    .fetch_one(blocker.conn())
                    .await
                    .expect("inspect blocked transaction");
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("transaction waits for the expected blocker");
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn floor_command_waits_for_signer_demotion() {
    assert_floor_waits_for_demotion(false).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn floor_command_waits_for_owner_demotion() {
    assert_floor_waits_for_demotion(true).await;
}

async fn add_community_admins(f: &Fixture) {
    for key in [&f.owner, &f.agent] {
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'admin')",
        )
        .bind(f.community.as_uuid())
        .bind(key.public_key().to_hex())
        .execute(f.db.pool())
        .await
        .expect("insert community admin");
    }
}

async fn assert_floor_waits_for_demotion(demote_owner: bool) {
    let f = Fixture::new().await;
    add_community_admins(&f).await;
    let event = command(&f, &DocumentId::Floor, EMPTY_FLOOR);
    prepare(&f, tx(&f).await, &event, None)
        .await
        .expect("both keys administer the floor before demotion");
    let mut writer = tx(&f).await;
    let target = if demote_owner { &f.owner } else { &f.agent };
    let changed = sqlx::query(
        "UPDATE relay_members SET role = 'member' WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(f.community.as_uuid())
    .bind(target.public_key().to_hex())
    .execute(writer.conn())
    .await
    .expect("demote community admin");
    assert_eq!(changed.rows_affected(), 1);
    assert_waits_for_writer(&f, writer, &event, None, true).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn reciprocal_owner_removal_waits_for_governance_command_commit() {
    let f = Fixture::new().await;
    add_community_admins(&f).await;
    let signer = f.agent.public_key();
    let signer_hex = signer.to_hex();
    let recorded_owner = f.owner.public_key();
    let linked = sqlx::query(
        "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3)",
    )
    .bind(f.community.as_uuid())
    .bind(recorded_owner.as_bytes())
    .bind(signer.as_bytes())
    .execute(f.db.pool())
    .await
    .expect("record the reciprocal owner");
    assert_eq!(linked.rows_affected(), 1);

    // Pause authorization after it locks the recorded owner's role, while it
    // waits for the signer's role. Cascading removal locks those rows in the
    // opposite order, so it must wait for the command before taking either.
    let mut holder = tx(&f).await;
    let holder_pid = backend_pid(&mut holder).await;
    sqlx::query("SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR UPDATE")
        .bind(f.community.as_uuid())
        .bind(&signer_hex)
        .fetch_one(holder.conn())
        .await
        .expect("hold the signer's roster row");
    let mut command_tx = tx(&f).await;
    let command_pid = backend_pid(&mut command_tx).await;

    let removal_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&crate::test_support::database_url())
        .await
        .expect("connect member removal");
    let removal_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&removal_pool)
        .await
        .expect("member removal PID");
    let removal = crate::relay_members::remove_relay_member(
        &removal_pool,
        f.community,
        &signer_hex,
        &f.owner,
    );
    tokio::pin!(removal);

    let event = command(&f, &DocumentId::Floor, EMPTY_FLOOR);
    let parsed = Command::parse(&event).expect("parse floor command");
    {
        let authorization = async {
            fence_authority_write(&mut command_tx, &parsed.document)
                .await
                .expect("fence authority");
            authorize(&mut command_tx, &parsed, signer.as_bytes(), None, true)
                .await
                .expect("authorize floor while both roles remain admins");
        };
        tokio::pin!(authorization);
        tokio::select! {
            () = &mut authorization => panic!("authorization completed while the signer row was held"),
            () = wait_for_blocking_pid(&mut holder, command_pid) => {}
        }
        let first_blocker = async {
            loop {
                let waiting: Option<(String, Vec<i32>)> = sqlx::query_as(
                    "SELECT locktype, pg_blocking_pids(pid) FROM pg_locks \
                     WHERE pid=$1 AND NOT granted",
                )
                .bind(removal_pid)
                .fetch_optional(holder.conn())
                .await
                .expect("inspect blocked member removal");
                if let Some(blocked) = waiting {
                    break blocked;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::select! {
            _ = &mut removal => panic!("removal completed while the signer row was held"),
            result = tokio::time::timeout(Duration::from_secs(5), first_blocker) => {
                let (locktype, blockers) = result.expect("member removal waits for authority");
                assert_eq!(
                    (locktype.as_str(), blockers.as_slice()),
                    ("advisory", &[command_pid][..]),
                    "member removal must wait on the command before reaching roster row holder {holder_pid}"
                );
            }
        }
        holder.rollback().await.expect("release signer's role");
        tokio::time::timeout(Duration::from_secs(5), authorization)
            .await
            .expect("authorization completes after release");
    }
    tokio::select! {
        _ = &mut removal => panic!("removal completed before command commit"),
        () = wait_for_blocking_pid(&mut command_tx, removal_pid) => {}
    }
    command_tx
        .commit()
        .await
        .expect("commit authorized transaction");

    let removed = tokio::time::timeout(Duration::from_secs(5), removal)
        .await
        .expect("removal finishes after command commit")
        .expect("remove reciprocal memberships");
    assert_eq!(removed.outcome, crate::relay_members::RemoveResult::Removed);
    let remaining: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM relay_members WHERE community_id=$1 AND pubkey=ANY($2)",
    )
    .bind(f.community.as_uuid())
    .bind(&[signer_hex.clone(), recorded_owner.to_hex()][..])
    .fetch_one(f.db.pool())
    .await
    .expect("count remaining reciprocal memberships");
    assert_eq!(remaining, 0);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn bind_command_waits_for_source_demotion() {
    assert_bind_waits_for_demotion(true).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn bind_command_waits_for_destination_demotion() {
    assert_bind_waits_for_demotion(false).await;
}

async fn assert_bind_waits_for_demotion(demote_source: bool) {
    let f = Fixture::new().await;
    f.role(f.channel, &f.owner, Some("admin")).await;
    f.role(f.next, &f.agent, Some("admin")).await;
    let grant = f.grant(GrantAction::Manage);
    let event = command(
        &f,
        &DocumentId::Bind(f.repo.clone()),
        &serde_json::json!({"version":1,"body":{"type":"bind","channel":f.next}}).to_string(),
    );
    prepare(&f, tx(&f).await, &event, Some(&grant))
        .await
        .expect("bind is authorized in both channels before demotion");
    let mut writer = tx(&f).await;
    let (channel, target) = if demote_source {
        (f.channel, &f.owner)
    } else {
        (f.next, &f.agent)
    };
    crate::channel_members::lock_channel_for_member_put(&mut writer, channel)
        .await
        .expect("lock channel membership write");
    let changed = sqlx::query(
        "UPDATE channel_members SET role = 'member' \
             WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
    )
    .bind(f.community.as_uuid())
    .bind(channel)
    .bind(target.public_key().as_bytes())
    .execute(writer.conn())
    .await
    .expect("demote channel admin");
    assert_eq!(changed.rows_affected(), 1);
    assert_waits_for_writer(&f, writer, &event, Some(&grant), true).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repo_command_waits_for_owner_ban() {
    let f = Fixture::new().await;
    f.role(f.channel, &f.agent, Some("admin")).await;
    let event = command(
        &f,
        &DocumentId::Rules(f.repo.clone()),
        r#"{"version":1,"body":{"type":"rules","rules":[]}}"#,
    );
    prepare(&f, tx(&f).await, &event, None)
        .await
        .expect("agent directly administers the repo before its owner is banned");
    let mut writer = tx(&f).await;
    crate::relay_invite::lock_admission(writer.conn(), f.community)
        .await
        .expect("lock admission writer");
    let ban = sqlx::query(
        "INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) \
         VALUES ($1, $2, true, $3)",
    )
    .bind(f.community.as_uuid())
    .bind(f.owner.public_key().as_bytes())
    .bind(Keys::generate().public_key().as_bytes())
    .execute(writer.conn())
    .await
    .expect("ban agent owner");
    assert_eq!(ban.rows_affected(), 1);
    assert_waits_for_writer(&f, writer, &event, None, true).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repo_command_fences_channel_row_through_authorization() {
    let f = Fixture::new().await;
    f.role(f.channel, &f.agent, Some("admin")).await;
    let event = command(
        &f,
        &DocumentId::Rules(f.repo.clone()),
        r#"{"version":1,"body":{"type":"rules","rules":[]}}"#,
    );
    let mut writer = tx(&f).await;
    // Archive changes the channel row without changing membership or write
    // authority. The command must wait for the row, then still authorize.
    let archive = sqlx::query(
        "UPDATE channels SET archived_at = clock_timestamp() WHERE community_id = $1 AND id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(f.channel)
    .execute(writer.conn())
    .await
    .expect("archive channel");
    assert_eq!(archive.rows_affected(), 1);
    assert_waits_for_writer(&f, writer, &event, None, false).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn multi_channel_membership_writer_takes_locks_in_uuid_order() {
    use crate::channel_members::{
        acquire_channel_membership_lock, lock_channel_memberships_in_transaction,
    };

    let f = Fixture::new().await;
    let mut channels = [f.channel, f.next];
    channels.sort_unstable();
    let [first, second] = channels;
    let mut holder = tx(&f).await;
    acquire_channel_membership_lock(holder.conn(), f.community, first)
        .await
        .expect("hold the first channel");

    let mut writer = tx(&f).await;
    let pid = backend_pid(&mut writer).await;
    let pending = async {
        lock_channel_memberships_in_transaction(&mut writer, &[second, first])
            .await
            .expect("acquire both channels with reversed input");
        writer.rollback().await.expect("release writer locks");
    };
    tokio::pin!(pending);
    tokio::select! {
        () = &mut pending => panic!("writer completed before the first channel was released"),
        () = wait_for_blocking_pid(&mut holder, pid) => {}
    }

    let mut probe = tx(&f).await;
    sqlx::query("SET LOCAL lock_timeout = '500ms'")
        .execute(probe.conn())
        .await
        .expect("bound the order check");
    acquire_channel_membership_lock(probe.conn(), f.community, second)
        .await
        .expect("the second channel stays available while the writer waits for the first");
    probe.rollback().await.expect("release second channel");
    holder.rollback().await.expect("release first channel");
    tokio::time::timeout(Duration::from_secs(5), pending)
        .await
        .expect("writer completes after the first channel is released");
}
