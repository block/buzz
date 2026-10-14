//! `schema/` is the gateway's desired-state schema in the one-table-per-file
//! shape SchemaBot reads; `migrations/` is the incremental path the binary
//! applies at startup. These tests keep the two from drifting apart.
use super::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schema")
}

fn strip_comments(text: &str) -> String {
    text.lines()
        .map(|line| line.split_once("--").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `schema/schema.sql` includes every file under `schema/` exactly once and
/// holds nothing but `\i` lines. `schema/tables/public/<table>.sql` declares
/// exactly that table plus plain `CREATE [UNIQUE] INDEX ... ON <table>`
/// statements (SchemaBot refuses `IF NOT EXISTS` and `CONCURRENTLY`).
#[test]
fn desired_state_schema_layout_is_complete_and_schemabot_shaped() {
    fn sql_files(dir: &Path, root: &Path, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).expect("read schema directory") {
            let path = entry.expect("schema directory entry").path();
            if path.is_dir() {
                sql_files(&path, root, out);
            } else {
                let relative = path.strip_prefix(root).expect("schema-relative path");
                out.insert(relative.to_str().expect("utf-8 path").replace('\\', "/"));
            }
        }
    }

    let dir = schema_dir();
    let manifest = std::fs::read_to_string(dir.join("schema.sql")).expect("read schema.sql");
    let mut included = Vec::new();
    for line in strip_comments(&manifest).lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        let include = line.strip_prefix("\\i ").unwrap_or_else(|| {
            panic!(
                "schema/schema.sql may only hold \\i includes; \
                 put table DDL in tables/public/<table>.sql: {line:.80}"
            )
        });
        included.push(include.trim().to_owned());
    }
    let included_set: BTreeSet<_> = included.iter().cloned().collect();
    assert_eq!(
        included.len(),
        included_set.len(),
        "schema.sql includes a file twice"
    );

    let mut on_disk = BTreeSet::new();
    sql_files(&dir, &dir, &mut on_disk);
    on_disk.remove("schema.sql");
    assert_eq!(
        on_disk, included_set,
        "every schema/ file must be included by schema/schema.sql exactly once"
    );

    for file in &included {
        let table = file
            .strip_prefix("tables/public/")
            .and_then(|name| name.strip_suffix(".sql"))
            .unwrap_or_else(|| {
                panic!("{file}: the gateway schema holds only tables/public/<table>.sql")
            });
        let text = std::fs::read_to_string(dir.join(file)).expect("read table file");
        let mut creates = 0;
        for statement in strip_comments(&text)
            .split(';')
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
        {
            if let Some(rest) = statement.strip_prefix("CREATE TABLE ") {
                assert!(
                    rest.starts_with(&format!("{table} (")),
                    "{file} may only create table {table}: {statement:.80}"
                );
                creates += 1;
                continue;
            }
            let index = statement
                .strip_prefix("CREATE INDEX ")
                .or_else(|| statement.strip_prefix("CREATE UNIQUE INDEX "))
                .unwrap_or_else(|| {
                    panic!("{file} may only hold CREATE TABLE/INDEX: {statement:.80}")
                });
            assert!(
                !index.starts_with("IF NOT EXISTS ") && !index.starts_with("CONCURRENTLY "),
                "{file} must use a plain CREATE INDEX: {statement:.80}"
            );
            assert!(
                index.contains(&format!(" ON {table} ")),
                "{file} may only index {table}: {statement:.80}"
            );
        }
        assert_eq!(creates, 1, "{file} must create exactly one table");
    }
    assert!(
        !included.is_empty(),
        "schema/tables/public declares no tables"
    );
}

/// One probe database is bootstrapped from `schema/schema.sql` through the
/// real `bin/pgschema apply` binary (the SchemaBot-equivalent desired-state
/// path); another runs every gateway migration. Their `public` catalogs must
/// match: the same tables, column shapes (full type with typmod, default,
/// nullability, identity, generation, collation; keyed by name, since
/// migrations drop and append columns), table options, constraints, and index
/// shapes including per-key `indoption`. Neither side may hold functions,
/// triggers, policies, views, sequences, or user types. The whole catalog is
/// compared, so a new migration that is not mirrored in `schema/` fails here.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn desired_state_schema_matches_migrations() {
    type Rows = Vec<Vec<Option<String>>>;
    async fn catalog(pool: &PgPool, sql: &str) -> Rows {
        sqlx::query(AssertSqlSafe(sql.to_owned()))
            .fetch_all(pool)
            .await
            .expect("read catalog")
            .iter()
            .map(|row| {
                (0..row.len())
                    .map(|i| row.get::<Option<String>, _>(i))
                    .collect()
            })
            .collect()
    }
    const TABLES: &str = "SELECT tablename::text FROM pg_tables \
        WHERE schemaname = 'public' AND tablename <> '_sqlx_migrations' ORDER BY 1";
    // `pg_attribute` rather than `information_schema.columns`: the latter's
    // `data_type` drops typmods and array element types (`varchar(64)` and
    // `varchar(128)` both read `character varying`).
    const COLUMNS: &str = "SELECT t.relname::text, a.attname::text, \
        format_type(a.atttypid, a.atttypmod), a.attnotnull::text, \
        pg_get_expr(d.adbin, d.adrelid), a.attidentity::text, a.attgenerated::text, \
        a.attcollation::regcollation::text \
        FROM pg_attribute a JOIN pg_class t ON t.oid = a.attrelid \
        JOIN pg_namespace n ON n.oid = t.relnamespace \
        LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
        WHERE n.nspname = 'public' AND t.relkind = 'r' AND t.relname <> '_sqlx_migrations' \
        AND a.attnum > 0 AND NOT a.attisdropped ORDER BY 1, 2";
    const TABLE_OPTIONS: &str = "SELECT t.relname::text, t.relpersistence::text, \
        t.relrowsecurity::text, t.relforcerowsecurity::text, t.reloptions::text \
        FROM pg_class t JOIN pg_namespace n ON n.oid = t.relnamespace \
        WHERE n.nspname = 'public' AND t.relkind = 'r' AND t.relname <> '_sqlx_migrations' \
        ORDER BY 1";
    const CONSTRAINTS: &str = "SELECT t.relname::text, c.conname::text, c.contype::text, \
        pg_get_constraintdef(c.oid) \
        FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid \
        JOIN pg_namespace n ON n.oid = t.relnamespace \
        WHERE n.nspname = 'public' AND t.relname <> '_sqlx_migrations' ORDER BY 1, 2";
    const INDEXES: &str = "SELECT t.relname::text, c.relname::text, \
        pg_get_indexdef(i.indexrelid), i.indoption::int2[]::text \
        FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid \
        JOIN pg_class t ON t.oid = i.indrelid \
        JOIN pg_namespace n ON n.oid = t.relnamespace \
        WHERE n.nspname = 'public' AND t.relname <> '_sqlx_migrations' ORDER BY 1, 2";
    // `schema/` may only declare tables and indexes, so neither path may
    // create anything else in `public`: SchemaBot would never reproduce it.
    const NON_TABLE_OBJECTS: &str = "SELECT 'function', p.proname::text FROM pg_proc p \
            JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = 'public' \
        UNION ALL SELECT 'trigger', tg.tgname::text FROM pg_trigger tg \
            JOIN pg_class t ON t.oid = tg.tgrelid JOIN pg_namespace n ON n.oid = t.relnamespace \
            WHERE n.nspname = 'public' AND NOT tg.tgisinternal \
        UNION ALL SELECT 'policy', pol.polname::text FROM pg_policy pol \
            JOIN pg_class t ON t.oid = pol.polrelid JOIN pg_namespace n ON n.oid = t.relnamespace \
            WHERE n.nspname = 'public' \
        UNION ALL SELECT 'relation', c.relname::text FROM pg_class c \
            JOIN pg_namespace n ON n.oid = c.relnamespace \
            WHERE n.nspname = 'public' AND c.relkind NOT IN ('r', 'i') \
        UNION ALL SELECT 'type', ty.typname::text FROM pg_type ty \
            JOIN pg_namespace n ON n.oid = ty.typnamespace \
            WHERE n.nspname = 'public' AND ty.typtype IN ('d', 'e', 'r', 'm') \
        ORDER BY 1, 2";

    let base_url = std::env::var("BUZZ_TEST_DATABASE_URL").expect("test database URL");
    let url = url::Url::parse(&base_url).expect("parse test database URL");
    let host = url.host_str().expect("database host").to_owned();
    let port = url.port().unwrap_or(5432).to_string();
    let user = url.username().to_owned();
    let password = url
        .password()
        .map(str::to_owned)
        .or_else(|| std::env::var("PGPASSWORD").ok())
        .unwrap_or_default();
    let admin = PgPool::connect(&base_url)
        .await
        .expect("connect admin database");
    let probe_url = |db: &str| {
        let mut probe = url.clone();
        probe.set_path(db);
        probe.to_string()
    };

    let suffix = Uuid::new_v4().simple();
    let desired_db = format!("push_gateway_desired_{suffix}");
    let migrated_db = format!("push_gateway_migrated_{suffix}");
    for db in [&desired_db, &migrated_db] {
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {db}")))
            .execute(&admin)
            .await
            .expect("create probe database");
    }

    let pgschema = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bin/pgschema");
    let schema_file = schema_dir().join("schema.sql");
    let apply = std::process::Command::new(&pgschema)
        .args([
            "apply",
            "--auto-approve",
            "--file",
            schema_file.to_str().expect("schema path utf-8"),
            "--host",
            &host,
            "--port",
            &port,
            "--user",
            &user,
            "--password",
            &password,
            "--db",
            &desired_db,
            "--plan-host",
            &host,
            "--plan-port",
            &port,
            "--plan-user",
            &user,
            "--plan-password",
            &password,
            "--plan-db",
            &desired_db,
        ])
        .output()
        .expect("run bin/pgschema apply (hermit env required)");
    assert!(
        apply.status.success(),
        "pgschema apply failed: {}\n{}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr),
    );

    let desired = PgPool::connect(&probe_url(&desired_db))
        .await
        .expect("connect desired-state probe");
    let migrated = PgPool::connect(&probe_url(&migrated_db))
        .await
        .expect("connect migrated probe");
    GATEWAY_MIGRATOR
        .run(&migrated)
        .await
        .expect("apply gateway migrations");

    let declared: BTreeSet<String> = catalog(&desired, TABLES)
        .await
        .into_iter()
        .filter_map(|row| row.into_iter().next().flatten())
        .collect();
    let served: BTreeSet<String> = GATEWAY_TABLES.iter().map(|t| t.to_string()).collect();
    assert_eq!(
        declared, served,
        "schema/ must declare exactly GATEWAY_TABLES, the set readiness probes and \
         runtime grants cover"
    );
    for pool in [&desired, &migrated] {
        assert_eq!(
            catalog(pool, NON_TABLE_OBJECTS).await,
            Rows::new(),
            "the gateway schema holds only tables and indexes"
        );
    }
    for (what, sql) in [
        ("table set", TABLES),
        ("column", COLUMNS),
        ("table option", TABLE_OPTIONS),
        ("constraint", CONSTRAINTS),
        ("index", INDEXES),
    ] {
        assert_eq!(
            catalog(&desired, sql).await,
            catalog(&migrated, sql).await,
            "{what} parity mismatch: crates/buzz-push-gateway/schema/ has drifted from \
             migrations/; update the table file to the post-migration end state"
        );
    }

    desired.close().await;
    migrated.close().await;
    for db in [desired_db, migrated_db] {
        sqlx::query(AssertSqlSafe(format!("DROP DATABASE {db} WITH (FORCE)")))
            .execute(&admin)
            .await
            .expect("drop probe database");
    }
}
