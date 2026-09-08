//! Regression test for the NIP-43 relay-admin durable-ban bypass
//! (BUZZ-SEC-007 class, reported 2026-07-27).
//!
//! `ingest_event` exempts relay-admin kinds 9030-9033 from its durable
//! write-path restriction gate so a *timed out* admin keeps its administrative
//! capability. That exemption was ban-blind, so a **banned** admin could still
//! add/remove relay members and change the workspace icon via signed NIP-98
//! `POST /events`. The ban is now enforced inside
//! `relay_admin::handle_relay_admin_event`; this test pins both halves of that
//! contract — bans refused, timeouts still admitted.
//!
//! Requires a running relay and its Postgres. Ignored by default:
//!   REPRO_RELAY_HTTP=http://localhost:3999 REPRO_HOST=localhost:3999 \
//!   DATABASE_URL=postgres://buzz:buzz_dev@localhost:5432/buzz_relay_admin_regression \
//!   cargo test -p buzz-test-client --test regression_relay_admin_ban_gate \
//!     -- --ignored --nocapture

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

fn http_base() -> String {
    std::env::var("REPRO_RELAY_HTTP").unwrap_or_else(|_| "http://localhost:3999".into())
}
fn host() -> String {
    std::env::var("REPRO_HOST").unwrap_or_else(|_| "localhost:3999".into())
}
fn db_url() -> String {
    std::env::var("DATABASE_URL").expect("DATABASE_URL required")
}

fn sha256_hex(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

fn nip98(keys: &Keys, url: &str, body: &str) -> String {
    let ev = EventBuilder::new(Kind::Custom(27_235), "")
        .tags(vec![
            Tag::parse(["u", url]).unwrap(),
            Tag::parse(["method", "POST"]).unwrap(),
            Tag::parse(["payload", &sha256_hex(body.as_bytes())]).unwrap(),
            Tag::parse(["nonce", &Uuid::new_v4().to_string()]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    format!(
        "Nostr {}",
        BASE64.encode(serde_json::to_string(&ev).unwrap())
    )
}

async fn post_event(keys: &Keys, event: &nostr::Event) -> (u16, String) {
    let body = serde_json::to_string(event).unwrap();
    let signed_url = format!("http://{}/events", host());
    let r = reqwest::Client::new()
        .post(format!("{}/events", http_base()))
        .header("Host", host())
        .header("Content-Type", "application/json")
        .header("Authorization", nip98(keys, &signed_url, &body))
        .body(body)
        .send()
        .await
        .expect("POST /events");
    let status = r.status().as_u16();
    (status, r.text().await.unwrap_or_default())
}

fn signed(keys: &Keys, kind: u16, tags: Vec<Tag>) -> nostr::Event {
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

async fn pool() -> sqlx::Pool<sqlx::Postgres> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .expect("connect Postgres")
}

async fn community_id(p: &sqlx::Pool<sqlx::Postgres>) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO communities (id, host) VALUES ($1, $2) ON CONFLICT (lower(host)) DO NOTHING",
    )
    .bind(id)
    .bind(host())
    .execute(p)
    .await
    .unwrap();
    sqlx::query_scalar("SELECT id FROM communities WHERE lower(host) = lower($1)")
        .bind(host())
        .fetch_one(p)
        .await
        .unwrap()
}

async fn seed(p: &sqlx::Pool<sqlx::Postgres>, cid: Uuid, keys: &Keys, role: &str) {
    sqlx::query("INSERT INTO users (community_id, pubkey) VALUES ($1, $2) ON CONFLICT DO NOTHING")
        .bind(cid)
        .bind(keys.public_key().to_bytes().to_vec())
        .execute(p)
        .await
        .ok();
    sqlx::query(
        "INSERT INTO relay_members (community_id, pubkey, role, added_by) VALUES ($1,$2,$3,NULL) \
         ON CONFLICT (community_id, pubkey) DO UPDATE SET role = $3, updated_at = now()",
    )
    .bind(cid)
    .bind(keys.public_key().to_hex())
    .bind(role)
    .execute(p)
    .await
    .unwrap();
}

/// Post-fix regression bar.
///
/// Asserts the full contract rather than just "the exploit stopped":
/// - banned admin: 403 + exact `blocked:` prefix on 9030/9031/9033, and a
///   banned *owner* likewise on 9032 (owner-only kind), covering all four
///   exempt kinds,
/// - no roster, role, or icon mutation from any of those attempts,
/// - a *timed-out* admin still reaches relay-admin authorization (the ingest
///   exemption's whole purpose — the fix must not silently widen to timeouts),
/// - an unrestricted admin's behaviour is unchanged, mutation included.
#[tokio::test]
#[ignore]
async fn banned_admin_is_refused_but_timed_out_admin_still_administers() {
    let p = pool().await;
    let cid = community_id(&p).await;

    let owner = Keys::generate();
    let banned_owner = Keys::generate();
    let banned_admin = Keys::generate();
    let timed_out_admin = Keys::generate();
    let good_admin = Keys::generate();
    let victim = Keys::generate();
    let victim2 = Keys::generate();
    let victim3 = Keys::generate();
    let role_target = Keys::generate();
    // Retained (not generated inline) so the 9030 attempt can be checked for
    // absence afterward — a planted member is the mutation that attempt buys.
    let planted = Keys::generate();
    for (k, r) in [
        (&owner, "owner"),
        (&banned_owner, "owner"),
        (&banned_admin, "admin"),
        (&timed_out_admin, "admin"),
        (&good_admin, "admin"),
        (&victim, "member"),
        (&victim2, "member"),
        (&victim3, "member"),
        (&role_target, "member"),
    ] {
        seed(&p, cid, k, r).await;
    }

    // Owner bans one admin and times out another, through the real 9040/9042
    // command path.
    let (s, _) = post_event(
        &owner,
        &signed(
            &owner,
            9040,
            vec![Tag::parse(["p", &banned_admin.public_key().to_hex()]).unwrap()],
        ),
    )
    .await;
    assert_eq!(s, 200, "ban must land");
    let expiry = (chrono_now() + 3600).to_string();
    let (s, b) = post_event(
        &owner,
        &signed(
            &owner,
            9042,
            vec![
                Tag::parse(["p", &timed_out_admin.public_key().to_hex()]).unwrap(),
                Tag::parse(["expiration", &expiry]).unwrap(),
            ],
        ),
    )
    .await;
    assert_eq!(s, 200, "timeout must land: {b}");

    // 9032 is owner-only, so its banned case needs a banned *owner*. Whether
    // one owner may 9040 another is a moderation-policy question independent of
    // this fix, so the ban row is seeded directly to keep the test pinned to
    // the admission gate.
    sqlx::query(
        "INSERT INTO community_bans (community_id, pubkey, banned, actor_pubkey) \
         VALUES ($1,$2,true,$3) \
         ON CONFLICT (community_id, pubkey) DO UPDATE SET banned = true",
    )
    .bind(cid)
    .bind(banned_owner.public_key().to_bytes().to_vec())
    .bind(owner.public_key().to_bytes().to_vec())
    .execute(&p)
    .await
    .unwrap();

    // ── Banned actors: every relay-admin kind must be 403 + `blocked:`. ──
    for (actor, kind, tags, label) in [
        (
            &banned_admin,
            9031u16,
            vec![Tag::parse(["p", &victim.public_key().to_hex()]).unwrap()],
            "9031 remove",
        ),
        (
            &banned_admin,
            9030u16,
            vec![
                Tag::parse(["p", &planted.public_key().to_hex()]).unwrap(),
                Tag::parse(["role", "member"]).unwrap(),
            ],
            "9030 add",
        ),
        (
            &banned_admin,
            9033u16,
            vec![Tag::parse(["icon", "https://evil.example/pwned.png"]).unwrap()],
            "9033 icon",
        ),
        (
            &banned_owner,
            9032u16,
            vec![
                Tag::parse(["p", &role_target.public_key().to_hex()]).unwrap(),
                Tag::parse(["role", "admin"]).unwrap(),
            ],
            "9032 change role",
        ),
    ] {
        let (st, body) = post_event(actor, &signed(actor, kind, tags)).await;
        println!("[banned] {label} -> {st} {body}");
        assert_eq!(
            st, 403,
            "{label}: banned actor must get 403, got {st} {body}"
        );
        let msg: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let text = msg.get("error").and_then(|v| v.as_str()).unwrap_or(&body);
        assert_eq!(
            text, "blocked: you are banned from this community",
            "{label}: must carry the exact `blocked:` wire contract"
        );
    }

    // No mutation from any banned attempt.
    let role_of = |k: &Keys| {
        let hex = k.public_key().to_hex();
        let p = p.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT role FROM relay_members WHERE community_id=$1 AND pubkey=$2",
            )
            .bind(cid)
            .bind(hex)
            .fetch_optional(&p)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        role_of(&victim).await.as_deref(),
        Some("member"),
        "9031: banned admin must not remove a member"
    );
    assert_eq!(
        role_of(&planted).await,
        None,
        "9030: banned admin must not plant a new member"
    );
    assert_eq!(
        role_of(&role_target).await.as_deref(),
        Some("member"),
        "9032: banned owner must not change a member's role"
    );
    let icon: Option<String> = sqlx::query_scalar("SELECT icon FROM communities WHERE id=$1")
        .bind(cid)
        .fetch_one(&p)
        .await
        .unwrap();
    assert!(
        icon.is_none(),
        "9033: banned admin must not change the workspace icon, got {icon:?}"
    );

    // ── Timed-out admin: still administers (ingest exemption preserved). ──
    let (ts, tb) = post_event(
        &timed_out_admin,
        &signed(
            &timed_out_admin,
            9031,
            vec![Tag::parse(["p", &victim2.public_key().to_hex()]).unwrap()],
        ),
    )
    .await;
    println!("[timed-out] 9031 remove -> {ts} {tb}");
    assert_eq!(
        ts, 200,
        "timed-out admin must still administer the roster: {tb}"
    );
    assert_eq!(
        role_of(&victim2).await,
        None,
        "timed-out admin's removal must take effect"
    );

    // Control: the same timed-out admin is still write-blocked for content.
    let (cs, cb) = post_event(
        &timed_out_admin,
        &EventBuilder::new(Kind::Custom(9), "x")
            .tags(vec![Tag::parse(["h", &Uuid::new_v4().to_string()]).unwrap()])
            .sign_with_keys(&timed_out_admin)
            .unwrap(),
    )
    .await;
    println!("[timed-out] control kind:9 -> {cs} {cb}");
    assert_ne!(
        cs, 200,
        "timed-out admin must still be write-blocked for content"
    );

    // ── Unrestricted admin: behaviour unchanged, mutation included. ──
    let (gs, gb) = post_event(
        &good_admin,
        &signed(
            &good_admin,
            9031,
            vec![Tag::parse(["p", &victim3.public_key().to_hex()]).unwrap()],
        ),
    )
    .await;
    println!("[clean] 9031 remove -> {gs} {gb}");
    assert_eq!(gs, 200, "unrestricted admin must be unaffected: {gb}");
    assert_eq!(
        role_of(&victim3).await,
        None,
        "unrestricted admin's removal must actually take effect"
    );

    // ── Unchanged rejection contract: non-admin still gets `invalid:`/400. ──
    let nobody = Keys::generate();
    seed(&p, cid, &nobody, "member").await;
    let (ns, nb) = post_event(
        &nobody,
        &signed(
            &nobody,
            9031,
            vec![Tag::parse(["p", &victim.public_key().to_hex()]).unwrap()],
        ),
    )
    .await;
    println!("[non-admin] 9031 -> {ns} {nb}");
    assert_eq!(
        ns, 400,
        "a plain member's 9031 must stay a 400 validation reject"
    );
    assert!(
        nb.contains("invalid: actor not authorized"),
        "non-admin rejection must keep its `invalid:` prefix, got {nb}"
    );

    println!("\nALL INVARIANTS HELD");
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
