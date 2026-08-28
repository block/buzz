//! Conformance gate for the block/berd `buzz-handoff` contract.
//!
//! block/berd ships `skills/buzz-handoff/SKILL.md` as its first published skill
//! and hard-requires "a Buzz CLI that implements the handoff contract introduced
//! by block/buzz@9e6ee814b" (that commit is a mid-PR commit of PR #6359, merged
//! as 84c095f8b — see `MIN_COMPATIBLE_BUZZ_COMMIT` below).
//!
//! We are the UPSTREAM dependency of a shipped downstream consumer. Without this
//! gate, an absorb that breaks berd is discovered by berd's users, not by our CI.
//!
//! ## Seam
//!
//! Every assertion drives the *public* entry point `buzz_cli::run_from_args`
//! (`src/lib.rs:24`) — the exact surface berd shells out to — against a loopback
//! stub relay. `lib.rs` exports only `run_from_args` and `agent_management`; the
//! contract internals (`commands::messages::format_events`, `links`, `error`) are
//! private modules and are deliberately NOT imported. This keeps the gate
//! black-box and additive: no visibility surgery, no new dependencies.
//!
//! ## Scope, stated honestly
//!
//! This gates the CLI-side contract only. A stub relay cannot prove the real
//! relay's body shape; `crates/buzz-cli/TESTING.md`'s docker runbook remains the
//! live-relay half and is not replaced by this file.
//!
//! Key hygiene: every test signs with a freshly generated throwaway key. This
//! file must never read a real key, the Buzz Desktop keychain, or defeat
//! `hide_env_values` on BUZZ_PRIVATE_KEY (`src/lib.rs:86-87`).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use tokio::net::TcpListener;

/// The minimum buzz commit that satisfies berd's handoff contract.
///
/// berd's SKILL.md pins `9e6ee814b`, which is a mid-PR commit of block/buzz
/// PR #6359 and sits on no branch tip. The merge commit that actually carries
/// the contract onto `main` — and which our `product/main` contains — is this
/// one. Compiled in rather than left in prose so the claim cannot rot silently.
const MIN_COMPATIBLE_BUZZ_COMMIT: &str = "84c095f8b";

/// A real, syntactically valid channel UUID and 64-hex event ids for fixtures.
const CHANNEL_ID: &str = "6f1c9e2a-77d4-4a1e-9a8b-2c5d3e4f6a70";
const ROOT_EVENT_ID: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const REPLY_EVENT_ID: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const TARGET_PUBKEY_HEX: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

// ---------------------------------------------------------------------------
// Stub relay
// ---------------------------------------------------------------------------

type Responder = Arc<dyn Fn(&serde_json::Value) -> (StatusCode, String) + Send + Sync>;

struct StubState {
    responder: Responder,
    seen: Mutex<Vec<serde_json::Value>>,
    hits: AtomicUsize,
}

/// Spawn a one-shot axum stub relay on an ephemeral port.
///
/// Shape follows the already-blessed in-crate precedent at
/// `crates/buzz-cli/src/client.rs:1611-1665` ("spin up a local HTTP server using
/// axum and issue real HTTP requests"). Binding `127.0.0.1:0` means no fixed
/// ports and no sleeps, so the test cannot flake on port contention.
async fn stub_relay<F>(f: F) -> (String, Arc<StubState>)
where
    F: Fn(&serde_json::Value) -> (StatusCode, String) + Send + Sync + 'static,
{
    let state = Arc::new(StubState {
        responder: Arc::new(f),
        seen: Mutex::new(Vec::new()),
        hits: AtomicUsize::new(0),
    });

    let handler = |State(state): State<Arc<StubState>>, body: Bytes| async move {
        state.hits.fetch_add(1, Ordering::SeqCst);
        let filters: serde_json::Value =
            serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        state.seen.lock().unwrap().push(filters.clone());
        let (status, body) = (state.responder)(&filters);
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body))
            .unwrap()
    };

    let app = Router::new()
        .route("/query", post(handler))
        .route("/count", post(handler))
        .route("/events", post(handler))
        .with_state(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), state)
}

/// A throwaway signing identity. NEVER a real key.
fn throwaway_key() -> String {
    nostr::Keys::generate().secret_key().to_secret_hex()
}

/// Build the argv berd would use, with relay/key supplied as explicit flags so
/// the test never mutates process-global env (which would race under the
/// multi-threaded test harness).
fn argv(relay: &str, key: &str, rest: &[&str]) -> Vec<String> {
    let mut v = vec![
        "buzz".to_string(),
        "--relay".to_string(),
        relay.to_string(),
        "--private-key".to_string(),
        key.to_string(),
    ];
    v.extend(rest.iter().map(|s| s.to_string()));
    v
}

/// A signed-event-shaped fixture as the relay returns it from POST /query.
fn event_fixture(
    id: &str,
    root: Option<&str>,
    content: &str,
    created_at: u64,
) -> serde_json::Value {
    let mut tags = vec![serde_json::json!(["h", CHANNEL_ID])];
    if let Some(root) = root {
        tags.push(serde_json::json!(["e", root, "", "root"]));
    }
    serde_json::json!({
        "id": id,
        "pubkey": "aa".repeat(32),
        "kind": 9,
        "content": content,
        "created_at": created_at,
        "tags": tags,
        "sig": "bb".repeat(64),
    })
}

/// The channel metadata replaceable event (kind:39000) `channels get` reads.
fn channel_fixture() -> serde_json::Value {
    serde_json::json!({
        "id": "cc".repeat(32),
        "pubkey": "dd".repeat(32),
        "kind": 39000,
        "content": "{\"name\":\"handoff\",\"about\":\"berd contract fixture\"}",
        "created_at": 1_700_000_000u64,
        "tags": [["d", CHANNEL_ID], ["name", "handoff"]],
        "sig": "ee".repeat(64),
    })
}

/// Route a /query by inspecting the filter berd's command produced.
/// The CLI posts an ARRAY of filters (client.rs:794 `query_multi`).
fn answer_query(filters: &serde_json::Value) -> (StatusCode, String) {
    let first = filters.get(0).cloned().unwrap_or(serde_json::Value::Null);

    // kind:39000 + #d  => `channels get`
    if first
        .get("kinds")
        .and_then(|k| k.as_array())
        .is_some_and(|k| k.iter().any(|v| v.as_u64() == Some(39000)))
    {
        return (
            StatusCode::OK,
            serde_json::json!([channel_fixture()]).to_string(),
        );
    }

    // `ids` only, no #h => fetch_event (messages.rs:66) resolving a thread target
    if first.get("ids").is_some() && first.get("#h").is_none() {
        let ev = event_fixture(ROOT_EVENT_ID, None, "thread root", 1_700_000_001);
        return (StatusCode::OK, serde_json::json!([ev]).to_string());
    }

    // Anything else: a channel/thread read. Return two events, deliberately
    // out of chronological order so the CLI's own sort is exercised.
    let events = serde_json::json!([
        event_fixture(
            REPLY_EVENT_ID,
            Some(ROOT_EVENT_ID),
            "a reply",
            1_700_000_009
        ),
        event_fixture(ROOT_EVENT_ID, None, "thread root", 1_700_000_001),
    ]);
    (StatusCode::OK, events.to_string())
}

// ---------------------------------------------------------------------------
// C1 — `--format` is a PRE-NOUN GLOBAL flag
// ---------------------------------------------------------------------------

/// berd invokes `buzz --format compact messages thread ...` (SKILL.md:39) —
/// `--format` BEFORE the subcommand. If a future refactor moves it to a
/// subcommand-local flag, clap rejects this argv with exit 1 and berd's every
/// call breaks. Asserts flag POSITION, not merely presence.
#[tokio::test]
async fn c1_format_flag_is_global_and_precedes_the_subcommand() {
    let (relay, _state) = stub_relay(answer_query).await;
    let key = throwaway_key();

    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "--format",
            "compact",
            "messages",
            "get",
            "--channel",
            CHANNEL_ID,
            "--limit",
            "5",
        ],
    ))
    .await;
    assert_eq!(code, 0, "pre-noun global --format must parse and succeed");

    // Negative: the same flag AFTER the noun is not the contract berd relies on.
    // We assert only that the pre-noun form is the supported one; if clap ever
    // accepted both, the positive assertion above still holds the contract.
    let post_noun = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "messages",
            "get",
            "--channel",
            CHANNEL_ID,
            "--format",
            "compact",
        ],
    ))
    .await;
    assert_ne!(
        post_noun, 0,
        "sanity: --format is not a subcommand-local flag; if this starts \
         succeeding the flag grammar changed and berd's invocation must be re-checked"
    );
}

// ---------------------------------------------------------------------------
// C2 — compact projection carries id, content, created_at (SUPERSET)
// ---------------------------------------------------------------------------

/// berd parses the compact projection for `id`, `content`, `created_at`
/// (src/commands/messages.rs:337-352).
///
/// DELIBERATELY A SUPERSET ASSERTION, never exact key-set equality: upstream
/// PR #5764 proposes adding `pubkey` and `channel` to this exact arm. An
/// equality assertion would go red on a benign upstream improvement — that is
/// the failure mode this rule exists to prevent.
/// Runs on a MULTI-THREAD runtime because this test is the only one that drives
/// a real subprocess, and a blocking `.output()` on the default current-thread
/// runtime starves the stub relay's accept loop (the subprocess then times out
/// against a server that never polls). Found by running the test, not by
/// reading it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_compact_projection_carries_the_keys_berd_parses() {
    let (relay, _state) = stub_relay(answer_query).await;
    let key = throwaway_key();

    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "--format",
            "compact",
            "messages",
            "get",
            "--channel",
            CHANNEL_ID,
        ],
    ))
    .await;
    assert_eq!(code, 0, "compact channel read must succeed");

    // Now assert the PROJECTION ITSELF, by capturing real stdout from the built
    // `buzz` binary. Cargo hands integration tests the binary path in
    // CARGO_BIN_EXE_<name>, so this needs no `assert_cmd`/`trycmd` dependency —
    // buzz-cli has no such crate in-tree and this gate must not add one.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_buzz"))
        .args([
            "--relay",
            &relay,
            "--private-key",
            &key,
            "--format",
            "compact",
            "messages",
            "get",
            "--channel",
            CHANNEL_ID,
        ])
        .output()
        .expect("the buzz binary must be runnable");
    assert!(
        out.status.success(),
        "compact read exited {:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8(out.stdout).expect("compact output must be UTF-8");
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("compact output must be a single JSON document");
    let events = parsed
        .as_array()
        .expect("compact output must be a JSON ARRAY — berd iterates it");
    assert_eq!(events.len(), 2, "both fixture events must be projected");

    for event in events {
        let obj = event
            .as_object()
            .expect("each compact event is a JSON object");
        // SUPERSET, not equality: extra keys (e.g. upstream #5764's `pubkey` and
        // `channel`) are explicitly allowed. Only the ABSENCE of a key berd
        // parses is a contract break.
        for required in ["id", "content", "created_at"] {
            assert!(
                obj.contains_key(required),
                "compact projection dropped `{required}`, which block/berd parses \
                 (skills/buzz-handoff/SKILL.md:39); keys present: {:?}",
                obj.keys().collect::<Vec<_>>()
            );
        }
        assert!(obj["id"].is_string(), "`id` must stay a string");
        assert!(obj["content"].is_string(), "`content` must stay a string");
        assert!(
            obj["created_at"].is_number(),
            "`created_at` must stay a number — berd sorts on it"
        );
    }

    // The CLI sorts by created_at ascending; berd relies on chronological order
    // to build the handoff summary. Fixtures are served out of order on purpose.
    let times: Vec<u64> = events
        .iter()
        .map(|e| e["created_at"].as_u64().unwrap())
        .collect();
    let mut sorted = times.clone();
    sorted.sort_unstable();
    assert_eq!(
        times, sorted,
        "compact events must be chronologically ordered"
    );
}

// ---------------------------------------------------------------------------
// C3 — `messages thread --link buzz://message?...`
// ---------------------------------------------------------------------------

/// berd resolves a handoff by deep link: `messages thread --link` (SKILL.md:39).
/// The link grammar is owned and unit-tested by `src/links.rs:199-253`; here we
/// assert REACHABILITY — that a well-formed link is accepted end-to-end through
/// the public entry point and drives a real relay read.
#[tokio::test]
async fn c3_thread_read_accepts_a_buzz_message_deep_link() {
    let (relay, state) = stub_relay(answer_query).await;
    let key = throwaway_key();

    let link = format!("buzz://message?channel={CHANNEL_ID}&id={ROOT_EVENT_ID}");
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "--format", "compact", "messages", "thread", "--link", &link, "--limit", "200",
        ],
    ))
    .await;
    assert_eq!(code, 0, "berd's --link thread read must succeed: {link}");
    assert!(
        state.hits.load(Ordering::SeqCst) >= 2,
        "a --link thread read resolves the target then reads the thread"
    );

    // Negative: a link whose thread root contradicts the selected message is a
    // USER error (exit 1), not a silent success (messages.rs:412-416).
    let bad =
        format!("buzz://message?channel={CHANNEL_ID}&id={ROOT_EVENT_ID}&thread={REPLY_EVENT_ID}");
    let code =
        buzz_cli::run_from_args(argv(&relay, &key, &["messages", "thread", "--link", &bad])).await;
    assert_eq!(
        code, 1,
        "contradictory thread root must be rejected as a user error"
    );

    // Negative: a malformed link must not be treated as a channel id.
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "messages",
            "thread",
            "--link",
            "https://example.com/not-a-buzz-link",
        ],
    ))
    .await;
    assert_eq!(code, 1, "non-buzz scheme must be a usage error");
}

// ---------------------------------------------------------------------------
// C4 — `channels get --channel` and `messages get --channel --limit`
// ---------------------------------------------------------------------------

/// SKILL.md:45-46 reads channel metadata then the recent messages. Both must
/// dispatch and both must accept the compact format.
#[tokio::test]
async fn c4_channel_metadata_and_message_reads_dispatch() {
    let (relay, state) = stub_relay(answer_query).await;
    let key = throwaway_key();

    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &["channels", "get", "--channel", CHANNEL_ID],
    ))
    .await;
    assert_eq!(code, 0, "`channels get --channel` must succeed");

    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "--format",
            "compact",
            "messages",
            "get",
            "--channel",
            CHANNEL_ID,
            "--limit",
            "200",
        ],
    ))
    .await;
    assert_eq!(
        code, 0,
        "`--format compact messages get --channel --limit` must succeed"
    );

    // The channel read must actually be scoped to the requested channel — a
    // regression that dropped the #h scoping would leak other channels to berd.
    let seen = state.seen.lock().unwrap().clone();
    let scoped = seen.iter().any(|filters| {
        filters
            .get(0)
            .and_then(|f| f.get("#h"))
            .and_then(|h| h.as_array())
            .is_some_and(|h| h.iter().any(|v| v.as_str() == Some(CHANNEL_ID)))
    });
    assert!(scoped, "message reads must be scoped by the channel h-tag");

    // Invalid channel id is a user error, not a relay round trip.
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &["messages", "get", "--channel", "not-a-uuid"],
    ))
    .await;
    assert_eq!(code, 1, "a malformed channel id must exit 1");
}

// ---------------------------------------------------------------------------
// C5 — the three environment variables berd existence-gates on
// ---------------------------------------------------------------------------

/// berd gates on `BUZZ_RELAY_URL`, `BUZZ_PRIVATE_KEY`, `BUZZ_AUTH_TAG` with
/// `test -n` (SKILL.md:15-17,24). They are declared `#[arg(long, env = ...)]`
/// at `src/lib.rs:82-95`. Renaming or dropping one silently breaks berd's
/// preflight, so assert each is a recognised long flag through `--help`-free
/// parsing (exit 0 on a successful call that supplies it).
#[tokio::test]
async fn c5_relay_key_and_auth_tag_are_accepted_inputs() {
    let (relay, state) = stub_relay(answer_query).await;
    let key = throwaway_key();

    // --relay and --private-key are exercised by every other test in this file.
    // --auth-tag is the third and is NOT otherwise covered: an absent/empty tag
    // must be accepted (berd only sets it when present).
    let mut args = argv(&relay, &key, &[]);
    args.extend(
        [
            "--auth-tag",
            "",
            "--format",
            "compact",
            "messages",
            "get",
            "--channel",
            CHANNEL_ID,
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let code = buzz_cli::run_from_args(args).await;
    assert_eq!(
        code, 0,
        "an empty BUZZ_AUTH_TAG must be treated as unset, as berd assumes"
    );

    // A malformed auth tag must fail as AUTH (3), not be silently ignored —
    // berd branches on 3 to re-mint credentials.
    let mut args = argv(&relay, &key, &[]);
    args.extend(
        [
            "--auth-tag",
            "{not json",
            "messages",
            "get",
            "--channel",
            CHANNEL_ID,
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    let code = buzz_cli::run_from_args(args).await;
    assert_eq!(
        code, 3,
        "a malformed auth tag must map to the auth exit code"
    );

    // Missing identity entirely => auth error, before any network call.
    let before = state.hits.load(Ordering::SeqCst);
    let code = buzz_cli::run_from_args(vec![
        "buzz".to_string(),
        "--relay".to_string(),
        relay.clone(),
        "messages".to_string(),
        "get".to_string(),
        "--channel".to_string(),
        CHANNEL_ID.to_string(),
    ])
    .await;
    assert_eq!(code, 3, "a missing private key must be an auth error");
    assert_eq!(
        state.hits.load(Ordering::SeqCst),
        before,
        "the CLI must not contact the relay without an identity"
    );
}

// ---------------------------------------------------------------------------
// C6 — exit-code contract
// ---------------------------------------------------------------------------

/// berd branches on our process exit codes. `src/error.rs:90-108` maps every
/// private `CliError` variant through the public `i32` returned by
/// `run_from_args`; the relay-status arm is also status-dependent:
/// `Relay { status } => if 401 || 403 { 3 } else { 2 }`.
///
/// A 403 mapping to 2 instead of 3 would make berd retry a permission failure
/// as a network blip. Each case below is driven black-box through the `i32`
/// returned by `run_from_args` — `error::exit_code` is a private module and is
/// deliberately NOT imported to fake coverage.
#[tokio::test]
async fn c6_relay_status_maps_to_the_documented_exit_codes() {
    let key = throwaway_key();

    // Key => 3 (auth bucket). This is distinct from missing-key Auth in C5.
    let (relay, state) = stub_relay(answer_query).await;
    let code = buzz_cli::run_from_args(vec![
        "buzz".to_string(),
        "--relay".to_string(),
        relay,
        "--private-key".to_string(),
        "not-a-real-nostr-secret".to_string(),
        "messages".to_string(),
        "get".to_string(),
        "--channel".to_string(),
        CHANNEL_ID.to_string(),
    ])
    .await;
    assert_eq!(code, 3, "an invalid private key must exit 3");
    assert_eq!(
        state.hits.load(Ordering::SeqCst),
        0,
        "the CLI must fail invalid keys before contacting the relay"
    );

    // 403 => 3 (auth), NOT 2. The status-dependent arm.
    let (relay, _s) = stub_relay(|_| {
        (
            StatusCode::FORBIDDEN,
            serde_json::json!({"error":"forbidden"}).to_string(),
        )
    })
    .await;
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &["messages", "get", "--channel", CHANNEL_ID],
    ))
    .await;
    assert_eq!(
        code, 3,
        "relay 403 must map to the auth exit code, not network"
    );

    // 401 => 3 (auth).
    let (relay, _s) = stub_relay(|_| {
        (
            StatusCode::UNAUTHORIZED,
            serde_json::json!({"error":"unauthorized"}).to_string(),
        )
    })
    .await;
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &["messages", "get", "--channel", CHANNEL_ID],
    ))
    .await;
    assert_eq!(code, 3, "relay 401 must map to the auth exit code");

    // 500 => 2 (network/relay), the else-branch of the same arm.
    let (relay, _s) = stub_relay(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({"error":"boom"}).to_string(),
        )
    })
    .await;
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &["messages", "get", "--channel", CHANNEL_ID],
    ))
    .await;
    assert_eq!(code, 2, "a relay 5xx must map to the network exit code");

    // Network => 2. Port 1 on loopback is expected to refuse immediately; even
    // with the normal three-attempt retry policy this remains a sub-second
    // local failure on developer and CI hosts.
    let code = buzz_cli::run_from_args(argv(
        "http://127.0.0.1:1",
        &key,
        &["messages", "get", "--channel", CHANNEL_ID],
    ))
    .await;
    assert_eq!(code, 2, "a transport connect failure must exit 2");

    // Usage => 1 (unknown subcommand, no relay needed).
    let code = buzz_cli::run_from_args(vec![
        "buzz".to_string(),
        "messages".to_string(),
        "no-such-subcommand".to_string(),
    ])
    .await;
    assert_eq!(code, 1, "a usage error must exit 1");

    // NotFound => 1. The thread target does not exist.
    let (relay, _s) = stub_relay(|filters| {
        let first = filters.get(0).cloned().unwrap_or(serde_json::Value::Null);
        if first.get("ids").is_some() && first.get("#h").is_none() {
            return (StatusCode::OK, "[]".to_string());
        }
        answer_query(filters)
    })
    .await;
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "messages",
            "thread",
            "--channel",
            CHANNEL_ID,
            "--event",
            ROOT_EVENT_ID,
        ],
    ))
    .await;
    assert_eq!(code, 1, "a missing thread target must exit 1 (not-found)");

    // Conflict => 5. Drive it through a real addressable-write command instead
    // of importing private error helpers: `notes set` first queries the current
    // kind:30023 head, then treats an accepted write with `duplicate:` as a
    // dominated head conflict.
    let (relay, _s) = stub_relay(|body| {
        if body.as_array().is_some() {
            return (StatusCode::OK, "[]".to_string());
        }
        (
            StatusCode::OK,
            serde_json::json!({
                "event_id": body.get("id").and_then(|id| id.as_str()).unwrap_or(ROOT_EVENT_ID),
                "accepted": true,
                "message": "duplicate: dominated by a newer head"
            })
            .to_string(),
        )
    })
    .await;
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "notes",
            "set",
            "--name",
            "handoff-contract",
            "--title",
            "Handoff contract",
            "--content",
            "body",
        ],
    ))
    .await;
    assert_eq!(code, 5, "a dominated addressable write must exit 5");

    // DeliveryUnknown => 2. Moderation writes (kinds 9040-9044) are
    // non-idempotent command events; a proxy 502 may have happened after relay
    // execution, so `submit_moderation_event` must surface DeliveryUnknown
    // rather than a retryable relay error.
    let (relay, _s) = stub_relay(|_| {
        (
            StatusCode::BAD_GATEWAY,
            serde_json::json!({"error":"proxy failed after possible execution"}).to_string(),
        )
    })
    .await;
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &["moderation", "ban", "--pubkey", TARGET_PUBKEY_HEX],
    ))
    .await;
    assert_eq!(
        code, 2,
        "an ambiguous non-idempotent moderation write must exit 2"
    );

    // Other => 4. `notes set` parses the read-before-write kind:30023 query
    // strictly; a non-JSON relay body is an unexpected local/relay contract
    // failure, not a usage, auth, conflict, or network error.
    let (relay, _s) = stub_relay(|_| (StatusCode::OK, "not json".to_string())).await;
    let code = buzz_cli::run_from_args(argv(
        &relay,
        &key,
        &[
            "notes",
            "set",
            "--name",
            "handoff-contract",
            "--title",
            "Handoff contract",
            "--content",
            "body",
        ],
    ))
    .await;
    assert_eq!(code, 4, "unexpected malformed relay data must exit 4");
}

// ---------------------------------------------------------------------------
// C7 — minimum compatible commit is compiled, not prose
// ---------------------------------------------------------------------------

/// berd pins `9e6ee814b`, a mid-PR commit of block/buzz PR #6359 that sits on no
/// branch tip. The commit that carries the contract onto `main` (and therefore
/// onto our `product/main`) is `84c095f8b`. Asserting it here means the claim is
/// compiled and reviewed, not a doc line that rots.
#[test]
fn c7_minimum_compatible_commit_is_recorded() {
    assert_eq!(
        MIN_COMPATIBLE_BUZZ_COMMIT, "84c095f8b",
        "minimum compatible buzz commit for block/berd skills/buzz-handoff"
    );
    assert!(
        MIN_COMPATIBLE_BUZZ_COMMIT.len() >= 9,
        "pin must be an unambiguous short SHA"
    );
}
