use super::*;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

struct LegacyInviteCase {
    community: Uuid,
}

struct RelayAdminActionFixture<'a> {
    community: Uuid,
    target_pubkey: &'a [u8; 32],
    report_bound: bool,
    state: &'a str,
    step_marker: Option<&'a str>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

async fn insert_restriction(
    pool: &PgPool,
    community: Uuid,
    pubkey: &[u8; 32],
    banned: bool,
    ban_expires_at: Option<DateTime<Utc>>,
    muted_until: Option<DateTime<Utc>>,
) {
    sqlx::query(
        "INSERT INTO community_bans \
         (community_id, pubkey, banned, ban_expires_at, muted_until, actor_pubkey) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(community)
    .bind(pubkey.as_slice())
    .bind(banned)
    .bind(ban_expires_at)
    .bind(muted_until)
    .bind([3u8; 32].as_slice())
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_legacy_invite(
    pool: &PgPool,
    community: Uuid,
    token_seed: u8,
    created_by: &str,
    created_at: DateTime<Utc>,
) -> [u8; 32] {
    let token_hash = [token_seed; 32];
    sqlx::query(
        "INSERT INTO relay_invites \
         (community_id, token_hash, created_by, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(community)
    .bind(token_hash.as_slice())
    .bind(created_by)
    .bind(created_at + Duration::days(2))
    .bind(created_at)
    .execute(pool)
    .await
    .unwrap();
    token_hash
}

async fn insert_moderation_action(
    pool: &PgPool,
    community: Uuid,
    action: &str,
    target_pubkey: &[u8; 32],
    created_at: DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO moderation_actions \
         (community_id, actor_pubkey, action, target_pubkey, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(community)
    .bind([2u8; 32].as_slice())
    .bind(action)
    .bind(target_pubkey.as_slice())
    .bind(created_at)
    .execute(pool)
    .await
    .unwrap();
}

async fn insert_relay_admin_action(pool: &PgPool, fixture: RelayAdminActionFixture<'_>) -> Uuid {
    let RelayAdminActionFixture {
        community,
        target_pubkey,
        report_bound,
        state,
        step_marker,
        created_at,
        updated_at,
    } = fixture;
    let report_id = report_bound.then(Uuid::new_v4);
    if let Some(report_id) = report_id {
        let mut report_event_id = [0u8; 32];
        report_event_id[..16].copy_from_slice(report_id.as_bytes());
        report_event_id[16..].copy_from_slice(report_id.as_bytes());
        sqlx::query(
            "INSERT INTO moderation_reports \
             (community_id, id, report_event_id, reporter_pubkey, target_kind, \
              target_pubkey, report_type) \
             VALUES ($1, $2, $3, $4, 'pubkey', $5, 'spam')",
        )
        .bind(community)
        .bind(report_id)
        .bind(report_event_id.as_slice())
        .bind([4u8; 32].as_slice())
        .bind(target_pubkey.as_slice())
        .execute(pool)
        .await
        .unwrap();
    }

    let id = Uuid::new_v4();
    let actor_pubkey = [3u8; 32];
    let cancelled_by = (state == "cancelled").then_some(actor_pubkey.as_slice());
    sqlx::query(
        "INSERT INTO relay_admin_actions \
         (id, report_id, report_community_id, request_id, actor_pubkey, actor_role, \
          action, state, step_marker, cancelled_by, enforcement_target_pubkey, \
          created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, 'operator', 'ban', $6, $7, $8, $9, $10, $11)",
    )
    .bind(id)
    .bind(report_id)
    .bind(community)
    .bind(Uuid::new_v4())
    .bind(actor_pubkey.as_slice())
    .bind(state)
    .bind(step_marker)
    .bind(cancelled_by)
    .bind(target_pubkey.as_slice())
    .bind(created_at)
    .bind(updated_at)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn set_deletion_state(pool: &PgPool, community: Uuid, state: &str) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT set_config('buzz.deletion_executor_community', $1, true), \
                set_config('buzz.deletion_fence_generation', '7', true)",
    )
    .bind(community.to_string())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE communities \
         SET deletion_state = $2, deletion_fence_generation = 7 WHERE id = $1",
    )
    .bind(community)
    .bind(state)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

async fn legacy_fixture() -> (PgPool, Vec<LegacyInviteCase>) {
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    // Pinned main includes the user deletion target in 0072-0073 and its fences in
    // 0074; invitations expand in 0075.
    crate::migration::run_migrations_through(&pool, 74)
        .await
        .unwrap();
    let last_migration: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        last_migration, 74,
        "fixtures start at the complete pinned-main schema"
    );

    let issuer = [1u8; 32];
    let mut cases = Vec::new();
    for index in 0u8..12 {
        let community = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("upgrade-{index}-{community}.example"))
            .execute(&pool)
            .await
            .unwrap();

        let created_by = match index {
            5 => {
                let agent = [4u8; 32];
                let owner = [5u8; 32];
                sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2), ($1, $3)")
                    .bind(community)
                    .bind(agent.as_slice())
                    .bind(owner.as_slice())
                    .execute(&pool)
                    .await
                    .unwrap();
                // Seed the owner link before testing the writer-side admission lock.
                sqlx::query(
                    "UPDATE users SET agent_owner_pubkey = $1 \
                     WHERE community_id = $2 AND pubkey = $3",
                )
                .bind(owner.as_slice())
                .bind(community)
                .bind(agent.as_slice())
                .execute(&pool)
                .await
                .unwrap();
                insert_restriction(&pool, community, &owner, false, None, None).await;
                hex::encode(agent)
            }
            6 => "not-a-pubkey".to_owned(),
            7 => hex::encode([0xabu8; 32]).to_ascii_uppercase(),
            _ => hex::encode(issuer),
        };

        match index {
            0 => insert_restriction(&pool, community, &issuer, true, None, None).await,
            1 => {
                insert_restriction(
                    &pool,
                    community,
                    &issuer,
                    true,
                    Some(Utc::now() - Duration::hours(1)),
                    None,
                )
                .await;
            }
            2 | 4 => insert_restriction(&pool, community, &issuer, false, None, None).await,
            3 => {
                insert_restriction(
                    &pool,
                    community,
                    &issuer,
                    false,
                    None,
                    Some(Utc::now() - Duration::hours(1)),
                )
                .await;
            }
            _ => {}
        }

        let old_action = if index == 10 {
            Some(
                insert_relay_admin_action(
                    &pool,
                    RelayAdminActionFixture {
                        community,
                        target_pubkey: &issuer,
                        report_bound: false,
                        state: "succeeded",
                        step_marker: Some("mutation_committed"),
                        created_at: Utc::now() - Duration::days(1),
                        updated_at: Utc::now() - Duration::days(1),
                    },
                )
                .await,
            )
        } else {
            None
        };
        let created_at = if index == 4 {
            Utc::now() + Duration::hours(1)
        } else {
            Utc::now() - Duration::hours(1)
        };
        insert_legacy_invite(&pool, community, index + 1, &created_by, created_at).await;

        if index == 9 {
            insert_moderation_action(
                &pool,
                community,
                "resolve:ban",
                &issuer,
                Utc::now() + Duration::minutes(1),
            )
            .await;
        }
        if index == 10 {
            let after = Utc::now() + Duration::minutes(1);
            insert_moderation_action(&pool, community, "ban", &issuer, after).await;
            for state in ["pending", "enforcing", "failed", "cancelled"] {
                let marker =
                    matches!(state, "enforcing" | "failed").then_some("mutation_committed");
                for report_bound in [true, false] {
                    insert_relay_admin_action(
                        &pool,
                        RelayAdminActionFixture {
                            community,
                            target_pubkey: &issuer,
                            report_bound,
                            state,
                            step_marker: marker,
                            created_at: after,
                            updated_at: after,
                        },
                    )
                    .await;
                }
            }
            sqlx::query("UPDATE relay_admin_actions SET updated_at = $2 WHERE id = $1")
                .bind(old_action.unwrap())
                .bind(Utc::now() + Duration::minutes(2))
                .execute(&pool)
                .await
                .unwrap();
        }
        if index == 11 {
            let bystander = Uuid::new_v4();
            sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
                .bind(bystander)
                .bind(format!("upgrade-bystander-{bystander}.example"))
                .execute(&pool)
                .await
                .unwrap();
            insert_restriction(&pool, bystander, &issuer, true, None, None).await;
        }
        if index == 1 {
            set_deletion_state(&pool, community, "quiescing").await;
        } else if index == 2 {
            set_deletion_state(&pool, community, "tombstone").await;
        }

        cases.push(LegacyInviteCase { community });
    }
    // Preserve a populated, non-default join role across both upgrade paths.
    sqlx::query(
        "INSERT INTO channels (community_id, name, created_by, join_role) \
         VALUES ($1, 'upgrade-guest', $2, 'guest')",
    )
    .bind(cases[0].community)
    .bind(issuer.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    assert_join_role_preserved(&pool, cases[0].community).await;
    (pool, cases)
}

async fn assert_join_role_preserved(pool: &PgPool, community: Uuid) {
    let column: (String, String, Option<String>) = sqlx::query_as(
        "SELECT data_type, is_nullable, column_default \
         FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'channels' \
           AND column_name = 'join_role'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        column,
        ("text".into(), "NO".into(), Some("'member'::text".into()))
    );
    let join_role: String = sqlx::query_scalar(
        "SELECT join_role FROM channels WHERE community_id = $1 AND name = 'upgrade-guest'",
    )
    .bind(community)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        join_role, "guest",
        "upgrading must preserve channel admission policy"
    );
}

async fn invite_revocation_state(pool: &PgPool) -> Vec<(Uuid, Option<DateTime<Utc>>, bool)> {
    sqlx::query_as(
        "SELECT community_id, revoked_at, legacy_admission \
         FROM relay_invites ORDER BY community_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn assert_admission_column(pool: &PgPool) {
    let column: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT data_type, is_nullable, column_default \
         FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'relay_invites' \
           AND column_name = 'legacy_admission'",
    )
    .fetch_optional(pool)
    .await
    .unwrap();
    assert_eq!(
        column,
        Some(("boolean".into(), "NO".into(), Some("true".into())))
    );
    let revoked: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT data_type, is_nullable, column_default \
         FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'relay_invites' \
           AND column_name = 'revoked_at'",
    )
    .fetch_optional(pool)
    .await
    .unwrap();
    assert_eq!(
        revoked,
        Some(("timestamp with time zone".into(), "YES".into(), None))
    );
}

async fn apply_pgschema_to_existing_database(pool: &PgPool) {
    // The pinned migration history has retired triggers on the partitioned
    // events parent and auto-named deletion-request checks that the desired
    // schema replaces. pgschema orders child-trigger drops before their parent
    // and schedules a replacement check before dropping its same-named copy.
    // Remove only those stale objects in this disposable fixture; pgschema
    // still applies desired state; historical repair is a separate reviewed
    // operation after deployment.
    sqlx::raw_sql(
        "DROP TRIGGER IF EXISTS trg_events_guard_nip_rs_hard_delete ON events; \
         DROP TRIGGER IF EXISTS trg_events_nip_rs_watermark ON events; \
         DROP TRIGGER IF EXISTS trg_events_purge_soft_deleted_nip_rs ON events; \
         DROP TRIGGER IF EXISTS trg_events_purge_soft_deleted_buzz_mesh_status ON events; \
         ALTER TABLE community_deletion_requests \
             DROP CONSTRAINT IF EXISTS community_deletion_requests_check1; \
         ALTER TABLE community_deletion_requests \
             DROP CONSTRAINT IF EXISTS community_deletion_requests_check2; \
         ALTER TABLE community_deletion_requests \
             DROP CONSTRAINT IF EXISTS community_deletion_requests_check3; \
         ALTER TABLE community_deletion_requests \
             DROP CONSTRAINT IF EXISTS community_deletion_requests_check4;",
    )
    .execute(pool)
    .await
    .unwrap();

    let database_url = crate::test_support::database_url();
    let options: sqlx::postgres::PgConnectOptions = database_url.parse().unwrap();
    let host = options.get_host().to_owned();
    let port = options.get_port().to_string();
    let user = options.get_username().to_owned();
    let password = database_url
        .split_once("://")
        .and_then(|(_, rest)| rest.split_once('@'))
        .and_then(|(authority, _)| authority.split_once(':'))
        .map(|(_, password)| password.to_owned())
        .or_else(|| std::env::var("PGPASSWORD").ok())
        .unwrap_or_default();
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await
        .unwrap();
    // Run from the repository root, like every pgschema caller, so
    // `.pgschemaignore` applies.
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema_file = repo_root.join("schema/schema.sql");
    let output = std::process::Command::new(repo_root.join("bin/pgschema"))
        .current_dir(&repo_root)
        .args([
            "apply",
            "--auto-approve",
            "--file",
            schema_file.to_str().unwrap(),
            "--host",
            &host,
            "--port",
            &port,
            "--user",
            &user,
            "--password",
            &password,
            "--db",
            &database,
            "--plan-host",
            &host,
            "--plan-port",
            &port,
            "--plan-user",
            &user,
            "--plan-password",
            &password,
            "--plan-db",
            &database,
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "pgschema apply failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_0075_adds_invite_columns_without_rewriting_history() {
    let (pool, cases) = legacy_fixture().await;
    crate::migration::run_migrations(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await
            .unwrap(),
        75
    );
    assert_admission_column(&pool).await;
    assert_join_role_preserved(&pool, cases[0].community).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM relay_invites WHERE revoked_at IS NOT NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0,
        "schema migration must not rewrite historical rows"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM relay_invites WHERE legacy_admission IS DISTINCT FROM true"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0,
        "pre-migration rows remain legacy-admitted"
    );

    insert_legacy_invite(
        &pool,
        cases[8].community,
        100,
        &hex::encode([1u8; 32]),
        Utc::now(),
    )
    .await;
    let old_code_insert_default: bool = sqlx::query_scalar(
        "SELECT legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(cases[8].community)
    .bind([100u8; 32].as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(old_code_insert_default);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_legacy_claim_lazily_revokes_history_after_a_fenced_community_reactivates()
{
    let (pool, cases) = legacy_fixture().await;
    crate::migration::run_migrations(&pool).await.unwrap();

    let community = cases[1].community;
    let token_hash = [2u8; 32];
    let fenced = claim_relay_invite(
        &pool,
        CommunityId::from_uuid(community),
        &token_hash,
        &format!("{:064x}", 9),
        None,
        &nostr::Keys::generate(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(fenced, crate::DbError::AccessDenied(_)),
        "{fenced:?}"
    );
    let revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM relay_invites WHERE community_id = $1",
    )
    .bind(community)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!revoked, "quiescing tenants stay fenced and untouched");

    set_deletion_state(&pool, community, "active").await;
    assert_eq!(
        claim_relay_invite(
            &pool,
            CommunityId::from_uuid(community),
            &token_hash,
            &format!("{:064x}", 9),
            None,
            &nostr::Keys::generate(),
        )
        .await
        .map(|write| write.outcome)
        .unwrap(),
        ClaimOutcome::Revoked
    );
    let revoked: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM relay_invites WHERE community_id = $1",
    )
    .bind(community)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        revoked,
        "reactivated legacy history is durably revoked on claim"
    );
    let legacy_admission: bool =
        sqlx::query_scalar("SELECT legacy_admission FROM relay_invites WHERE community_id = $1")
            .bind(community)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        legacy_admission,
        "claim does not stand in for maintenance repair"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_desired_pgschema_reconcile_matches_populated_upgrade() {
    let (pool, cases) = legacy_fixture().await;
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM relay_invites")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, cases.len() as i64);

    // Exercise the deployment path against populated data at the complete
    // pinned-main schema through 0074, rather than reproducing its DDL by hand.
    apply_pgschema_to_existing_database(&pool).await;
    assert_admission_column(&pool).await;
    assert_join_role_preserved(&pool, cases[0].community).await;
    let before_reconcile: i64 =
        sqlx::query_scalar("SELECT count(*) FROM relay_invites WHERE revoked_at IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        before_reconcile, 0,
        "pgschema does not execute historical DML"
    );

    let before_reconcile = invite_revocation_state(&pool).await;
    let reconcile = include_str!("../../../../../scripts/reconcile-schema-after-pgschema.sql");
    sqlx::raw_sql(reconcile).execute(&pool).await.unwrap();
    assert_eq!(invite_revocation_state(&pool).await, before_reconcile);
    // Reconciliation is schema-only and idempotent; no historical data is
    // rewritten by the desired-state path.
    sqlx::raw_sql(reconcile).execute(&pool).await.unwrap();
    assert_eq!(invite_revocation_state(&pool).await, before_reconcile);
    assert_join_role_preserved(&pool, cases[0].community).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM relay_invites WHERE legacy_admission IS DISTINCT FROM true"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0,
        "pgschema preserves the legacy default for populated rows"
    );
}
