//! Restart and isolation contract for the public session-store API.

use std::path::Path;

use buzz_acp::session_store::sqlite::SqliteSessionStore;
use buzz_acp::session_store::{ContextKey, SessionStore, StoreScope};
use rusqlite::params;
use uuid::Uuid;

fn scope_a() -> StoreScope {
    StoreScope::new(
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "wss://relay.example/",
        "adapter-a",
    )
}

fn scope_b() -> StoreScope {
    StoreScope::new(
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "wss://relay.example",
        "adapter-b",
    )
}

fn db_path(dir: &Path) -> std::path::PathBuf {
    dir.join("sessions.db")
}

#[tokio::test]
async fn session_store_bindings_survive_reopen() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = db_path(dir.path());
    let key = ContextKey::Channel(Uuid::new_v4());

    let store = SqliteSessionStore::open(&path, scope_a()).expect("open");
    store.save_binding(&key, "ses_persist").await.expect("save");
    let first = store
        .load_binding(&key)
        .await
        .expect("load")
        .expect("binding");
    drop(store);

    let store = SqliteSessionStore::open(&path, scope_a()).expect("reopen");
    let again = store
        .load_binding(&key)
        .await
        .expect("load after reopen")
        .expect("binding after reopen");
    assert_eq!(again.session_id, "ses_persist");
    assert_eq!(again.created_at, first.created_at);
    assert_eq!(again.last_used_at, first.last_used_at);
}

#[tokio::test]
async fn session_store_processed_events_survive_reopen_and_dedupe() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = db_path(dir.path());
    let channel = Uuid::new_v4();

    let store = SqliteSessionStore::open(&path, scope_a()).expect("open");
    store
        .mark_events_processed(channel, &["evt-1".into(), "evt-2".into()])
        .await
        .expect("mark");
    drop(store);

    let store = SqliteSessionStore::open(&path, scope_a()).expect("reopen");
    assert!(store.is_event_processed("evt-1").await.expect("check 1"));
    assert!(store.is_event_processed("evt-2").await.expect("check 2"));
    assert!(!store
        .is_event_processed("evt-unknown")
        .await
        .expect("check unknown"));
    let mut ids = store
        .processed_event_ids_for_channel(channel)
        .await
        .expect("channel ids");
    ids.sort();
    assert_eq!(ids, vec!["evt-1", "evt-2"]);
}

#[tokio::test]
async fn session_store_scopes_are_isolated() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = db_path(dir.path());
    let key = ContextKey::Heartbeat;

    let a = SqliteSessionStore::open(&path, scope_a()).expect("open a");
    a.save_binding(&key, "ses_a").await.expect("save a");

    let b = SqliteSessionStore::open(&path, scope_b()).expect("open b");
    b.save_binding(&key, "ses_b").await.expect("save b");

    assert_eq!(
        a.load_binding(&key)
            .await
            .expect("load a")
            .unwrap()
            .session_id,
        "ses_a"
    );
    assert_eq!(
        b.load_binding(&key)
            .await
            .expect("load b")
            .unwrap()
            .session_id,
        "ses_b"
    );
}

#[tokio::test]
async fn session_store_malformed_binding_is_discarded_not_resumed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = db_path(dir.path());
    let channel = Uuid::new_v4();
    let scope = scope_a();

    {
        let store = SqliteSessionStore::open(&path, scope.clone()).expect("bootstrap schema");
        drop(store);
        let conn = rusqlite::Connection::open(&path).expect("raw open");
        conn.execute(
            "INSERT INTO session_bindings (
                scope_agent, scope_relay, scope_adapter, context_key,
                session_id, created_at, last_used_at
             ) VALUES (?1, ?2, ?3, ?4, '', 1, 1)",
            params![
                scope.agent_pubkey,
                scope.relay_url,
                scope.adapter,
                channel.to_string()
            ],
        )
        .expect("insert empty session_id");
    }

    let store = SqliteSessionStore::open(&path, scope.clone()).expect("reopen");
    let loaded = store
        .load_binding(&ContextKey::Channel(channel))
        .await
        .expect("load malformed");
    assert!(loaded.is_none(), "malformed binding must not resume");

    let remaining: i64 = {
        let conn = rusqlite::Connection::open(&path).expect("raw reopen");
        conn.query_row(
            "SELECT COUNT(*) FROM session_bindings
             WHERE scope_agent = ?1 AND context_key = ?2",
            params![scope.agent_pubkey, channel.to_string()],
            |row| row.get(0),
        )
        .expect("count")
    };
    assert_eq!(remaining, 0, "malformed row must be deleted");
}

#[tokio::test]
async fn session_store_file_contains_ids_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = db_path(dir.path());
    let store = SqliteSessionStore::open(&path, scope_a()).expect("open");
    let channel = Uuid::new_v4();
    store
        .save_binding(&ContextKey::Channel(channel), "ses_ids_only")
        .await
        .expect("save");
    store
        .mark_events_processed(channel, &["evt-ids-only".into()])
        .await
        .expect("mark");
    drop(store);

    let bytes = std::fs::read(&path).expect("read db");
    let haystack = String::from_utf8_lossy(&bytes);
    assert!(
        !haystack.contains("SENTINEL_PROMPT_DO_NOT_PERSIST"),
        "store must never contain a prompt that was never written"
    );
    assert!(
        !haystack.contains("nsec"),
        "store must not contain key material tokens"
    );

    let conn = rusqlite::Connection::open(&path).expect("pragma open");
    let binding_cols = table_columns(&conn, "session_bindings");
    assert_eq!(
        binding_cols,
        vec![
            "scope_agent",
            "scope_relay",
            "scope_adapter",
            "context_key",
            "session_id",
            "created_at",
            "last_used_at"
        ]
    );
    let event_cols = table_columns(&conn, "processed_events");
    assert_eq!(
        event_cols,
        vec![
            "scope_agent",
            "scope_relay",
            "scope_adapter",
            "event_id",
            "channel_id",
            "processed_at"
        ]
    );
}

#[tokio::test]
async fn session_store_wal_concurrent_open_within_busy_timeout() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = db_path(dir.path());
    let a = SqliteSessionStore::open(&path, scope_a()).expect("open a");
    let b = SqliteSessionStore::open(&path, scope_b()).expect("open b");
    let key_a = ContextKey::Channel(Uuid::new_v4());
    let key_b = ContextKey::Channel(Uuid::new_v4());

    let (ra, rb) = tokio::join!(
        a.save_binding(&key_a, "ses_race_a"),
        b.save_binding(&key_b, "ses_race_b"),
    );
    ra.expect("race write a");
    rb.expect("race write b");
    assert_eq!(
        a.load_binding(&key_a)
            .await
            .expect("load a")
            .unwrap()
            .session_id,
        "ses_race_a"
    );
    assert_eq!(
        b.load_binding(&key_b)
            .await
            .expect("load b")
            .unwrap()
            .session_id,
        "ses_race_b"
    );
}

#[tokio::test]
async fn session_store_worker_scoped_bindings_and_channel_removal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = db_path(dir.path());
    let store = SqliteSessionStore::open(&path, scope_a()).expect("open");
    let channel = Uuid::new_v4();
    let other = Uuid::new_v4();
    let worker0 = ContextKey::Channel(channel).for_worker(0);
    let worker1 = ContextKey::Channel(channel).for_worker(1);
    let other_key = ContextKey::Channel(other).for_worker(0);
    store
        .save_binding(&worker0, "ses_w0")
        .await
        .expect("save w0");
    store
        .save_binding(&worker1, "ses_w1")
        .await
        .expect("save w1");
    store
        .save_binding(&other_key, "ses_other")
        .await
        .expect("save other");
    drop(store);

    let store = SqliteSessionStore::open(&path, scope_a()).expect("reopen");
    assert_eq!(
        store
            .load_binding(&worker0)
            .await
            .expect("load w0")
            .unwrap()
            .session_id,
        "ses_w0"
    );
    store
        .remove_bindings_for_channel(channel)
        .await
        .expect("remove channel");
    assert!(store.load_binding(&worker0).await.unwrap().is_none());
    assert!(store.load_binding(&worker1).await.unwrap().is_none());
    assert_eq!(
        store
            .load_binding(&other_key)
            .await
            .unwrap()
            .unwrap()
            .session_id,
        "ses_other"
    );
}

fn table_columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .expect("pragma");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("rows");
    rows.map(|r| r.expect("col")).collect()
}
