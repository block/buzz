//! scripts/maintenance/plan_duplicate_listed_channel_renames.sql lists the
//! renames that leave no two listed channels in a community with one name,
//! ignoring case. It keeps the name on the oldest unarchived channel and picks
//! a new name that no other listed channel has.

use super::*;

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
pub(super) async fn rename_plan(pool: &PgPool, host: &str) -> Vec<(Uuid, String, String)> {
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
pub(super) async fn rename_duplicates(pool: &PgPool, host: &str) -> usize {
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
    let (pool, name) = create_scratch_db(&admin, "listed_name_plan").await;
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
    let (pool, name) = create_scratch_db(&admin, "listed_name_plan_taken").await;
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

/// The real script, with a stand-in CLI that logs each call and applies the
/// rename. A name may hold any character but NUL, so a name holding control
/// characters must still yield exactly one rename, of its own channel. Like
/// clap, the stand-in takes a value that starts with "-" only as `--name=`.
#[cfg(unix)]
#[tokio::test]
#[ignore = "requires Postgres"]
async fn script_renames_each_planned_channel_once_whatever_its_name() {
    let admin = PgPool::connect(&admin_url().await).await.unwrap();
    let (pool, name) = create_scratch_db(&admin, "listed_name_script").await;
    let seed = Seed::new(&pool).await;
    let [kept, duplicate, other] = [(); 3].map(|_| Uuid::new_v4());
    let tricky = format!("-ops\x1e{other}\x1fowned\x1f\n\t\"$(x)");
    seed.open(&pool, kept, &tricky, 30).await;
    seed.open(&pool, duplicate, &tricky, 10).await;
    seed.open(&pool, other, "other", 20).await;

    let dir = std::env::temp_dir().join(format!("listed-name-script-{}", Uuid::new_v4().simple()));
    std::fs::create_dir(&dir).unwrap();
    let (cli, log) = (dir.join("buzz"), dir.join("calls"));
    std::fs::write(
        &cli,
        "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\0' \"$@\" >>\"$STUB_LOG\"\n\
         [[ $3 == --channel=* && $4 == --name=* ]] || exit 2\n\
         psql \"$DATABASE_URL\" -X -q -v ON_ERROR_STOP=1 -v id=\"${3#*=}\" -v name=\"${4#*=}\" <<'SQL'\n\
         UPDATE channels SET name = :'name' WHERE id = :'id';\nSQL\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();

    let base = admin_url().await;
    let database_url = format!("{}/{}", &base[..base.rfind('/').unwrap()], name);
    let mut path = std::env::var("PATH").unwrap_or_default();
    if let Ok(bin) = std::env::var("PG_BIN_DIR") {
        path = format!("{bin}:{path}");
    }
    let output = std::process::Command::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/maintenance/rename_duplicate_listed_channel_names.sh"
    ))
    .arg(&seed.host)
    .env("PATH", path)
    .env("DATABASE_URL", &database_url)
    .env("BUZZ_PRIVATE_KEY", "unused")
    .env("BUZZ_CLI", &cli)
    .env("STUB_LOG", &log)
    .output()
    .expect("run the rename script");
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let new_name = renamed(duplicate, &tricky).1;
    let calls = std::fs::read(&log).unwrap();
    let args: Vec<&[u8]> = calls.split(|b| *b == 0).collect();
    let channel = format!("--channel={duplicate}");
    let name_arg = format!("--name={new_name}");
    let expected: Vec<&[u8]> = vec![
        b"channels",
        b"update",
        channel.as_bytes(),
        name_arg.as_bytes(),
        b"",
    ];
    assert_eq!(args, expected, "one call, for the duplicate only");
    let mut names = vec![
        (kept, tricky.clone()),
        (duplicate, new_name.clone()),
        (other, "other".to_owned()),
    ];
    names.sort();
    assert_eq!(seed.names(&pool).await, names);
    std::fs::remove_dir_all(&dir).unwrap();
    drop_scratch_db(&admin, pool, &name).await;
}
