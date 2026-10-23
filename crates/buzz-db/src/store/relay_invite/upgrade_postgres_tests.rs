use super::*;
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

struct LegacyInviteCase {
    community: Uuid,
    revoke: bool,
    active: bool,
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
    // Pinned main includes entity deletion intake in 0068; invitations expand in 0069.
    crate::migration::run_migrations_through(&pool, 68)
        .await
        .unwrap();
    let last_migration: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        last_migration, 68,
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

        let active = !matches!(index, 1 | 2);
        cases.push(LegacyInviteCase {
            community,
            revoke: index < 8 && active,
            active,
        });
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

async fn start_invite_repair_script(
    pool: &PgPool,
    application_name: Option<&str>,
) -> std::process::Child {
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
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/maintenance/backfill_invite_revocations.sql");
    let mut command = std::process::Command::new("psql");
    command
        .args(["-X", "-v", "ON_ERROR_STOP=1"])
        .arg("--host")
        .arg(host)
        .arg("--port")
        .arg(port)
        .arg("--username")
        .arg(user)
        .arg("--dbname")
        .arg(database)
        .arg("--file")
        .arg(script)
        .env("PGPASSWORD", password)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(application_name) = application_name {
        command.env("PGAPPNAME", application_name);
    }
    command.spawn().expect("start invite repair script")
}

pub(super) async fn run_invite_repair_script(pool: &PgPool) -> String {
    let child = start_invite_repair_script(pool, None).await;
    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "invite repair script failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        stdout.lines().any(|line| line.trim() == "0"),
        "repair validation did not return zero unrepaired invites:\n{stdout}"
    );
    stdout
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

async fn assert_repaired_and_fenced(pool: &PgPool, cases: &[LegacyInviteCase]) {
    assert_admission_column(pool).await;
    for case in cases {
        let (revoked, legacy): (bool, bool) = sqlx::query_as(
            "SELECT revoked_at IS NOT NULL, legacy_admission \
             FROM relay_invites WHERE community_id = $1",
        )
        .bind(case.community)
        .fetch_one(pool)
        .await
        .unwrap();
        assert_eq!(revoked, case.revoke, "tenant {}", case.community);
        assert_eq!(legacy, !case.active, "legacy marker for {}", case.community);
    }
    let unfenced_error =
        sqlx::query("UPDATE relay_invites SET use_count = use_count + 1 WHERE community_id = $1")
            .bind(cases[1].community)
            .execute(pool)
            .await
            .unwrap_err();
    assert!(
        unfenced_error
            .to_string()
            .contains("community write fenced"),
        "{unfenced_error}"
    );
    let invitation_triggers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger WHERE NOT tgisinternal \
         AND tgname IN ('invite_restriction_lock', 'invite_issuer_revocation', \
                        'invite_owner_lock', 'invite_owner_revocation', \
                        'community_write_fence_relay_invites')",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        invitation_triggers, 1,
        "only the community deletion fence remains"
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
    let pgschema = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bin/pgschema");
    let schema_file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/schema.sql");
    let output = std::process::Command::new(pgschema)
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
async fn migration_schema_backfill_repairs_retained_restrictions_and_ignores_audit_only_history() {
    let (pool, cases) = legacy_fixture().await;
    crate::migration::run_migrations(&pool).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&pool)
            .await
            .unwrap(),
        69
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

    run_invite_repair_script(&pool).await;
    let repaired = invite_revocation_state(&pool).await;
    assert_repaired_and_fenced(&pool, &cases).await;
    run_invite_repair_script(&pool).await;
    assert_eq!(invite_revocation_state(&pool).await, repaired);
    assert_repaired_and_fenced(&pool, &cases).await;
    assert_join_role_preserved(&pool, cases[0].community).await;

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
    run_invite_repair_script(&pool).await;
    let old_code_insert_repaired: bool = sqlx::query_scalar(
        "SELECT legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(cases[8].community)
    .bind([100u8; 32].as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!old_code_insert_repaired);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_legacy_claim_lazily_revokes_history_after_a_fenced_community_reactivates()
{
    let (pool, cases) = legacy_fixture().await;
    crate::migration::run_migrations(&pool).await.unwrap();
    run_invite_repair_script(&pool).await;

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
        "fenced communities retain the legacy marker until repaired"
    );
    run_invite_repair_script(&pool).await;
    let legacy_admission: bool =
        sqlx::query_scalar("SELECT legacy_admission FROM relay_invites WHERE community_id = $1")
            .bind(community)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !legacy_admission,
        "active community is repaired on the next run"
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
    // pinned-main schema through 0068, rather than reproducing its DDL by hand.
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

    run_invite_repair_script(&pool).await;
    let repaired = invite_revocation_state(&pool).await;
    assert_repaired_and_fenced(&pool, &cases).await;
    run_invite_repair_script(&pool).await;
    assert_eq!(invite_revocation_state(&pool).await, repaired);
    assert_repaired_and_fenced(&pool, &cases).await;
    assert_join_role_preserved(&pool, cases[0].community).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_repaired_marker_survives_reconnect_and_timeout_expires_without_revocation(
) {
    let (pool, cases) = legacy_fixture().await;
    crate::migration::run_migrations(&pool).await.unwrap();
    run_invite_repair_script(&pool).await;

    let community = CommunityId::from_uuid(cases[8].community);
    let token_hash = [9u8; 32];
    let legacy_admission: bool = sqlx::query_scalar(
        "SELECT legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(cases[8].community)
    .bind(token_hash.as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !legacy_admission,
        "repair clears a spared active legacy row"
    );

    pool.close().await;
    let pool = PgPool::connect(&crate::test_support::database_url())
        .await
        .unwrap();
    let legacy_admission: bool = sqlx::query_scalar(
        "SELECT legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(cases[8].community)
    .bind(token_hash.as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !legacy_admission,
        "repaired marker is persisted in the database"
    );

    let issuer = [1u8; 32];
    crate::moderation::timeout_member(
        &pool,
        community,
        &issuer,
        &[3; 32],
        Utc::now() + Duration::hours(1),
        None,
    )
    .await
    .unwrap();
    let claimant = hex::encode([9u8; 32]);
    assert_eq!(
        super::revocation_postgres_tests::claim_relay_invite_outcome(
            &pool,
            community,
            &token_hash,
            &claimant,
            None,
        )
        .await
        .unwrap(),
        ClaimOutcome::IssuerRestricted
    );
    let revoked_while_timed_out: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(cases[8].community)
    .bind(token_hash.as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !revoked_while_timed_out,
        "a post-repair timeout is temporary"
    );

    sqlx::query(
        "UPDATE community_bans SET muted_until=clock_timestamp()-interval '1 second' \
         WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(community.as_uuid())
    .bind(issuer.as_slice())
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        super::revocation_postgres_tests::claim_relay_invite_outcome(
            &pool,
            community,
            &token_hash,
            &claimant,
            None,
        )
        .await
        .unwrap(),
        ClaimOutcome::Joined { .. }
    ));
    let revoked_after_timeout: bool = sqlx::query_scalar(
        "SELECT revoked_at IS NOT NULL FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(cases[8].community)
    .bind(token_hash.as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!revoked_after_timeout);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invite_repair_revokes_new_protocol_invites_for_active_bans() {
    let (pool, community, issuer, claimant) = super::revocation_postgres_tests::fixture().await;
    let invite = mint_relay_invite(&pool, community, &issuer, 3600, None)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) \
         VALUES ($1, $2, true, $3)",
    )
    .bind(community.as_uuid())
    .bind([1u8; 32].as_slice())
    .bind([3u8; 32].as_slice())
    .execute(&pool)
    .await
    .unwrap();

    run_invite_repair_script(&pool).await;
    let (revoked, legacy): (bool, bool) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL, legacy_admission \
         FROM relay_invites WHERE community_id=$1 AND id=$2",
    )
    .bind(community.as_uuid())
    .bind(invite.invite_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        revoked,
        "repair preserves active-ban revocation for new rows"
    );
    assert!(!legacy, "new-protocol rows remain outside legacy admission");
    assert_eq!(
        super::revocation_postgres_tests::claim_relay_invite_outcome(
            &pool,
            community,
            &hash_v2_code(&invite.code),
            &claimant,
            None,
        )
        .await
        .unwrap(),
        ClaimOutcome::Revoked
    );
}

async fn assert_repair_waits_for_admission(pool: &PgPool, application_name: &str) {
    let has_shared_deletion_lock: bool = sqlx::query_scalar(
        "SELECT EXISTS (\
             SELECT 1 FROM pg_locks AS lock \
             JOIN pg_stat_activity AS activity ON activity.pid=lock.pid \
             WHERE activity.application_name=$1 \
               AND lock.locktype='advisory' AND lock.granted \
               AND lock.mode='ShareLock'\
         )",
    )
    .bind(application_name)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        has_shared_deletion_lock,
        "repair takes the shared deletion lock before waiting for admission"
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invite_repair_serializes_with_ban_before_clearing_legacy_marker() {
    let (pool, community, issuer, _) = super::revocation_postgres_tests::fixture().await;
    insert_legacy_invite(&pool, *community.as_uuid(), 81, &issuer, Utc::now()).await;
    insert_restriction(&pool, *community.as_uuid(), &[1; 32], false, None, None).await;

    let mut restriction_row = pool.begin().await.unwrap();
    sqlx::query("SELECT pubkey FROM community_bans WHERE community_id=$1 FOR UPDATE")
        .bind(community.as_uuid())
        .fetch_one(&mut *restriction_row)
        .await
        .unwrap();
    let (ban_pool, ban_name) = super::revocation_postgres_tests::named_pool().await;
    let ban = tokio::spawn(async move {
        crate::moderation::ban_member(&ban_pool, community, &[1; 32], &[3; 32], None, None).await
    });
    super::revocation_postgres_tests::wait_for_lock(&pool, &ban_name, "transactionid", &ban).await;

    let repair_name = format!("invite-repair-{}", Uuid::new_v4());
    let child = start_invite_repair_script(&pool, Some(&repair_name)).await;
    let repair = tokio::task::spawn_blocking(move || child.wait_with_output());
    super::revocation_postgres_tests::wait_for_lock(&pool, &repair_name, "advisory", &repair).await;
    assert_repair_waits_for_admission(&pool, &repair_name).await;

    restriction_row.commit().await.unwrap();
    ban.await.unwrap().unwrap();
    let output = repair.await.unwrap().unwrap();
    assert!(
        output.status.success(),
        "repair failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let (revoked, legacy): (bool, bool) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL, legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(community.as_uuid())
    .bind([81u8; 32].as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(revoked);
    assert!(!legacy);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invite_repair_serializes_with_owner_change_before_clearing_legacy_marker() {
    let (pool, community, _, _) = super::revocation_postgres_tests::fixture().await;
    let agent = [4u8; 32];
    let owner = [5u8; 32];
    sqlx::query("INSERT INTO users (community_id,pubkey) VALUES ($1,$2),($1,$3)")
        .bind(community.as_uuid())
        .bind(agent.as_slice())
        .bind(owner.as_slice())
        .execute(&pool)
        .await
        .unwrap();
    insert_restriction(&pool, *community.as_uuid(), &owner, false, None, None).await;
    let issuer = hex::encode(agent);
    insert_legacy_invite(&pool, *community.as_uuid(), 82, &issuer, Utc::now()).await;

    let mut user_row = pool.begin().await.unwrap();
    sqlx::query("SELECT pubkey FROM users WHERE community_id=$1 AND pubkey=$2 FOR UPDATE")
        .bind(community.as_uuid())
        .bind(agent.as_slice())
        .fetch_one(&mut *user_row)
        .await
        .unwrap();
    let (owner_pool, owner_name) = super::revocation_postgres_tests::named_pool().await;
    let owner_write = tokio::spawn(async move {
        crate::user::set_agent_owner(&owner_pool, community, &agent, &owner).await
    });
    super::revocation_postgres_tests::wait_for_lock(
        &pool,
        &owner_name,
        "transactionid",
        &owner_write,
    )
    .await;

    let repair_name = format!("invite-repair-{}", Uuid::new_v4());
    let child = start_invite_repair_script(&pool, Some(&repair_name)).await;
    let repair = tokio::task::spawn_blocking(move || child.wait_with_output());
    super::revocation_postgres_tests::wait_for_lock(&pool, &repair_name, "advisory", &repair).await;
    assert_repair_waits_for_admission(&pool, &repair_name).await;

    user_row.commit().await.unwrap();
    assert!(owner_write.await.unwrap().unwrap());
    let output = repair.await.unwrap().unwrap();
    assert!(
        output.status.success(),
        "repair failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let (revoked, legacy): (bool, bool) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL, legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(community.as_uuid())
    .bind([82u8; 32].as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        revoked,
        "repair sees the committed current owner restriction"
    );
    assert!(!legacy);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn invite_repair_waits_for_deletion_transition_before_clearing_legacy_marker() {
    let (pool, community, issuer, _) = super::revocation_postgres_tests::fixture().await;
    insert_legacy_invite(&pool, *community.as_uuid(), 83, &issuer, Utc::now()).await;
    insert_restriction(&pool, *community.as_uuid(), &[1; 32], false, None, None).await;

    let mut deletion = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(community_deletion_lock_key($1))")
        .bind(community.as_uuid())
        .execute(&mut *deletion)
        .await
        .unwrap();
    sqlx::query(
        "SELECT set_config('buzz.deletion_executor_community',$1,true), \
                set_config('buzz.deletion_fence_generation','7',true)",
    )
    .bind(community.to_string())
    .execute(&mut *deletion)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE communities SET deletion_state='quiescing', \
                deletion_fence_generation=7 WHERE id=$1",
    )
    .bind(community.as_uuid())
    .execute(&mut *deletion)
    .await
    .unwrap();

    let repair_name = format!("invite-repair-{}", Uuid::new_v4());
    let child = start_invite_repair_script(&pool, Some(&repair_name)).await;
    let repair = tokio::task::spawn_blocking(move || child.wait_with_output());
    super::revocation_postgres_tests::wait_for_lock(&pool, &repair_name, "advisory", &repair).await;
    let waiting_for_deletion_before_admission: bool = sqlx::query_scalar(
        "SELECT EXISTS (\
             SELECT 1 FROM pg_locks AS lock \
             JOIN pg_stat_activity AS activity ON activity.pid=lock.pid \
             WHERE activity.application_name=$1 AND lock.locktype='advisory' \
               AND NOT lock.granted AND lock.mode='ShareLock'\
         ) AND NOT EXISTS (\
             SELECT 1 FROM pg_locks AS lock \
             JOIN pg_stat_activity AS activity ON activity.pid=lock.pid \
             WHERE activity.application_name=$1 AND lock.locktype='advisory' \
               AND lock.granted AND lock.mode='ExclusiveLock'\
         )",
    )
    .bind(&repair_name)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        waiting_for_deletion_before_admission,
        "repair waits for the deletion lock before taking admission"
    );
    let (revoked, legacy): (bool, bool) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL, legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(community.as_uuid())
    .bind([83u8; 32].as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !revoked && legacy,
        "repair must not write through deletion lock"
    );

    deletion.commit().await.unwrap();
    let output = repair.await.unwrap().unwrap();
    assert!(
        output.status.success(),
        "repair failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let (revoked, legacy): (bool, bool) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL, legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(community.as_uuid())
    .bind([83u8; 32].as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!revoked && legacy, "quiescing communities remain fenced");

    set_deletion_state(&pool, *community.as_uuid(), "active").await;
    run_invite_repair_script(&pool).await;
    let (revoked, legacy): (bool, bool) = sqlx::query_as(
        "SELECT revoked_at IS NOT NULL, legacy_admission FROM relay_invites \
         WHERE community_id=$1 AND token_hash=$2",
    )
    .bind(community.as_uuid())
    .bind([83u8; 32].as_slice())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        revoked && !legacy,
        "active community is repaired on the next run"
    );
}
