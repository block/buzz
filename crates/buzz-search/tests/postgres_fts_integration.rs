//! Integration tests for community-scoped Postgres FTS.
//!
//! The shared search cases in [`search_suite`] run here against Postgres;
//! the tests below them cover behavior only the Postgres backend has.
//!
//! Run through `scripts/postgres-test-run.sh -p buzz-search --tests` for a
//! fresh desired-state database per test. Outside that lane, point
//! BUZZ_TEST_DATABASE_URL at a disposable dedicated database; fixtures use
//! the production migrator and unique community IDs.

mod search_suite;

use buzz_core::CommunityId;
use buzz_search::{ChannelScope, SearchQuery, SearchService};
use search_suite::{
    insert_event, mk_community, rand_bytes32, search_suite_tests, setup, teardown, Backend,
    TEST_DB_URL,
};
use sqlx::{postgres::PgPoolOptions, PgPool};

search_suite_tests! {
    Backend::Postgres;
    #[ignore = "requires Postgres"]
    fn search_finds_event_in_same_community,
    fn search_does_not_return_other_community_events,
    fn kind0_search_by_display_name_works_without_flattening,
    fn short_kind0_prefix_prioritizes_exact_lexeme_on_a_noisy_page,
    fn prefix_mode_matches_final_token_prefix_without_changing_full_text,
    fn prefix_mode_handles_tsquery_boundary_punctuation,
    fn prefix_mode_preserves_storage_level_privacy_exclusions,
    fn prefix_relevance_ranks_only_the_newest_matches,
    fn authors_filter_restricts_results,
    fn channel_scope_restricts_results,
    fn channel_scope_of_thousands_of_channels,
    fn deleted_events_are_excluded,
    fn empty_query_returns_empty_result_no_roundtrip,
    fn since_until_filters,
    fn pagination_works,
    fn newest_sort_pages_by_time_then_id_within_scope,
    fn channel_less_only_excludes_per_channel_events,
    fn nul_bytes_in_query_are_sanitized,
    fn enormous_page_number_is_clamped,
    fn very_long_query_is_bounded_before_pg_parse,
    fn excluded_kinds_are_storage_level_unsearchable,
    fn author_only_kinds_are_storage_level_unsearchable,
    fn p_gated_persistent_kinds_have_storage_null_tsvector,
    fn artifact_search_uses_only_live_current_head,
}

/// Insert a current kind:0 profile; returns its event id.
async fn put_profile(pool: &PgPool, community: CommunityId, content: &str, at: i64) -> [u8; 32] {
    let id = rand_bytes32();
    insert_event(pool, community, id, rand_bytes32(), 0, content, None, at).await;
    id
}

async fn profile_names(
    pool: &PgPool,
    community: CommunityId,
    q: &str,
    page: u32,
    per_page: u32,
) -> Vec<[u8; 32]> {
    SearchService::new(pool.clone())
        .search_profile_names(community, q, None, page, per_page)
        .await
        .expect("profile name search")
        .hits
        .into_iter()
        .map(|hit| hit.event_id)
        .collect()
}

/// Profile name search keeps the NIP-50 prefix rule (whole words in any
/// order, last word a prefix) but only `name` and `display_name` count. An
/// equal name ranks first, then a name prefix, then a later word prefix, then
/// other matches; ties stay newest first.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn profile_names_match_name_words_only() {
    let (pool, schema) = setup().await;
    let c = mk_community(&pool, "profile-names.example").await;
    let other = mk_community(&pool, "profile-names-other.example").await;

    let exact = put_profile(
        &pool,
        c,
        r#"{"display_name":"Wes","picture":"data:image/png;base64,P7+Wes+dYL"}"#,
        1_700_000_000,
    )
    .await;
    let westie_old = put_profile(&pool, c, r#"{"name":"Westie"}"#, 1_700_000_010).await;
    let westie_new = put_profile(&pool, c, r#"{"display_name":"Westie"}"#, 1_700_000_020).await;
    let word = put_profile(
        &pool,
        c,
        r#"{"name":"carl","display_name":"Carl (borrowed from Wes)"}"#,
        1_700_000_030,
    )
    .await;
    let full = put_profile(
        &pool,
        c,
        r#"{"name":"wjohnson","display_name":"Wes  Johnson"}"#,
        1_700_000_040,
    )
    .await;
    // Words split across the two fields still match, as in NIP-50 search.
    let split = put_profile(
        &pool,
        c,
        r#"{"name":"johnson","display_name":"Wes"}"#,
        1_700_000_050,
    )
    .await;
    // A word that only contains the query is not a match: same as NIP-50.
    put_profile(&pool, c, r#"{"name":"Dwes"}"#, 1_700_000_060).await;

    for (i, noise) in [
        r#"{"display_name":"Blueberry","picture":"data:image/png;base64,P7+Wes+dYL"}"#,
        r#"{"display_name":"Pollen","about":"Wes Johnson's bestie"}"#,
        r#"{"display_name":"Pollen","nip05":"wes@example.com"}"#,
        r#"{"display_name":"Pollen","banner":"https://wes.example/b.png"}"#,
        r#"{"name":["wes"],"display_name":{"text":"wes"}}"#,
        r#"not json wes"#,
        r#"["wes"]"#,
    ]
    .into_iter()
    .enumerate()
    {
        put_profile(&pool, c, noise, 1_700_000_100 + i as i64).await;
    }
    // A replaced profile (soft-deleted) no longer matches.
    let replaced = put_profile(&pool, c, r#"{"name":"wes"}"#, 1_699_000_000).await;
    sqlx::query("UPDATE events SET deleted_at = now() WHERE community_id = $1 AND id = $2")
        .bind(c.as_uuid())
        .bind(&replaced[..])
        .execute(&pool)
        .await
        .expect("replace profile");
    put_profile(&pool, other, r#"{"name":"Wes"}"#, 1_700_000_000).await;

    // Tier 0: exact (Wes; "Wes" display name of `split`), newest first.
    // Tier 1: name prefix (Wes Johnson, Westie x2). Tier 2: later word (Carl).
    let all = vec![split, exact, full, westie_new, westie_old, word];
    assert_eq!(profile_names(&pool, c, "WES", 1, 100).await, all);
    // Pages split the same order without overlap or gaps.
    assert_eq!(profile_names(&pool, c, "wes", 1, 4).await, all[..4]);
    assert_eq!(profile_names(&pool, c, "wes", 2, 4).await, all[4..]);
    assert!(profile_names(&pool, c, "wes", 3, 4).await.is_empty());

    // Word order does not matter; completed words must be whole words.
    assert_eq!(
        profile_names(&pool, c, "johnson wes", 1, 100).await,
        vec![split, full]
    );
    assert_eq!(
        profile_names(&pool, c, "wes  jo", 1, 100).await,
        vec![full, split]
    );
    assert!(profile_names(&pool, c, "estie", 1, 100).await.is_empty());
    assert!(profile_names(&pool, c, "bestie", 1, 100).await.is_empty());
    assert!(profile_names(&pool, c, "example", 1, 100).await.is_empty());
    assert!(profile_names(&pool, c, "%", 1, 100).await.is_empty());
    assert!(profile_names(&pool, c, "   ", 1, 100).await.is_empty());

    // The query and the names are normalized by the same SQL, so a non-ASCII
    // exact name is tier 0 whatever the database collation lower-cases.
    let emile = put_profile(&pool, c, r#"{"name":"Émile"}"#, 1_700_000_200).await;
    let emilee = put_profile(&pool, c, r#"{"name":"Émilee"}"#, 1_700_000_210).await;
    assert_eq!(
        profile_names(&pool, c, "Émile", 1, 100).await,
        vec![emile, emilee]
    );

    teardown(pool, &schema).await;
}

/// Stored kind:0 content is whatever the signer sent. Content that cannot
/// become jsonb must not fail the query for everyone else.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn profile_names_survive_unparsable_content() {
    let (pool, schema) = setup().await;
    let c = mk_community(&pool, "profile-names-unparsable.example").await;

    let plain = put_profile(&pool, c, r#"{"name":"Wes"}"#, 1_700_000_000).await;

    // Valid JSON objects that cannot become jsonb. Each one names Wes; it is
    // not findable, but it must not break the search.
    let deep = format!(
        r#"{{"name":"Wes","nested":{}{}}}"#,
        "[".repeat(60_000),
        "]".repeat(60_000)
    );
    for (i, poison) in [
        r#"{"name":"Wes","about":"\u0000"}"#,
        r#"{"name":"Wes","about":"\ud800"}"#,
        r#"{"name":"Wes","lud16":1e1000000}"#,
        deep.as_str(),
    ]
    .into_iter()
    .enumerate()
    {
        put_profile(&pool, c, poison, 1_700_000_100 + i as i64).await;
    }

    assert_eq!(profile_names(&pool, c, "wes", 1, 100).await, vec![plain]);

    // The nesting bound counts brackets, so content at the bound (4096,
    // counting the object's own `{`) still has names even when the other
    // brackets sit inside a string.
    let bracketed = put_profile(
        &pool,
        c,
        &format!(r#"{{"name":"Ada","about":"{}"}}"#, "[".repeat(4095)),
        1_700_000_200,
    )
    .await;
    assert_eq!(
        profile_names(&pool, c, "ada", 1, 100).await,
        vec![bracketed]
    );

    teardown(pool, &schema).await;
}

/// Insert a kind:30382 verified profile for `subject`, signed by `author`.
async fn put_verified(
    pool: &PgPool,
    community: CommunityId,
    author: [u8; 32],
    subject: [u8; 32],
    names: &[(&str, &str)],
    at: i64,
) -> [u8; 32] {
    let id = rand_bytes32();
    let d: String = subject.iter().map(|b| format!("{b:02x}")).collect();
    let mut tags = vec![format!(r#"["d","{d}"]"#), format!(r#"["p","{d}"]"#)];
    tags.extend(names.iter().map(|(k, v)| format!(r#"["{k}","{v}"]"#)));
    let tags = format!("[{}]", tags.join(","));
    sqlx::query(
        "INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig, d_tag) \
         VALUES ($1, $2, $3, to_timestamp($4), 30382, $5::jsonb, '', $6, $7)",
    )
    .bind(community.as_uuid())
    .bind(&id[..])
    .bind(&author[..])
    .bind(at)
    .bind(&tags)
    .bind(b"signature".as_slice())
    .bind(&d)
    .execute(pool)
    .await
    .expect("insert verified profile");
    // The relay keeps the newest one per person as the head.
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let event = format!(
        r#"{{"id":"{}","pubkey":"{}","created_at":{at},"kind":30382,"tags":{tags},"content":"","sig":""}}"#,
        hex(&id),
        hex(&author)
    );
    sqlx::query(
        "INSERT INTO nip_fi_profile_heads (community_id, pubkey, issuer, issued_at, event) \
         VALUES ($1, $2, 'issuer', 0, $3::jsonb) \
         ON CONFLICT (community_id, pubkey) DO UPDATE SET event = EXCLUDED.event",
    )
    .bind(community.as_uuid())
    .bind(&d)
    .bind(event)
    .execute(pool)
    .await
    .expect("insert verified profile head");
    id
}

/// Insert a kind:0 profile for `pubkey`; returns its event id.
async fn put_profile_for(
    pool: &PgPool,
    community: CommunityId,
    pubkey: [u8; 32],
    content: &str,
    at: i64,
) -> [u8; 32] {
    let id = rand_bytes32();
    insert_event(pool, community, id, pubkey, 0, content, None, at).await;
    id
}

async fn people(
    pool: &PgPool,
    community: CommunityId,
    q: &str,
    verified_by: Option<[u8; 32]>,
    page: u32,
    per_page: u32,
) -> Vec<[u8; 32]> {
    SearchService::new(pool.clone())
        .search_profile_names(community, q, verified_by, page, per_page)
        .await
        .expect("profile name search")
        .hits
        .into_iter()
        .map(|hit| hit.event_id)
        .collect()
}

/// With a verifying key, people also match on the names in their current
/// kind:30382 signed by that key. Pages count people; each person returns
/// their kind:0 then their verified profile, whichever matched.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn profile_names_match_verified_names() {
    let (pool, schema) = setup().await;
    let c = mk_community(&pool, "profile-names-verified.example").await;
    let other = mk_community(&pool, "profile-names-verified-other.example").await;
    let relay = rand_bytes32();
    let forger = rand_bytes32();

    let fake = rand_bytes32();
    let fake_profile = put_profile_for(
        &pool,
        c,
        fake,
        r#"{"name":"Fake","display_name":"Fak"}"#,
        1_700_000_000,
    )
    .await;
    let fake_verified = put_verified(
        &pool,
        c,
        relay,
        fake,
        &[
            ("display_name", "fname"),
            ("username", "fname"),
            ("full_name", "Fake Name"),
        ],
        1_700_000_010,
    )
    .await;
    // Another Fake, never verified.
    let namesake = rand_bytes32();
    let namesake_profile =
        put_profile_for(&pool, c, namesake, r#"{"name":"Fake"}"#, 1_700_000_020).await;
    // Verified, with no kind:0 at all.
    let gina = rand_bytes32();
    let gina_verified = put_verified(
        &pool,
        c,
        relay,
        gina,
        &[("display_name", "gname"), ("full_name", "Name Gina")],
        1_700_000_030,
    )
    .await;
    // Signed by another key: not a verified profile.
    let fred = rand_bytes32();
    put_profile_for(&pool, c, fred, r#"{"name":"Fred"}"#, 1_700_000_040).await;
    put_verified(
        &pool,
        c,
        forger,
        fred,
        &[("full_name", "Fred Name")],
        1_700_000_050,
    )
    .await;
    // Replaced (soft-deleted) verified profile.
    let dan = rand_bytes32();
    put_profile_for(&pool, c, dan, r#"{"name":"Dan"}"#, 1_700_000_060).await;
    let replaced = put_verified(
        &pool,
        c,
        relay,
        dan,
        &[("full_name", "Dan Name")],
        1_700_000_070,
    )
    .await;
    sqlx::query("UPDATE events SET deleted_at = now() WHERE community_id = $1 AND id = $2")
        .bind(c.as_uuid())
        .bind(&replaced[..])
        .execute(&pool)
        .await
        .expect("replace verified profile");
    // Another community.
    put_verified(
        &pool,
        other,
        relay,
        rand_bytes32(),
        &[("full_name", "Olga Name")],
        1_700_000_080,
    )
    .await;

    // Gina is tier 1 (a name starts with "name"); Fake is tier 2.
    let by_name = vec![gina_verified, fake_profile, fake_verified];
    assert_eq!(people(&pool, c, "name", Some(relay), 1, 100).await, by_name);
    assert_eq!(
        people(&pool, c, "NAME", Some(relay), 1, 1).await,
        by_name[..1]
    );
    assert_eq!(
        people(&pool, c, "name", Some(relay), 2, 1).await,
        by_name[1..]
    );
    assert!(people(&pool, c, "name", Some(relay), 3, 1).await.is_empty());
    assert_eq!(
        people(&pool, c, "fnam", Some(relay), 1, 100).await,
        vec![fake_profile, fake_verified]
    );
    assert_eq!(
        people(&pool, c, "name fake", Some(relay), 1, 100).await,
        vec![fake_profile, fake_verified]
    );
    // Full name and username together: both are in the verified profile.
    assert_eq!(
        people(&pool, c, "fake fname", Some(relay), 1, 100).await,
        vec![fake_profile, fake_verified]
    );
    // A kind:0 match still returns the person's verified profile.
    let fakes = people(&pool, c, "fake", Some(relay), 1, 100).await;
    assert_eq!(fakes.len(), 3, "{fakes:?}");
    let at = fakes
        .iter()
        .position(|id| *id == fake_profile)
        .expect("fake");
    assert_eq!(fakes.get(at + 1), Some(&fake_verified));
    assert!(fakes.contains(&namesake_profile));
    // Verified names need a three-character prefix; kind:0 does not.
    assert!(people(&pool, c, "gn", Some(relay), 1, 100).await.is_empty());
    assert_eq!(
        people(&pool, c, "gna", Some(relay), 1, 100).await,
        vec![gina_verified]
    );
    let mut short = people(&pool, c, "fa", Some(relay), 1, 100).await;
    short.sort();
    let mut full = people(&pool, c, "fake", Some(relay), 1, 100).await;
    full.sort();
    assert_eq!(short, full);
    // Words split between kind:0 and the verified profile do not match.
    assert!(people(&pool, c, "fak fname", Some(relay), 1, 100)
        .await
        .is_empty());

    // Without a verifying key, only kind:0 counts, as before.
    assert!(people(&pool, c, "name", None, 1, 100).await.is_empty());
    let mut fakes = people(&pool, c, "fake", None, 1, 100).await;
    fakes.sort();
    let mut want = vec![fake_profile, namesake_profile];
    want.sort();
    assert_eq!(fakes, want);
    // The forger's key verifies only its own events.
    assert!(people(&pool, c, "fred name", Some(relay), 1, 100)
        .await
        .is_empty());

    // A person ranks by their single best match. Kim's exact kind:0 name
    // outranks Ken's, and Ken's better-ranked verified match has a worse tier.
    let ken = rand_bytes32();
    let ken_profile = put_profile_for(&pool, c, ken, r#"{"name":"Kname"}"#, 1_700_000_100).await;
    put_verified(
        &pool,
        c,
        relay,
        ken,
        &[
            ("display_name", "Doctor Kname"),
            ("username", "doctor-kname"),
            ("full_name", "Doctor Kname"),
        ],
        1_700_000_110,
    )
    .await;
    let kim = rand_bytes32();
    let kim_profile = put_profile_for(
        &pool,
        c,
        kim,
        r#"{"name":"Kname","display_name":"Kname"}"#,
        1_700_000_000,
    )
    .await;
    assert_eq!(
        people(&pool, c, "kname", Some(relay), 1, 1).await,
        vec![kim_profile]
    );
    assert_eq!(
        people(&pool, c, "kname", Some(relay), 2, 1).await.first(),
        Some(&ken_profile)
    );

    teardown(pool, &schema).await;
}

/// Verified-name search reads `idx_nip_fi_profile_heads_names`.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn profile_names_use_verified_name_index() {
    let (setup_pool, schema) = setup().await;
    let c = mk_community(&setup_pool, "profile-names-verified-index.example").await;
    let relay = rand_bytes32();
    for i in 0..2_000 {
        put_verified(
            &setup_pool,
            c,
            relay,
            rand_bytes32(),
            &[("full_name", &format!("Person{i} Name{i}"))],
            1_700_000_000 + i,
        )
        .await;
    }
    sqlx::query("ANALYZE nip_fi_profile_heads")
        .execute(&setup_pool)
        .await
        .expect("analyze heads");
    // One connection, so this backend's statistics hold the search's scans.
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect test database");
    let scans = || async {
        sqlx::query("SELECT pg_stat_force_next_flush()")
            .execute(&pool)
            .await
            .expect("flush statistics");
        sqlx::query("SELECT pg_stat_clear_snapshot()")
            .execute(&pool)
            .await
            .expect("clear statistics snapshot");
        sqlx::query_scalar::<_, i64>(
            "SELECT idx_scan FROM pg_stat_user_indexes \
             WHERE indexrelname = 'idx_nip_fi_profile_heads_names'",
        )
        .fetch_one(&pool)
        .await
        .expect("index statistics")
    };
    let before = scans().await;
    let found = SearchService::new(pool.clone())
        .search_profile_names(c, "name1234", Some(relay), 1, 10)
        .await
        .expect("profile name search")
        .hits;
    assert_eq!(found.len(), 1);
    assert!(
        scans().await > before,
        "verified-name search did not use its index"
    );
    pool.close().await;
    teardown(setup_pool, &schema).await;
}

/// Connect the way the relay does: [`buzz_search::pool_options`].
async fn search_pool() -> PgPool {
    let url = std::env::var("BUZZ_TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.to_string());
    buzz_search::pool_options()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("connect search pool")
}

/// Backends in this test's database, other than the caller's, that wait on
/// a lock inside a search. Scoped to the database so concurrent tests in
/// sibling databases cannot change the count. Matches each search by its
/// leading text, since `pg_stat_activity` keeps only the first 1 kB of a query.
async fn waiting_searches(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE pid <> pg_backend_pid() \
         AND datname = current_database() AND wait_event_type = 'Lock' \
         AND (query LIKE 'SELECT id, kind, pubkey, channel_id, %ts_rank_cd(%' \
           OR query LIKE 'WITH search_query AS (SELECT %')",
    )
    .fetch_one(pool)
    .await
    .expect("count waiting searches")
}

/// Poll until [`waiting_searches`] is `want`, failing after five seconds.
async fn await_waiting_searches(pool: &PgPool, want: i64, why: &str) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let got = waiting_searches(pool).await;
        if got == want {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{why}: {got} waiting searches"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// Hold `start()`'s search on a table lock, drop it while it waits, and
/// require that its statement stops; then run it again unlocked.
async fn abandon_under_lock<F, Fut>(pool: &PgPool, name: &str, start: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<buzz_search::SearchResult, buzz_search::SearchError>>,
{
    let mut lock = pool.begin().await.expect("begin lock");
    sqlx::query("LOCK TABLE events IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .expect("lock events");
    // Boxed so `drop` drops the search itself, not a pinned reference to it.
    let mut running = Box::pin(start());
    tokio::select! {
        result = &mut running => panic!("{name} finished under the lock: {result:?}"),
        () = await_waiting_searches(pool, 1, "search never reached the lock") => {}
    }
    drop(running);
    await_waiting_searches(pool, 0, &format!("abandoned {name} still running")).await;
    lock.rollback().await.expect("release lock");
    // The cancelled connection is usable again.
    start()
        .await
        .unwrap_or_else(|e| panic!("{name} after cancel: {e}"));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn search_pool_bounds_every_statement() {
    let (pool, schema) = setup().await;
    let search = search_pool().await;
    let mut conns = Vec::new();
    for _ in 0..2 {
        let mut conn = search.acquire().await.expect("search connection");
        let timeout: String = sqlx::query_scalar("SHOW statement_timeout")
            .fetch_one(&mut *conn)
            .await
            .expect("read statement_timeout");
        assert_eq!(timeout, "20s", "every search connection is bounded");
        conns.push(conn);
    }
    drop(conns);
    search.close().await;
    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn abandoned_search_cancels_its_statement() {
    let (pool, schema) = setup().await;
    let c = mk_community(&pool, "abandoned-search.example").await;
    insert_event(
        &pool,
        c,
        rand_bytes32(),
        rand_bytes32(),
        9,
        "project plan",
        None,
        1_700_000_000,
    )
    .await;
    let svc = SearchService::new(search_pool().await);
    let message_query = SearchQuery {
        community: c,
        q: "project".into(),
        channel_scope: ChannelScope::Any,
        kinds: Some(vec![9]),
        authors: None,
        since: None,
        until: None,
        page: 1,
        per_page: 10,
        mode: buzz_search::SearchMode::Prefix,
        sort: buzz_search::SearchSort::Relevance,
    };
    abandon_under_lock(&pool, "search", || svc.search(&message_query)).await;
    abandon_under_lock(&pool, "search_profile_names", || {
        svc.search_profile_names(c, "proj", None, 1, 10)
    })
    .await;
    teardown(pool, &schema).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn cancel_signals_only_the_named_backend() {
    let (pool, schema) = setup().await;
    let mut victim = pool.acquire().await.expect("victim connection");
    let (pid, started_us): (i32, i64) = sqlx::query_as(
        "SELECT pid, (EXTRACT(EPOCH FROM backend_start) * 1000000)::bigint \
         FROM pg_stat_activity WHERE pid = pg_backend_pid()",
    )
    .fetch_one(&mut *victim)
    .await
    .expect("victim backend");
    let sleeping = tokio::spawn(async move {
        sqlx::query("SELECT pg_sleep(30)")
            .execute(&mut *victim)
            .await
            .map(|_| ())
    });
    // Signal only once the victim's statement is running: a cancel that
    // lands first is a no-op, and the named cancel below would then pass
    // without stopping anything.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let running: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
             WHERE pid = $1 AND state = 'active' AND query LIKE '%pg_sleep(30)%')",
        )
        .bind(pid)
        .fetch_one(&pool)
        .await
        .expect("victim activity");
        if running {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "victim statement never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    // A same-pid backend that started at another time (another replica, or a
    // reused pid) is not signalled.
    let other: Option<bool> = sqlx::query_scalar(buzz_search::cancel::CANCEL_SQL)
        .bind(pid)
        .bind(started_us + 1)
        .fetch_optional(&pool)
        .await
        .expect("cancel other backend");
    assert_eq!(other, None);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        !sleeping.is_finished(),
        "an unmatched cancel must not stop the statement"
    );

    let named: Option<bool> = sqlx::query_scalar(buzz_search::cancel::CANCEL_SQL)
        .bind(pid)
        .bind(started_us)
        .fetch_optional(&pool)
        .await
        .expect("cancel named backend");
    assert_eq!(named, Some(true));
    let err = tokio::time::timeout(std::time::Duration::from_secs(5), sleeping)
        .await
        .expect("cancelled statement ends")
        .expect("join")
        .expect_err("statement was cancelled");
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("57014")
    );
    teardown(pool, &schema).await;
}
