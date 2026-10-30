//! scripts/maintenance/rename_duplicate_listed_channel_names.sql leaves no two
//! listed channels in a community with one name, ignoring case. It keeps the
//! name on the oldest unarchived channel, picks a new name that is free, and
//! leaves alone a row that a relay changed while the script waited for it.

use super::*;

const REPAIR_SCRIPT: &str =
    include_str!("../../../../../scripts/maintenance/rename_duplicate_listed_channel_names.sql");

struct Seed {
    community: Uuid,
    created_by: Vec<u8>,
}

impl Seed {
    async fn new(pool: &PgPool) -> Self {
        let community = Uuid::new_v4();
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community)
            .bind(format!("listed-name-{}.example", community.simple()))
            .execute(pool)
            .await
            .expect("insert community");
        Self {
            community,
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

/// Runs the repair the way `psql -f` does: the `DO` block as its own
/// statement, so that its per-batch COMMITs run outside a transaction block.
/// A repair that stops making progress fails at the statement timeout.
/// Returns the number of rows the script's validation query reports.
pub(super) async fn run_repair(pool: &PgPool) -> i64 {
    let (repair, validation) = REPAIR_SCRIPT
        .split_once("\n-- Validation.")
        .expect("script has a validation query");
    let mut conn = pool.acquire().await.expect("acquire repair connection");
    sqlx::query("SET statement_timeout = '30s'")
        .execute(&mut *conn)
        .await
        .expect("bound the repair");
    sqlx::raw_sql(sqlx::AssertSqlSafe(repair))
        .execute(&mut *conn)
        .await
        .expect("run repair");
    drop(conn);
    let (_, query) = validation.split_once("\nSELECT").expect("validation query");
    sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM (SELECT{}) AS remaining",
        query.trim_end().trim_end_matches(';')
    )))
    .fetch_one(pool)
    .await
    .expect("run validation")
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repair_keeps_each_listed_name_on_its_oldest_unarchived_channel() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db(&admin, "listed_name_repair").await;
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
    // A community with no duplicates is not touched.
    let other = Seed::new(&pool).await;
    other.open(&pool, Uuid::new_v4(), "square-ios", 0).await;
    let other_before = other.names(&pool).await;

    assert_eq!(run_repair(&pool).await, 0, "no duplicates remain");
    let mut expected = vec![
        (kept, "square-ios".to_owned()),
        renamed(archived, "Square-iOS"),
        renamed(newer, "SQUARE-IOS"),
        (private, "square-ios".to_owned()),
        (deleted, "square-ios".to_owned()),
    ];
    expected.sort();
    assert_eq!(seed.names(&pool).await, expected);
    assert_eq!(other.names(&pool).await, other_before);

    assert_eq!(run_repair(&pool).await, 0);
    assert_eq!(
        seed.names(&pool).await,
        expected,
        "a second run changes nothing"
    );
    drop_scratch_db(&admin, pool, &name).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repair_picks_a_free_name_when_the_short_one_is_taken() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db(&admin, "listed_name_repair_taken").await;
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

    assert_eq!(run_repair(&pool).await, 0, "no duplicates remain");
    let mut expected = vec![
        (kept, long.clone()),
        (duplicate, format!("{}-{duplicate}", "a".repeat(218))),
        (squatter, short),
    ];
    expected.sort();
    assert_eq!(seed.names(&pool).await, expected);
    assert!(expected.iter().all(|(_, name)| name.chars().count() <= 255));
    drop_scratch_db(&admin, pool, &name).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn repair_leaves_a_row_that_stopped_being_a_duplicate_while_it_waited() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db(&admin, "listed_name_repair_wait").await;
    let seed = Seed::new(&pool).await;
    let (kept, duplicate) = (Uuid::new_v4(), Uuid::new_v4());
    seed.open(&pool, kept, "payments", 30).await;
    seed.open(&pool, duplicate, "payments", 10).await;

    // A relay write holds the duplicate's row while the script starts.
    let mut relay = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM channels WHERE community_id = $1 AND id = $2 FOR NO KEY UPDATE")
        .bind(seed.community)
        .bind(duplicate)
        .execute(&mut *relay)
        .await
        .unwrap();
    let repair_pool = PgPool::connect_with(
        (*pool.connect_options())
            .clone()
            .application_name("listed-name-repair"),
    )
    .await
    .unwrap();
    let repair = tokio::spawn({
        let repair_pool = repair_pool.clone();
        async move { run_repair(&repair_pool).await }
    });
    crate::test_support::wait_for_lock_wait(&pool, "listed-name-repair").await;

    // The relay makes the channel private and renames it, then commits.
    sqlx::query(
        "UPDATE channels SET name = 'payments-private', visibility = 'private' \
         WHERE community_id = $1 AND id = $2",
    )
    .bind(seed.community)
    .bind(duplicate)
    .execute(&mut *relay)
    .await
    .unwrap();
    relay.commit().await.unwrap();

    assert_eq!(repair.await.unwrap(), 0);
    let mut expected = vec![
        (kept, "payments".to_owned()),
        (duplicate, "payments-private".to_owned()),
    ];
    expected.sort();
    assert_eq!(
        seed.names(&pool).await,
        expected,
        "the private row is not renamed"
    );
    repair_pool.close().await;
    drop_scratch_db(&admin, pool, &name).await;
}
