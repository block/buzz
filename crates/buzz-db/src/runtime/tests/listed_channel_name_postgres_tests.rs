//! scripts/maintenance/plan_duplicate_listed_channel_renames.sql lists the
//! renames that leave no two listed channels in a community with one name,
//! ignoring case. It keeps the name on the oldest unarchived channel and picks
//! a new name that no other listed channel has.
//!
//! Migration 0079 then adds the unique index. It refuses to run over
//! duplicates that were not renamed or, on a populated table, without a
//! concurrent prebuild; it accepts a valid prebuild without replacing it and
//! rejects an invalid one.

use super::*;

/// The last migration before 0079. Duplicates can exist, and the plan has
/// work to do, only before the index does.
const BEFORE_INDEX: Option<i64> = Some(78);

const PLAN_SQL: &str =
    include_str!("../../../../../scripts/maintenance/plan_duplicate_listed_channel_renames.sql");

struct Seed {
    community: Uuid,
    host: String,
    created_by: Vec<u8>,
}

impl Seed {
    async fn new(pool: &PgPool) -> Self {
        let community = Uuid::new_v4();
        let host = format!("listed-name-{}.example", community.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(&host)
            .execute(pool)
            .await
            .expect("insert community");
        Self {
            community,
            host,
            created_by: nostr::Keys::generate().public_key().to_bytes().to_vec(),
        }
    }

    /// Inserts an open stream `age` seconds old with plain SQL, as a relay
    /// without the name rule did.
    async fn open(&self, pool: &PgPool, id: Uuid, name: &str, age: i64) {
        sqlx::query(
            "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by, \
                                   created_at) \
             VALUES ($1, $2, $3, 'stream', 'open', $4, now() - make_interval(secs => $5))",
        )
        .bind(id)
        .bind(self.community)
        .bind(name)
        .bind(&self.created_by)
        .bind(age as f64)
        .execute(pool)
        .await
        .expect("insert channel");
    }

    async fn names(&self, pool: &PgPool) -> Vec<(Uuid, String)> {
        sqlx::query_as("SELECT id, name FROM channels WHERE community_id = $1 ORDER BY id")
            .bind(self.community)
            .fetch_all(pool)
            .await
            .expect("read channel names")
    }
}

fn renamed(id: Uuid, name: &str) -> (Uuid, String) {
    (id, format!("{name}-{}", &id.to_string()[..8]))
}

/// The plan for `host`: (channel, current name, new name), as the rename
/// script reads it.
async fn rename_plan(pool: &PgPool, host: &str) -> Vec<(Uuid, String, String)> {
    let (_, query) = PLAN_SQL.split_once("\nWITH").expect("plan query");
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "WITH{}",
        query
            .trim_end()
            .trim_end_matches(';')
            .replace(":'host'", "$1")
    )))
    .bind(host)
    .fetch_all(pool)
    .await
    .expect("run the rename plan")
}

/// Applies the plan the way the relay applies each rename the script sends,
/// and returns how many channels it renamed.
async fn rename_duplicates(pool: &PgPool, host: &str) -> usize {
    let plan = rename_plan(pool, host).await;
    for (channel, _, new_name) in &plan {
        sqlx::query("UPDATE channels SET name = $2 WHERE id = $1")
            .bind(channel)
            .bind(new_name)
            .execute(pool)
            .await
            .expect("rename channel");
    }
    plan.len()
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn plan_keeps_each_listed_name_on_its_oldest_unarchived_channel() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "listed_name_plan", BEFORE_INDEX).await;
    let seed = Seed::new(&pool).await;
    let [kept, archived, newer, private, deleted] = [(); 5].map(|_| Uuid::new_v4());
    seed.open(&pool, kept, "square-ios", 30).await;
    seed.open(&pool, archived, "Square-iOS", 60).await;
    seed.open(&pool, newer, "SQUARE-IOS", 10).await;
    seed.open(&pool, private, "square-ios", 5).await;
    seed.open(&pool, deleted, "square-ios", 90).await;
    sqlx::query(
        "UPDATE channels SET \
             archived_at = CASE WHEN id = $2 THEN now() END, \
             visibility = CASE WHEN id = $3 THEN 'private'::channel_visibility ELSE visibility END, \
             deleted_at = CASE WHEN id = $4 THEN now() END \
         WHERE community_id = $1",
    )
    .bind(seed.community)
    .bind(archived)
    .bind(private)
    .bind(deleted)
    .execute(&pool)
    .await
    .unwrap();
    // Another community's channel with the same name is not a duplicate.
    let other = Seed::new(&pool).await;
    other.open(&pool, Uuid::new_v4(), "square-ios", 0).await;
    // A community that is not active is left alone.
    let inactive = Seed::new(&pool).await;
    inactive.open(&pool, Uuid::new_v4(), "payments", 20).await;
    inactive.open(&pool, Uuid::new_v4(), "payments", 10).await;
    crate::test_support::quiesce_community_for_tests(
        &pool,
        buzz_core::CommunityId::from_uuid(inactive.community),
    )
    .await;

    let mut expected = vec![
        (
            archived,
            "Square-iOS".to_owned(),
            renamed(archived, "Square-iOS").1,
        ),
        (
            newer,
            "SQUARE-IOS".to_owned(),
            renamed(newer, "SQUARE-IOS").1,
        ),
    ];
    expected.sort();
    assert_eq!(rename_plan(&pool, &seed.host).await, expected);
    assert!(rename_plan(&pool, &other.host).await.is_empty());
    assert!(rename_plan(&pool, &inactive.host).await.is_empty());

    assert_eq!(rename_duplicates(&pool, &seed.host).await, 2);
    assert!(
        rename_plan(&pool, &seed.host).await.is_empty(),
        "nothing is left to rename"
    );
    let mut names = vec![
        (kept, "square-ios".to_owned()),
        renamed(archived, "Square-iOS"),
        renamed(newer, "SQUARE-IOS"),
        (private, "square-ios".to_owned()),
        (deleted, "square-ios".to_owned()),
    ];
    names.sort();
    assert_eq!(seed.names(&pool).await, names);
    drop_scratch_db(&admin, pool, &name).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn plan_picks_a_free_name_when_the_short_one_is_taken() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) =
        create_scratch_db_through(&admin, "listed_name_plan_taken", BEFORE_INDEX).await;
    let seed = Seed::new(&pool).await;
    // The longest name the column holds, so the new name must truncate it.
    let long = "a".repeat(255);
    let duplicate = Uuid::parse_str("20000000-0000-4000-8000-000000000000").unwrap();
    let (kept, squatter) = (Uuid::new_v4(), Uuid::new_v4());
    seed.open(&pool, kept, &long, 30).await;
    seed.open(&pool, duplicate, &long, 10).await;
    // Another listed channel already has the short new name, in another case.
    let short = format!("{}-20000000", "A".repeat(246));
    seed.open(&pool, squatter, &short, 60).await;

    let new_name = format!("{}-{duplicate}", "a".repeat(218));
    assert_eq!(
        rename_plan(&pool, &seed.host).await,
        vec![(duplicate, long.clone(), new_name.clone())]
    );
    assert!(new_name.chars().count() <= 255);
    drop_scratch_db(&admin, pool, &name).await;
}

/// Runs the real script for `host` against database `db`, with `cli_body` as
/// the stand-in CLI. Returns the script's output and each CLI call's
/// arguments.
#[cfg(unix)]
async fn run_rename_script(
    host: &str,
    db: &str,
    cli_body: &str,
) -> (std::process::Output, Vec<Vec<Vec<u8>>>) {
    let dir = std::env::temp_dir().join(format!("listed-name-script-{}", Uuid::new_v4().simple()));
    std::fs::create_dir(&dir).unwrap();
    let (cli, log) = (dir.join("buzz"), dir.join("calls"));
    std::fs::write(
        &cli,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\0' \"$@\" '' >>\"$STUB_LOG\"\n{cli_body}"
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

    let base = admin_url().await;
    let database_url = format!("{}/{}", &base[..base.rfind('/').unwrap()], db);
    let mut path = std::env::var("PATH").unwrap_or_default();
    if let Ok(bin) = std::env::var("PG_BIN_DIR") {
        path = format!("{bin}:{path}");
    }
    let output = std::process::Command::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/maintenance/rename_duplicate_listed_channel_names.sh"
    ))
    .arg(host)
    .env("PATH", path)
    .env("DATABASE_URL", &database_url)
    .env("BUZZ_PRIVATE_KEY", "unused")
    .env("BUZZ_CLI", &cli)
    .env("STUB_LOG", &log)
    .output()
    .expect("run the rename script");

    // Each call ends with an empty field, so "\0\0" splits the calls.
    let calls = std::fs::read(&log).unwrap_or_default();
    let calls = calls
        .split(|b| *b == 0)
        .collect::<Vec<_>>()
        .split(|field| field.is_empty())
        .filter(|call| !call.is_empty())
        .map(|call| call.iter().map(|field| field.to_vec()).collect())
        .collect();
    std::fs::remove_dir_all(&dir).unwrap();
    (output, calls)
}

/// The real script, with a stand-in CLI that logs each call and applies the
/// rename. A name may hold any character but NUL, so a name holding control
/// characters must still yield exactly one rename, of its own channel. Like
/// clap, the stand-in takes a value that starts with "-" only as `--name=`.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires Postgres"]
async fn script_renames_each_planned_channel_once_whatever_its_name() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "listed_name_script", BEFORE_INDEX).await;
    let seed = Seed::new(&pool).await;
    let [kept, duplicate, other] = [(); 3].map(|_| Uuid::new_v4());
    let tricky = format!("-ops\x1e{other}\x1fowned\x1f\n\t\"$(x)");
    seed.open(&pool, kept, &tricky, 30).await;
    seed.open(&pool, duplicate, &tricky, 10).await;
    seed.open(&pool, other, "other", 20).await;

    let (output, calls) = run_rename_script(
        &seed.host,
        &name,
        "[[ $3 == --channel=* && $4 == --name=* ]] || exit 2\n\
         psql \"$DATABASE_URL\" -X -q -v ON_ERROR_STOP=1 -v id=\"${3#*=}\" -v name=\"${4#*=}\" <<'SQL'\n\
         UPDATE channels SET name = :'name' WHERE id = :'id';\nSQL\n",
    )
    .await;
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let new_name = renamed(duplicate, &tricky).1;
    let expected: Vec<Vec<Vec<u8>>> = vec![vec![
        b"channels".to_vec(),
        b"update".to_vec(),
        format!("--channel={duplicate}").into_bytes(),
        format!("--name={new_name}").into_bytes(),
    ]];
    assert_eq!(calls, expected, "one call, for the duplicate only");
    let mut names = vec![
        (kept, tricky.clone()),
        (duplicate, new_name.clone()),
        (other, "other".to_owned()),
    ];
    names.sort();
    assert_eq!(seed.names(&pool).await, names);
    drop_scratch_db(&admin, pool, &name).await;
}

/// An auth refusal (CLI exit code 3, for example 403 relay_membership_required)
/// stops the script at the first rename, with a message that names the
/// membership precondition.
/// Mutation: drop the exit-code-3 branch in the script → RED.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires Postgres"]
async fn script_stops_at_the_first_auth_refusal() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db(&admin, "listed_name_script_auth").await;
    let seed = Seed::new(&pool).await;
    for (group, age) in [("first", 30), ("first", 10), ("second", 30), ("second", 10)] {
        seed.open(&pool, Uuid::new_v4(), group, age).await;
    }

    let (output, calls) = run_rename_script(&seed.host, &name, "exit 3\n").await;
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(calls.len(), 1, "it stops after the first refusal");
    assert!(stderr.contains("a member of"), "{stderr}");
    drop_scratch_db(&admin, pool, &name).await;
}

/// The relay serves only an active, unarchived community, so the script stops
/// before any rename for an archived community, one being deleted, or a host
/// no community has, and says why.
/// Mutation: drop the community check in the script → RED.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires Postgres"]
async fn script_stops_for_a_community_the_relay_does_not_serve() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db(&admin, "listed_name_script_state").await;
    let archived = Seed::new(&pool).await;
    let deleting = Seed::new(&pool).await;
    for seed in [&archived, &deleting] {
        seed.open(&pool, Uuid::new_v4(), "ops", 30).await;
        seed.open(&pool, Uuid::new_v4(), "ops", 10).await;
    }
    sqlx::query("UPDATE communities SET archived_at = now() WHERE id = $1")
        .bind(archived.community)
        .execute(&pool)
        .await
        .unwrap();
    crate::test_support::quiesce_community_for_tests(
        &pool,
        buzz_core::CommunityId::from_uuid(deleting.community),
    )
    .await;

    for (host, message) in [
        (archived.host.as_str(), "Unarchive it"),
        (deleting.host.as_str(), "being deleted"),
        ("missing.example", "no community has host"),
    ] {
        let (output, calls) = run_rename_script(host, &name, "exit 0\n").await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{host}: {stderr}");
        assert!(calls.is_empty(), "{host}: no rename");
        assert!(stderr.contains(message), "{host}: {stderr}");
    }
    drop_scratch_db(&admin, pool, &name).await;
}

async fn ledger_version(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .expect("read migration ledger")
}

async fn index_oid(pool: &PgPool) -> Option<i64> {
    sqlx::query_scalar("SELECT to_regclass('public.idx_channels_listed_name')::oid::bigint")
        .fetch_one(pool)
        .await
        .expect("look up index")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_listed_name_index_needs_the_renames_and_a_prebuild() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db_through(&admin, "listed_name_index", BEFORE_INDEX).await;
    let seed = Seed::new(&pool).await;
    let (kept, newer) = (Uuid::new_v4(), Uuid::new_v4());
    seed.open(&pool, kept, "square-ios", 30).await;
    seed.open(&pool, newer, "Square-iOS", 10).await;
    let before = seed.names(&pool).await;

    let error = migration::run_migrations(&pool).await.unwrap_err();
    assert!(
        error.to_string().contains("duplicate names remain"),
        "must name the duplicate rows, not merely fail: {error}"
    );
    assert_eq!(ledger_version(&pool).await, 78);
    assert_eq!(index_oid(&pool).await, None);
    assert_eq!(seed.names(&pool).await, before, "no data changes");

    assert_eq!(rename_duplicates(&pool, &seed.host).await, 1);
    assert!(rename_plan(&pool, &seed.host).await.is_empty());
    let error = migration::run_migrations(&pool).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("prebuild idx_channels_listed_name concurrently"),
        "a populated table is not indexed under a write-blocking lock: {error}"
    );
    assert_eq!(ledger_version(&pool).await, 78);
    assert_eq!(index_oid(&pool).await, None);

    crate::test_support::prebuild_listed_channel_name_index(&pool).await;
    migration::run_migrations(&pool)
        .await
        .expect("migrate after the renames and the prebuild");
    assert_eq!(ledger_version(&pool).await, 79);
    let mut expected = vec![
        (kept, "square-ios".to_owned()),
        renamed(newer, "Square-iOS"),
    ];
    expected.sort();
    assert_eq!(seed.names(&pool).await, expected);

    // The database now refuses what an older relay could still write.
    let duplicate = sqlx::query(
        "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
         VALUES ($1, $2, 'SQUARE-IOS', 'forum', 'open', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(seed.community)
    .bind(&seed.created_by)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(
        duplicate
            .as_database_error()
            .and_then(|e| e.constraint())
            .map(str::to_owned),
        Some("idx_channels_listed_name".to_owned())
    );
    drop_scratch_db(&admin, pool, &name).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn migration_schema_listed_name_index_validates_a_prebuild() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) =
        create_scratch_db_through(&admin, "listed_name_prebuild", BEFORE_INDEX).await;
    for remnant in [
        // A different predicate: archived channels left out.
        "CREATE UNIQUE INDEX idx_channels_listed_name ON channels (community_id, lower(name)) \
         WHERE visibility = 'open' AND NOT unlisted AND archived_at IS NULL",
        // A failed concurrent build. Simulated only in this disposable
        // superuser database.
        "CREATE UNIQUE INDEX idx_channels_listed_name ON channels (community_id, lower(name)) \
         WHERE visibility = 'open' AND NOT unlisted \
           AND channel_type IN ('stream', 'forum') AND deleted_at IS NULL; \
         UPDATE pg_index SET indisvalid = false \
         WHERE indexrelid = 'idx_channels_listed_name'::regclass",
    ] {
        sqlx::raw_sql(sqlx::AssertSqlSafe(remnant))
            .execute(&pool)
            .await
            .unwrap();
        let error = migration::run_migrations(&pool).await.unwrap_err();
        assert!(
            error.to_string().contains("invalid or wrong definition"),
            "must reject the catalog shape: {error}"
        );
        assert_eq!(ledger_version(&pool).await, 78);
        sqlx::query("DROP INDEX CONCURRENTLY idx_channels_listed_name")
            .execute(&pool)
            .await
            .unwrap();
    }

    crate::test_support::prebuild_listed_channel_name_index(&pool).await;
    let oid = index_oid(&pool).await;
    migration::run_migrations(&pool)
        .await
        .expect("accept the prebuild");
    assert_eq!(ledger_version(&pool).await, 79);
    assert_eq!(index_oid(&pool).await, oid, "the prebuild is not replaced");
    drop_scratch_db(&admin, pool, &name).await;
}
