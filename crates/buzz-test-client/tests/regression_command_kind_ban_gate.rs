//! Regression test for the command-kind durable-ban/timeout bypass.
//!
//! `ingest_event` routes command kinds (DM open/add/hide, workflow def/trigger,
//! approval grant/deny) straight to `command_executor::handle_command` and
//! returns *before* the durable ban/timeout write-block gate. Unlike the
//! moderation-command and relay-admin exemptions — which each re-check the ban
//! inside their own handler — `handle_command` performs no restriction check,
//! so a banned or timed-out member could still open DMs, define/trigger
//! workflows, and grant/deny approvals over signed NIP-98 `POST /events`.
//!
//! The gate now covers command kinds; this test pins the contract:
//!   - a **banned** member's command kind is refused 403 + `blocked:`,
//!   - a **timed-out** member's command kind is refused 403 + `restricted:`
//!     (command kinds are ordinary writes, not admin capability, so a timeout
//!     blocks them — the relay-admin exemption does NOT extend here),
//!   - an **unrestricted** member's command kind still succeeds and mutates.
//!
//! Requires a running relay and its Postgres. Ignored by default:
//!   REPRO_RELAY_HTTP=http://localhost:3030 REPRO_HOST=localhost:3030 \
//!   DATABASE_URL=postgres://buzz:buzz_dev@localhost:5471/buzz \
//!   cargo test -p buzz-test-client --test regression_command_kind_ban_gate \
//!     -- --ignored --nocapture

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use nostr::{EventBuilder, Keys, Kind, Tag};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const KIND_DM_OPEN: u16 = 41010;

fn http_base() -> String {
    std::env::var("REPRO_RELAY_HTTP").unwrap_or_else(|_| "http://localhost:3030".into())
}
fn host() -> String {
    std::env::var("REPRO_HOST").unwrap_or_else(|_| "localhost:3030".into())
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

/// A DM-open command from `actor` to `peer` (a single `p` tag is the minimum
/// `handle_dm_open` accepts).
fn dm_open(actor: &Keys, peer: &Keys) -> nostr::Event {
    signed(
        actor,
        KIND_DM_OPEN,
        vec![Tag::parse(["p", &peer.public_key().to_hex()]).unwrap()],
    )
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

/// Count active DM (channel) memberships a pubkey holds — the observable
/// mutation `handle_dm_open` produces. Zero means no DM was created for them.
async fn channel_membership_count(p: &sqlx::Pool<sqlx::Postgres>, cid: Uuid, keys: &Keys) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM channel_members WHERE community_id = $1 AND pubkey = $2",
    )
    .bind(cid)
    .bind(keys.public_key().to_bytes().to_vec())
    .fetch_one(p)
    .await
    .unwrap()
}

/// Post-fix regression bar for command kinds.
///
/// Pre-fix this FAILS at the first assertion: the banned member's `DM_OPEN` is
/// accepted (200) instead of refused (403) — the reproduction. Post-fix all
/// three actors behave per the durable write-block contract.
#[tokio::test]
#[ignore]
async fn banned_or_timed_out_member_cannot_run_a_command_kind() {
    let p = pool().await;
    let cid = community_id(&p).await;

    let owner = Keys::generate();
    let banned = Keys::generate();
    let timed_out = Keys::generate();
    let good = Keys::generate();
    for (k, r) in [
        (&owner, "owner"),
        (&banned, "member"),
        (&timed_out, "member"),
        (&good, "member"),
    ] {
        seed(&p, cid, k, r).await;
    }

    // Owner bans one member and times out another via the real 9040/9042 path.
    let (s, b) = post_event(
        &owner,
        &signed(
            &owner,
            9040,
            vec![Tag::parse(["p", &banned.public_key().to_hex()]).unwrap()],
        ),
    )
    .await;
    assert_eq!(s, 200, "ban must land: {b}");
    let expiry = (chrono_now() + 3600).to_string();
    let (s, b) = post_event(
        &owner,
        &signed(
            &owner,
            9042,
            vec![
                Tag::parse(["p", &timed_out.public_key().to_hex()]).unwrap(),
                Tag::parse(["expiration", &expiry]).unwrap(),
            ],
        ),
    )
    .await;
    assert_eq!(s, 200, "timeout must land: {b}");

    // Snapshot DM memberships each actor holds AFTER moderation — a ban/timeout
    // itself sends the target a notice DM — so the mutation check below measures
    // only what each actor's own DM_OPEN attempt adds.
    let before_banned = channel_membership_count(&p, cid, &banned).await;
    let before_timed_out = channel_membership_count(&p, cid, &timed_out).await;
    let before_good = channel_membership_count(&p, cid, &good).await;

    // ── Banned member: DM_OPEN must be 403 + `blocked:`. ──
    let (bs, bb) = post_event(&banned, &dm_open(&banned, &owner)).await;
    println!("[banned] DM_OPEN -> {bs} {bb}");
    assert_eq!(
        bs, 403,
        "banned member's command kind must get 403, got {bs} {bb}"
    );
    let msg: serde_json::Value = serde_json::from_str(&bb).unwrap_or_default();
    let text = msg.get("error").and_then(|v| v.as_str()).unwrap_or(&bb);
    assert_eq!(
        text, "blocked: you are banned from this community",
        "banned member must carry the exact `blocked:` wire contract"
    );

    // ── Timed-out member: DM_OPEN must be 403 + `restricted:`. Command kinds are
    // ordinary writes, not admin capability, so a timeout blocks them. ──
    let (ts, tb) = post_event(&timed_out, &dm_open(&timed_out, &owner)).await;
    println!("[timed-out] DM_OPEN -> {ts} {tb}");
    assert_eq!(
        ts, 403,
        "timed-out member's command kind must get 403, got {ts} {tb}"
    );
    let tmsg: serde_json::Value = serde_json::from_str(&tb).unwrap_or_default();
    let ttext = tmsg.get("error").and_then(|v| v.as_str()).unwrap_or(&tb);
    assert!(
        ttext.starts_with("restricted: you are timed out until"),
        "timed-out member must carry the `restricted:` wire contract, got {ttext:?}"
    );

    // ── Unrestricted member: DM_OPEN still succeeds and mutates. ──
    let (gs, gb) = post_event(&good, &dm_open(&good, &owner)).await;
    println!("[clean] DM_OPEN -> {gs} {gb}");
    assert_eq!(gs, 200, "unrestricted member must be unaffected: {gb}");

    // ── Mutation defense-in-depth: the DM_OPEN attempt creates a DM only for
    // the unrestricted member. Absolute counts include the ban/timeout notice
    // DMs, so compare against the pre-attempt snapshots. ──
    assert_eq!(
        channel_membership_count(&p, cid, &banned).await,
        before_banned,
        "banned member's DM_OPEN must create no DM"
    );
    assert_eq!(
        channel_membership_count(&p, cid, &timed_out).await,
        before_timed_out,
        "timed-out member's DM_OPEN must create no DM"
    );
    assert!(
        channel_membership_count(&p, cid, &good).await > before_good,
        "unrestricted member's DM must actually be created"
    );

    println!("\nALL INVARIANTS HELD");
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
