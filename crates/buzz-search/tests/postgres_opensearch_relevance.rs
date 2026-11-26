//! OpenSearch ranking: typos, rare words, phrases, word parts, accents,
//! markdown, stems, prefixes and recency, each checked through
//! [`SearchService::search`] against a real cluster.
//!
//! Needs Postgres (run through `scripts/postgres-test-run.sh -p buzz-search
//! --tests`) and an OpenSearch cluster at `BUZZ_TEST_OPENSEARCH_URL`
//! (default `http://localhost:9200`).

#[allow(unused_macros, unused_imports)]
mod search_suite;

use buzz_core::CommunityId;
use buzz_search::{ChannelScope, SearchMode, SearchQuery, SearchSort};
use search_suite::{insert_event, mk_community, rand_bytes32, Backend, Fixture};
use sqlx::PgPool;
use uuid::Uuid;

const DAY: i64 = 86_400;

/// A community of its own on a fresh OpenSearch fixture.
async fn setup() -> (PgPool, Fixture, CommunityId) {
    let (pool, fixture) = Fixture::setup(Backend::OpenSearch).await;
    let host = format!("{}.relevance.example", Uuid::new_v4().simple());
    let community = mk_community(&pool, &host).await;
    (pool, fixture, community)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Store a kind 9 message `age` seconds old, queued for indexing.
async fn post(pool: &PgPool, community: CommunityId, content: &str, age: i64) -> [u8; 32] {
    let id = rand_bytes32();
    insert_event(
        pool,
        community,
        id,
        rand_bytes32(),
        9,
        content,
        None,
        now() - age,
    )
    .await;
    id
}

fn query(community: CommunityId, q: &str, mode: SearchMode, sort: SearchSort) -> SearchQuery {
    SearchQuery {
        community,
        q: q.into(),
        channel_scope: ChannelScope::Any,
        kinds: None,
        authors: None,
        since: None,
        until: None,
        page: 1,
        per_page: 50,
        mode,
        sort,
    }
}

/// Hit ids of a relevance search, best first.
async fn ranked(fx: &Fixture, community: CommunityId, q: &str, mode: SearchMode) -> Vec<[u8; 32]> {
    fx.service()
        .search(&query(community, q, mode, SearchSort::Relevance))
        .await
        .expect("search")
        .hits
        .iter()
        .map(|hit| hit.event_id)
        .collect()
}

async fn words(fx: &Fixture, community: CommunityId, q: &str) -> Vec<[u8; 32]> {
    ranked(fx, community, q, SearchMode::FullText).await
}

/// Each hit's query match (Q) for a word search.
async fn query_match(fx: &Fixture, community: CommunityId, q: &str) -> Vec<([u8; 32], f32)> {
    fx.service()
        .search_scored(&query(
            community,
            q,
            SearchMode::FullText,
            SearchSort::Relevance,
        ))
        .await
        .expect("debug search")
        .hits
        .iter()
        .map(|hit| (hit.hit.event_id, hit.parts.query_match))
        .collect()
}

fn q_of(scores: &[([u8; 32], f32)], id: &[u8; 32], what: &str) -> f32 {
    scores
        .iter()
        .find(|(hit, _)| hit == id)
        .unwrap_or_else(|| panic!("{what} not found"))
        .1
}

fn position(hits: &[[u8; 32]], id: &[u8; 32], what: &str) -> usize {
    hits.iter()
        .position(|hit| hit == id)
        .unwrap_or_else(|| panic!("{what} not found in {} hits", hits.len()))
}

/// `better` is found and ranks above `worse`, which is found too.
fn ranks_above(hits: &[[u8; 32]], better: &[u8; 32], worse: &[u8; 32], what: &str) {
    let (b, w) = (position(hits, better, what), position(hits, worse, what));
    assert!(b < w, "{what}: expected rank {b} above {w}");
}

/// One- and two-letter typos still find a message, below an exact match;
/// the first letter is never a typo.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn typos_find_the_message_below_an_exact_match() {
    let (pool, fx, c) = setup().await;
    let exact = post(&pool, c, "kubernetes upgrade tonight", DAY).await;
    let misspelt = post(&pool, c, "kubernates upgrade tonight", DAY).await;
    post(&pool, c, "unrelated chatter", DAY).await;

    for typo in ["kubernates", "kubarnates", "kubrenetes"] {
        let hits = words(&fx, c, typo).await;
        assert!(hits.contains(&exact), "{typo} finds the exact spelling");
    }
    let hits = words(&fx, c, "kubernetes").await;
    ranks_above(&hits, &exact, &misspelt, "exact above typo");
    // Kept well apart, not just below: a typo is worth under half a word.
    let scores = query_match(&fx, c, "kubernetes").await;
    let (exact_q, typo_q) = (
        q_of(&scores, &exact, "exact"),
        q_of(&scores, &misspelt, "typo"),
    );
    assert!(typo_q < exact_q * 0.5, "typo Q {typo_q}, exact Q {exact_q}");
    assert!(
        words(&fx, c, "cubernetes").await.is_empty(),
        "the first letter must match"
    );
    fx.teardown(pool).await;
}

/// The message with the rare query word ranks above one repeating common
/// query words.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_rare_word_outranks_repeated_common_words() {
    let (pool, fx, c) = setup().await;
    for n in 0..30 {
        post(&pool, c, &format!("deploy today item {n}"), DAY).await;
    }
    let rare = post(&pool, c, "zephyrine deploy", DAY).await;
    let repeated = post(&pool, c, "deploy today deploy today deploy today", DAY).await;

    let hits = words(&fx, c, "zephyrine deploy today").await;
    ranks_above(&hits, &rare, &repeated, "rare word above repeats");
    assert_eq!(hits.first(), Some(&rare));
    fx.teardown(pool).await;
}

/// An exact phrase ranks above the same words scattered.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn an_exact_phrase_outranks_scattered_words() {
    let (pool, fx, c) = setup().await;
    let scattered = post(&pool, c, "train everyone before the next release", DAY).await;
    let phrase = post(&pool, c, "everyone catch the release train before", DAY).await;

    let hits = words(&fx, c, "release train").await;
    ranks_above(&hits, &phrase, &scattered, "phrase above scattered");
    fx.teardown(pool).await;
}

/// A short message holding the query as a phrase outranks long messages
/// holding its words several times, close together or scattered: the
/// production "ranking rules" case, where long agent output filled the top.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn a_short_exact_phrase_outranks_long_messages_repeating_the_words() {
    let (pool, fx, c) = setup().await;
    let filler = |n: usize| {
        (0..n)
            .map(|i| format!("step{i} output"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let close = post(
        &pool,
        c,
        &format!(
            "{} ranking rules {} ranking rules {} ranking rules {}",
            filler(40),
            filler(40),
            filler(40),
            filler(40)
        ),
        DAY,
    )
    .await;
    let scattered = post(
        &pool,
        c,
        &format!(
            "rules {} ranking {} rules {} ranking {} rules ranking",
            filler(50),
            filler(50),
            filler(50),
            filler(50)
        ),
        DAY,
    )
    .await;
    let short = post(&pool, c, "where are the ranking rules", DAY).await;

    let hits = words(&fx, c, "ranking rules").await;
    assert_eq!(hits.first(), Some(&short), "the short phrase comes first");
    ranks_above(&hits, &short, &close, "short above close repeats");
    ranks_above(&hits, &short, &scattered, "short above scattered repeats");
    fx.teardown(pool).await;
}

/// Words are OR-ed with a minimum: most of the words is enough, one is not.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn most_query_words_are_enough() {
    let (pool, fx, c) = setup().await;
    let most = post(&pool, c, "quarterly budget review notes", DAY).await;
    let one = post(&pool, c, "budget only", DAY).await;

    let hits = words(&fx, c, "quarterly budget review meeting").await;
    assert!(hits.contains(&most), "three of four words match");
    assert!(!hits.contains(&one), "one of four words is not enough");
    fx.teardown(pool).await;
}

/// URLs, hostnames, dotted file names, package names and code identifiers
/// match whole and by part, as do words wrapped in markdown punctuation.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn urls_names_and_markdown_match_whole_and_by_part() {
    let (pool, fx, c) = setup().await;
    let url = post(
        &pool,
        c,
        "read https://docs.example.org/guide/setup first",
        DAY,
    )
    .await;
    let file = post(&pool, c, "edit config.prod.yaml then restart", DAY).await;
    let package = post(&pool, c, "install @example-scope/search-widget now", DAY).await;
    let identifier = post(&pool, c, "call getUserName or buzz_search::run", DAY).await;
    let quoted = post(&pool, c, "ask \"Ingrid\" about it", DAY).await;
    let code = post(&pool, c, "run `frobnicate` twice", DAY).await;
    let bold = post(&pool, c, "this is **crucial** stuff", DAY).await;
    let link = post(&pool, c, "see [handbook](https://example.org/hb)", DAY).await;
    let fenced = post(&pool, c, "```\nwibble --force\n```", DAY).await;
    let italic = post(&pool, c, "an _emphatic_ yes", DAY).await;

    for (q, id) in [
        ("https://docs.example.org/guide/setup", url),
        ("docs.example.org", url),
        ("example", url),
        ("guide", url),
        ("config.prod.yaml", file),
        ("yaml", file),
        ("prod", file),
        ("@example-scope/search-widget", package),
        ("search-widget", package),
        ("widget", package),
        ("getUserName", identifier),
        ("user", identifier),
        ("UserName", identifier),
        ("buzz_search", identifier),
        ("search", identifier),
        ("\"Ingrid\"", quoted),
        ("Ingrid", quoted),
        ("frobnicate", code),
        ("crucial", bold),
        ("handbook", link),
        ("wibble", fenced),
        ("emphatic", italic),
    ] {
        assert!(words(&fx, c, q).await.contains(&id), "{q}");
    }
    fx.teardown(pool).await;
}

/// Accented and unaccented spellings find each other.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn accents_fold() {
    let (pool, fx, c) = setup().await;
    let accented = post(&pool, c, "meet at the Café Zoë", DAY).await;
    let unaccented = post(&pool, c, "meet at the Cafe Zoe", DAY).await;
    let plain = post(&pool, c, "a naive resume", DAY).await;

    // Folded spellings are the same word, not a typo of it.
    let scores = query_match(&fx, c, "cafe zoe").await;
    let (folded, same) = (
        q_of(&scores, &accented, "accented"),
        q_of(&scores, &unaccented, "unaccented"),
    );
    assert!((folded - same).abs() <= same * 1e-3, "{folded} vs {same}");
    assert!(words(&fx, c, "naïve résumé").await.contains(&plain));
    fx.teardown(pool).await;
}

/// A plural finds its singular and back, and the exact form ranks first.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn stems_match_below_the_exact_form() {
    let (pool, fx, c) = setup().await;
    let plural = post(&pool, c, "the queries were slow", DAY).await;
    let singular = post(&pool, c, "the query was slow", DAY).await;

    let hits = words(&fx, c, "queries").await;
    ranks_above(&hits, &plural, &singular, "exact plural first");
    let hits = words(&fx, c, "query").await;
    ranks_above(&hits, &singular, &plural, "exact singular first");
    fx.teardown(pool).await;
}

/// Prefix mode completes the last word only, and long machine tokens such
/// as `nostr:` URIs and hex ids get no prefixes while the message's other
/// words still do.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn prefix_mode_completes_the_last_word_but_not_long_tokens() {
    let (pool, fx, c) = setup().await;
    let planning = post(
        &pool,
        c,
        "project planning milestone transcontinentalrailways",
        DAY,
    )
    .await;
    let archive = post(&pool, c, "project archive", DAY).await;
    let npub = format!("npub1{}", "q".repeat(58));
    let hex = "a3f9b2c4".repeat(8);
    let machine = post(
        &pool,
        c,
        &format!("ping nostr:{npub} about event {hex} and the roadmap"),
        DAY,
    )
    .await;

    let prefix = |q: &'static str| ranked(&fx, c, q, SearchMode::Prefix);
    assert_eq!(prefix("project pla").await, vec![planning]);
    let pro = prefix("pro").await;
    assert!(pro.contains(&planning) && pro.contains(&archive));
    assert!(
        ranked(&fx, c, "pro", SearchMode::FullText).await.is_empty(),
        "full text does not complete words"
    );
    assert_eq!(
        prefix("transcontinentalrailw").await,
        vec![planning],
        "a partial word longer than the longest prefix matches by its start"
    );
    assert_eq!(prefix("roadm").await, vec![machine]);
    let variable = post(
        &pool,
        c,
        "set BUZZ_OPENSEARCH_AWS_SIGV4_SERVICE to aoss",
        DAY,
    )
    .await;
    assert_eq!(
        prefix("set BUZZ_OPENSEARCH_AW").await,
        vec![variable],
        "part of an identifier too long for its own prefixes"
    );
    assert!(prefix("npub1qqq").await.is_empty(), "no prefixes of a key");
    assert!(
        prefix("a3f9b2c4a3").await.is_empty(),
        "no prefixes of an id"
    );
    assert_eq!(
        words(&fx, c, &hex).await,
        vec![machine],
        "the whole id still matches"
    );
    fx.teardown(pool).await;
}

/// Prefix mode ranks every match: a strong match older than the newest 1000
/// still comes first.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn prefix_mode_ranks_a_strong_match_older_than_the_newest_thousand() {
    let (pool, fx, c) = setup().await;
    let old = post(&pool, c, "planning planning planning planning", 3 * DAY).await;
    sqlx::query(
        "WITH rows AS ( \
           SELECT sha256((g || '/' || $1::text)::bytea) AS id, now() - make_interval(secs => g) AS at \
           FROM generate_series(1, 1000) g), \
         stored AS ( \
           INSERT INTO events (community_id, id, pubkey, created_at, kind, tags, content, sig) \
           SELECT $2, id, id, at, 9, '[]'::jsonb, 'planning notes', 'signature' FROM rows) \
         INSERT INTO search_outbox (community_id, event_id, event_created_at) \
         SELECT $2, id, at FROM rows",
    )
    .bind(c.as_uuid().to_string())
    .bind(c.as_uuid())
    .execute(&pool)
    .await
    .expect("store 1000 newer, weaker matches");

    let hits = ranked(&fx, c, "plan", SearchMode::Prefix).await;
    assert_eq!(hits.first(), Some(&old));
    fx.teardown(pool).await;
}

/// Between equal matches the newer ranks first, but an old exact match still
/// outranks recent weak ones.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn recency_breaks_close_calls_only() {
    let (pool, fx, c) = setup().await;
    let older = post(&pool, c, "standup moved to thursday", 40 * DAY).await;
    let newer = post(&pool, c, "standup moved to thursday", 2 * DAY).await;
    let hits = words(&fx, c, "standup thursday").await;
    ranks_above(&hits, &newer, &older, "newer of two equal matches");

    let old_exact = post(&pool, c, "incident rollback checklist", 2 * 365 * DAY).await;
    let filler = (0..60)
        .map(|i| format!("log{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut recent = Vec::new();
    for n in 0..5 {
        recent.push(
            post(
                &pool,
                c,
                &format!("incident {filler} checklist {filler} {n}"),
                n * 60,
            )
            .await,
        );
    }
    let hits = words(&fx, c, "incident rollback checklist").await;
    assert_eq!(hits.first(), Some(&old_exact), "old exact match first");
    assert_eq!(hits.len(), 6, "the weak matches are still found");
    fx.teardown(pool).await;
}

/// Newest sort orders by time alone, whatever the match quality, so it
/// matches no typos: they would crowd out the word that was typed.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn newest_sort_ignores_relevance() {
    let (pool, fx, c) = setup().await;
    let strong = post(&pool, c, "migration migration plan", 3 * DAY).await;
    let typo = post(&pool, c, "migraton notes", DAY).await;
    let weak = post(
        &pool,
        c,
        &format!("{} migration", "word ".repeat(80)),
        2 * DAY,
    )
    .await;

    for mode in [SearchMode::FullText, SearchMode::Prefix] {
        let hits: Vec<_> = fx
            .service()
            .search(&query(c, "migration", mode, SearchSort::Newest))
            .await
            .expect("search")
            .hits
            .iter()
            .map(|hit| hit.event_id)
            .collect();
        assert_eq!(hits, vec![weak, strong], "{mode:?}");
        assert!(!hits.contains(&typo));
    }
    fx.teardown(pool).await;
}

/// Debug mode gives each hit's query match Q and age factor T, with
/// T = 0.2 + 0.8 × 0.5^(age / 30 days) and the hit's rank Q × T.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn debug_mode_returns_query_match_and_age() {
    let (pool, fx, c) = setup().await;
    let ages = [0, 30 * DAY, 60 * DAY, 3 * 365 * DAY];
    let mut ids = Vec::new();
    for age in ages {
        ids.push(post(&pool, c, "telemetry dashboard", age).await);
    }

    let scored = fx
        .service()
        .search_scored(&query(
            c,
            "telemetry dashboard",
            SearchMode::FullText,
            SearchSort::Relevance,
        ))
        .await
        .expect("debug search");
    assert_eq!(scored.hits.len(), ages.len());
    for (id, age) in ids.iter().zip(ages) {
        let hit = scored
            .hits
            .iter()
            .find(|hit| &hit.hit.event_id == id)
            .expect("hit");
        let expected = 0.2 + 0.8 * 0.5_f64.powf(age as f64 / (30 * DAY) as f64);
        assert!(
            (f64::from(hit.parts.age) - expected).abs() < 0.01,
            "age {age}: T {} not {expected}",
            hit.parts.age
        );
        assert!(hit.parts.query_match > 0.0);
        let rank = hit.parts.query_match * hit.parts.age;
        assert!(
            (rank - hit.hit.rank).abs() <= rank * 1e-3,
            "rank {} is Q × T {rank}",
            hit.hit.rank
        );
    }
    let in_order: Vec<_> = scored.hits.iter().map(|hit| hit.hit.event_id).collect();
    assert_eq!(in_order, ids, "equal matches, newest first");
    fx.teardown(pool).await;
}

/// Relevance pages tile one order, so paging through a long list of
/// matches, as the relay's search scan does, returns each message once. The
/// order holds within the rescore window, where relevance results end: past
/// it, hits would keep a rank without the age factor and interleave with
/// the window's.
#[tokio::test]
#[ignore = "requires Postgres and OpenSearch"]
async fn relevance_pages_tile_one_order_up_to_the_rescore_window() {
    let (pool, fx, c) = setup().await;
    for _ in 0..1_100 {
        post(&pool, c, "quarterly report", 365 * DAY).await;
    }
    let page = |page: u32, per_page: u32| SearchQuery {
        page,
        per_page,
        ..query(c, "quarterly", SearchMode::FullText, SearchSort::Relevance)
    };
    let hits = |page: SearchQuery| {
        let fx = &fx;
        async move { fx.service().search(&page).await.expect("search").hits }
    };

    let mut seen = std::collections::HashSet::new();
    for n in 1..=10 {
        let hits = hits(page(n, 100)).await;
        assert_eq!(hits.len(), 100, "page {n} is full");
        for hit in hits {
            assert!(seen.insert(hit.event_id), "page {n} repeats a hit");
        }
    }
    assert!(
        hits(page(11, 100)).await.is_empty(),
        "results end at the window"
    );
    assert_eq!(
        hits(page(4, 300)).await.len(),
        100,
        "a page across it is cut"
    );
    fx.teardown(pool).await;
}
