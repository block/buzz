#![deny(unsafe_code)]

//! Buzz instance administration CLI.
//!
//! # Member management (NIP-43)
//!
//! ## Why only kind:13534 (membership list), not kind:8000/8001 (deltas)
//!
//! CLI intentionally does not emit kind 8000/8001 deltas —
//! `publish_nip43_delta` is in-process-only (no Redis hop), so a sidecar call
//! stores but never pushes. The 13534 list snapshot is the authoritative roster
//! and rides Redis to live clients. Do not wire a delta call that passes
//! in-process tests and silently no-ops in the deployed `compose exec` path.
//!
//! ## Same-second domination guard
//!
//! The `custom_created_at = max(now, newest_existing_13534 + 1s)` bump defeats
//! same-second domination for serial invocations; it does NOT serialize
//! concurrent CLI processes — two near-simultaneous adds can read the same
//! newest timestamp and collide on the bumped second. run.sh serialization is
//! the guard against parallel adds (e.g. `xargs -P`).

use std::sync::Arc;

use anyhow::Result;
use buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST;
use buzz_core::tenant::{relay_url_authority, TenantContext};
use buzz_db::{Db, DbConfig};
use buzz_pubsub::{EventTopic, PubSubManager};
use clap::{Parser, Subcommand};
use nostr::{EventBuilder, Keys, Kind, Tag};
use tracing::warn;

#[derive(Parser)]
#[command(name = "buzz-admin", about = "Buzz instance administration")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a pubkey to the relay membership list.
    ///
    /// Accepts a bech32 npub or 64-char hex pubkey. After inserting the DB row,
    /// publishes a kind:13534 membership roster via Redis so live clients see
    /// the updated list immediately.
    AddMember {
        /// Nostr public key — bech32 npub or 64-char hex.
        #[arg(long)]
        pubkey: String,

        /// Role: "admin" or "member" (default: member). Cannot be "owner" —
        /// use RELAY_OWNER_PUBKEY config to set the relay owner.
        #[arg(long, default_value = "member")]
        role: String,
    },
    /// Remove a pubkey from the relay membership list.
    ///
    /// Accepts a bech32 npub or 64-char hex pubkey. After removing the DB row,
    /// publishes a kind:13534 membership roster via Redis. Cannot remove the
    /// relay owner — change RELAY_OWNER_PUBKEY config instead.
    RemoveMember {
        /// Nostr public key — bech32 npub or 64-char hex.
        #[arg(long)]
        pubkey: String,

        /// Only remove if the member's current role matches this value.
        /// Omit to remove regardless of role.
        #[arg(long)]
        role: Option<String>,
    },
    /// List all relay members.
    ListMembers,
    /// Generate a new Nostr keypair (for bootstrapping).
    GenerateKey,
    /// Run pending database migrations.
    Migrate,
    /// Inspect deployment-wide Buzz product feedback.
    ProductFeedback {
        #[command(subcommand)]
        command: ProductFeedbackCommand,
    },
    /// Emit kind:39000/39002 events for channels missing them.
    ///
    /// Channels created via direct SQL (seed scripts, pre-migration data) won't
    /// have Nostr discovery events. This command creates them so pure-nostr
    /// clients can see those channels. Idempotent — safe to run multiple times.
    ReconcileChannels {
        /// Relay private key (hex) for signing events. Falls back to
        /// BUZZ_RELAY_PRIVATE_KEY env var. If neither is set, generates
        /// an ephemeral key (events will be unverifiable after restart).
        #[arg(long)]
        relay_key: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProductFeedbackCommand {
    /// List feedback across every community as JSON.
    List {
        /// Maximum records to return.
        #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
    },
}

#[tokio::main]
async fn main() {
    // Install the ring CryptoProvider for rustls. The workspace redis TLS
    // feature compiles both aws-lc-rs and ring in transitively, so rustls can't
    // auto-select a provider and would panic on the first rediss:// (ElastiCache)
    // Redis TLS connection without this. Mirrors buzz-relay's main().
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let cli = Cli::parse();

    let code = match run(cli).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            5
        }
    };
    std::process::exit(code);
}

async fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::GenerateKey => {
            let keys = Keys::generate();
            println!("Public key:  {}", keys.public_key().to_hex());
            println!("Secret key:  {}", keys.secret_key().display_secret());
            println!("\nSet BUZZ_PRIVATE_KEY to the secret key to use this identity.");
            Ok(0)
        }
        Command::Migrate => {
            let db = connect_db().await?;
            db.migrate().await?;
            println!("Database migrations complete.");
            Ok(0)
        }
        Command::AddMember { pubkey, role } => cmd_add_member(pubkey, role).await,
        Command::RemoveMember { pubkey, role } => cmd_remove_member(pubkey, role).await,
        Command::ListMembers => cmd_list_members().await,
        Command::ProductFeedback {
            command: ProductFeedbackCommand::List { limit },
        } => cmd_list_product_feedback(limit).await,
        Command::ReconcileChannels { relay_key } => {
            reconcile_channels(relay_key).await?;
            Ok(0)
        }
    }
}

async fn cmd_add_member(pubkey_arg: String, role: String) -> Result<i32> {
    if let Err(msg) = validate_role(&role) {
        eprintln!("error: {msg}");
        return Ok(1);
    }

    let pubkey_hex = match parse_pubkey_hex(&pubkey_arg) {
        Ok(h) => h,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(1);
        }
    };

    let (db, pubsub, relay_keypair) = connect_member_services().await?;

    let tenant = resolve_admin_tenant(&db).await?;
    match db
        .add_relay_member(tenant.community(), &pubkey_hex, &role, None)
        .await
    {
        Ok(true) => println!("added {pubkey_hex} as {role}"),
        Ok(false) => println!("already a member: {pubkey_hex} (no change)"),
        Err(e) => {
            eprintln!("error: DB write failed: {e}");
            return Ok(5);
        }
    }

    if let Err(e) = publish_membership_list_with_bump(&db, &pubsub, &relay_keypair, &tenant).await {
        eprintln!("warning: member added to DB but list publish failed: {e}");
    }

    Ok(0)
}

async fn cmd_remove_member(pubkey_arg: String, role_filter: Option<String>) -> Result<i32> {
    if let Some(ref role) = role_filter {
        if let Err(msg) = validate_role(role) {
            eprintln!("error: {msg}");
            return Ok(1);
        }
    }

    let pubkey_hex = match parse_pubkey_hex(&pubkey_arg) {
        Ok(h) => h,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(1);
        }
    };

    let (db, pubsub, relay_keypair) = connect_member_services().await?;

    let tenant = resolve_admin_tenant(&db).await?;
    use buzz_db::relay_members::RemoveResult;
    let result = if let Some(ref role) = role_filter {
        db.remove_relay_member_if_role(tenant.community(), &pubkey_hex, role)
            .await
    } else {
        db.remove_relay_member(tenant.community(), &pubkey_hex)
            .await
    };

    match result {
        Ok(RemoveResult::Removed) => println!("removed {pubkey_hex}"),
        Ok(RemoveResult::NotFound) => {
            eprintln!("error: member not found: {pubkey_hex}");
            return Ok(2);
        }
        Ok(RemoveResult::IsOwner) => {
            eprintln!(
                "error: cannot remove relay owner: {pubkey_hex}\n\
                 To change the owner, update RELAY_OWNER_PUBKEY and restart."
            );
            return Ok(3);
        }
        Ok(RemoveResult::RoleMismatch) => {
            let role_str = role_filter.as_deref().unwrap_or("(unknown)");
            eprintln!("error: role mismatch — {pubkey_hex} is not currently '{role_str}'");
            return Ok(4);
        }
        Err(e) => {
            eprintln!("error: DB write failed: {e}");
            return Ok(5);
        }
    }

    if let Err(e) = publish_membership_list_with_bump(&db, &pubsub, &relay_keypair, &tenant).await {
        eprintln!("warning: member removed from DB but list publish failed: {e}");
    }

    Ok(0)
}

async fn cmd_list_product_feedback(limit: u16) -> Result<i32> {
    let db = connect_db().await?;
    let feedback = db.list_product_feedback(i64::from(limit)).await?;
    println!("{}", serde_json::to_string_pretty(&feedback)?);
    Ok(0)
}

async fn cmd_list_members() -> Result<i32> {
    let db = connect_db().await?;
    let tenant = resolve_admin_tenant(&db).await?;
    let members = db.list_relay_members(tenant.community()).await?;

    if members.is_empty() {
        println!("(no relay members)");
        return Ok(0);
    }

    println!(
        "{:<66} {:<8} {:<66} created_at",
        "pubkey", "role", "added_by"
    );
    println!("{}", "-".repeat(160));
    for m in &members {
        let added_by = m.added_by.as_deref().unwrap_or("-");
        println!(
            "{:<66} {:<8} {:<66} {}",
            m.pubkey,
            m.role,
            added_by,
            m.created_at.format("%Y-%m-%dT%H:%M:%SZ")
        );
    }

    Ok(0)
}

/// Validate that `role` is `"member"` or `"admin"`. Rejects `"owner"`.
fn validate_role(role: &str) -> std::result::Result<(), String> {
    match role {
        "member" | "admin" => Ok(()),
        "owner" => {
            Err("role 'owner' cannot be set via CLI — use RELAY_OWNER_PUBKEY config".to_string())
        }
        other => Err(format!(
            "invalid role '{other}': must be 'member' or 'admin'"
        )),
    }
}

/// Parse a bech32 npub or 64-char hex pubkey into lowercase hex.
fn parse_pubkey_hex(input: &str) -> std::result::Result<String, String> {
    nostr::PublicKey::parse(input)
        .map(|pk| pk.to_hex())
        .map_err(|e| format!("invalid pubkey '{input}': {e}"))
}

/// Compute the `created_at` for a new kind:13534 snapshot:
/// `max(now, newest_existing + 1s)`.
///
/// Defeats same-second domination for serial invocations (see module doc);
/// also monotonic under backwards clock skew (`newest_existing > now`).
fn bumped_created_at(now: u64, newest_existing: Option<u64>) -> u64 {
    match newest_existing {
        Some(existing) => (existing + 1).max(now),
        None => now,
    }
}

/// Publish kind:13534 with `custom_created_at = max(now, newest_existing + 1s)`.
///
/// Guarantees the new event is not dominated by a same-second prior invocation,
/// so `replace_addressable_event` always inserts and dispatches to Redis.
///
/// See module-level doc for the TOCTOU caveat on concurrent CLI processes.
async fn publish_membership_list_with_bump(
    db: &Db,
    pubsub: &Arc<PubSubManager>,
    relay_keypair: &Keys,
    tenant: &TenantContext,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let relay_pubkey = relay_keypair.public_key();
    let relay_pubkey_bytes = relay_pubkey.to_bytes();

    // Query the newest existing kind:13534 for this relay's pubkey (channel_id=None).
    let newest_ts = db
        .get_latest_global_replaceable(
            tenant.community(),
            KIND_NIP43_MEMBERSHIP_LIST as i32,
            &relay_pubkey_bytes,
        )
        .await?
        .map(|e| e.event.created_at.as_secs());

    // custom_created_at = max(now, existing + 1s) — defeats same-second domination.
    let ts = bumped_created_at(now, newest_ts);

    let members = db.list_relay_members(tenant.community()).await?;

    let mut tags: Vec<Tag> = Vec::with_capacity(members.len() + 1);
    // NIP-70 protected-event marker — prevents re-broadcasting by third parties.
    tags.push(Tag::parse(["-"]).map_err(|e| anyhow::anyhow!("failed to build '-' tag: {e}"))?);
    for member in &members {
        tags.push(
            Tag::parse(["member", &member.pubkey, &member.role])
                .map_err(|e| anyhow::anyhow!("failed to build member tag: {e}"))?,
        );
    }

    let event = EventBuilder::new(Kind::Custom(KIND_NIP43_MEMBERSHIP_LIST as u16), "")
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(ts))
        .sign_with_keys(relay_keypair)
        .map_err(|e| anyhow::anyhow!("failed to sign kind:13534: {e}"))?;

    let (stored, was_inserted) = db
        .replace_addressable_event(tenant.community(), &event, None)
        .await?;
    if was_inserted {
        // Publish to Redis so live clients receive the updated roster.
        // Community-global scope (EventTopic::Global) matches the relay's own
        // membership-list publish path; the tenant fixes the community.
        if let Err(e) = pubsub
            .publish_event(tenant, EventTopic::Global, &stored.event)
            .await
        {
            warn!("Redis publish of kind:13534 failed: {e}");
        }
    }

    tracing::info!(
        member_count = members.len(),
        ts,
        "NIP-43 membership list published by buzz-admin"
    );
    Ok(())
}

/// Connect to DB, Redis pub/sub, and load the relay keypair.
///
/// `BUZZ_RELAY_PRIVATE_KEY` is required — the CLI signs kind:13534 events.
async fn connect_member_services() -> Result<(Db, Arc<PubSubManager>, Keys)> {
    let db = connect_db().await?;

    let relay_keypair = {
        let hex = std::env::var("BUZZ_RELAY_PRIVATE_KEY").map_err(|_| {
            anyhow::anyhow!(
                "BUZZ_RELAY_PRIVATE_KEY is required for add-member/remove-member.\n\
                 The relay must have a stable signing key to publish kind:13534 events."
            )
        })?;
        Keys::parse(&hex).map_err(|e| anyhow::anyhow!("invalid BUZZ_RELAY_PRIVATE_KEY: {e}"))?
    };

    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".to_string());

    let redis_pool = {
        let cfg = deadpool_redis::Config::from_url(&redis_url);
        cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .map_err(|e| anyhow::anyhow!("Redis pool creation failed: {e}"))?
    };

    let pubsub = Arc::new(
        PubSubManager::new(&redis_url, redis_pool)
            .await
            .map_err(|e| anyhow::anyhow!("PubSub init failed: {e}"))?,
    );

    Ok((db, pubsub, relay_keypair))
}

async fn connect_db() -> Result<Db> {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://buzz:buzz_dev@localhost:5432/buzz".to_string());
    let db = Db::new(&DbConfig {
        database_url: db_url,
        ..DbConfig::default()
    })
    .await?;
    Ok(db)
}

/// Resolve the deployment's tenant from the configured `RELAY_URL` host.
///
/// `buzz-admin` runs inside the relay container (`compose exec relay
/// buzz-admin …`), so it shares the relay's `RELAY_URL` and resolves the same
/// single community against the durable `communities` host map. This is
/// deliberately NOT a default tenant: an unmapped host fails closed with an
/// error, mirroring the relay's own `bind_community` row-zero seam. The CLI is
/// single-community per invocation — there is no cross-community sweep.
async fn resolve_admin_tenant(db: &Db) -> Result<TenantContext> {
    let relay_url =
        std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string());
    // Derive the authority the *same* way startup seeding and live request
    // resolution do (`buzz_core::tenant::relay_url_authority`): host plus an
    // explicit non-default port, IPv6 brackets preserved. A plain
    // `Url::host_str()` drops the port/brackets, so for `ws://localhost:3000`
    // the admin would look up `localhost` while startup seeded `localhost:3000`
    // — and `wss://relay.example:8443` would resolve `relay.example`. Sharing
    // the helper keeps buzz-admin byte-identical to the community startup seeds.
    let host = relay_url_authority(&relay_url);
    let record = db.lookup_community_by_host(&host).await?.ok_or_else(|| {
        anyhow::anyhow!(
            "RELAY_URL host '{host}' is not mapped to a community.\n\
             buzz-admin operates on the configured relay's community; ensure the \
             relay has started and seeded its community (or set RELAY_URL to a \
             mapped host)."
        )
    })?;
    Ok(TenantContext::resolved(record.id, record.host))
}

async fn reconcile_channels(relay_key_arg: Option<String>) -> Result<()> {
    use buzz_core::kind::KIND_NIP29_GROUP_ADMINS;
    use buzz_db::event::EventQuery;

    let db = connect_db().await?;

    // Resolve relay signing key: arg > env > ephemeral
    let relay_keys = match relay_key_arg.or_else(|| std::env::var("BUZZ_RELAY_PRIVATE_KEY").ok()) {
        Some(key_hex) => {
            Keys::parse(&key_hex).map_err(|e| anyhow::anyhow!("invalid relay key: {e}"))?
        }
        None => {
            let k = Keys::generate();
            eprintln!(
                "Warning: no relay key provided — using ephemeral key {}",
                k.public_key().to_hex()
            );
            eprintln!("Events signed with this key won't be verifiable after this run.");
            eprintln!("Pass --relay-key or set BUZZ_RELAY_PRIVATE_KEY for production use.");
            k
        }
    };

    let tenant = resolve_admin_tenant(&db).await?;
    let channels = db.list_channels(tenant.community(), None).await?;
    if channels.is_empty() {
        println!("No channels in database.");
        return Ok(());
    }

    let mut reconciled = 0u32;
    let mut skipped = 0u32;

    for channel in &channels {
        let channel_id_str = channel.id.to_string();

        // Check if kind:39000 already exists
        let existing = db
            .query_events(&EventQuery {
                kinds: Some(vec![39000]),
                d_tag: Some(channel_id_str.clone()),
                limit: Some(1),
                ..EventQuery::for_community(tenant.community())
            })
            .await
            .unwrap_or_default();

        if !existing.is_empty() {
            skipped += 1;
            continue;
        }

        let members = db.get_members(tenant.community(), channel.id).await?;

        // kind:39000 — channel metadata
        {
            let mut tags: Vec<Tag> = vec![Tag::parse(["d", &channel_id_str])?];
            tags.push(Tag::parse(["name", &channel.name])?);
            if let Some(ref desc) = channel.description {
                if !desc.is_empty() {
                    tags.push(Tag::parse(["about", desc])?);
                }
            }
            if channel.visibility == "private" {
                tags.push(Tag::parse(["private"])?);
            } else {
                tags.push(Tag::parse(["public"])?);
            }
            if channel.channel_type == "dm" {
                tags.push(Tag::parse(["hidden"])?);
            }
            tags.push(Tag::parse(["closed"])?);
            tags.push(Tag::parse(["t", &channel.channel_type])?);

            let event = EventBuilder::new(Kind::Custom(39000), "")
                .tags(tags)
                .sign_with_keys(&relay_keys)
                .map_err(|e| anyhow::anyhow!("sign kind:39000: {e}"))?;
            db.replace_addressable_event(tenant.community(), &event, Some(channel.id))
                .await?;
        }

        // kind:39001 — admins
        {
            let mut tags: Vec<Tag> = vec![Tag::parse(["d", &channel_id_str])?];
            for m in members
                .iter()
                .filter(|m| m.role == "owner" || m.role == "admin")
            {
                let pk = hex::encode(&m.pubkey);
                tags.push(Tag::parse(["p", &pk, &m.role])?);
            }
            let event = EventBuilder::new(Kind::Custom(KIND_NIP29_GROUP_ADMINS as u16), "")
                .tags(tags)
                .sign_with_keys(&relay_keys)
                .map_err(|e| anyhow::anyhow!("sign kind:39001: {e}"))?;
            db.replace_addressable_event(tenant.community(), &event, Some(channel.id))
                .await?;
        }

        // kind:39002 — members
        {
            let mut tags: Vec<Tag> = vec![Tag::parse(["d", &channel_id_str])?];
            for m in &members {
                let pk = hex::encode(&m.pubkey);
                tags.push(Tag::parse(["p", &pk, "", &m.role])?);
            }
            let event = EventBuilder::new(Kind::Custom(39002), "")
                .tags(tags)
                .sign_with_keys(&relay_keys)
                .map_err(|e| anyhow::anyhow!("sign kind:39002: {e}"))?;
            db.replace_addressable_event(tenant.community(), &event, Some(channel.id))
                .await?;
        }

        reconciled += 1;
    }

    println!(
        "Reconciled {reconciled} channels ({skipped} already had events, {} total).",
        channels.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use nostr::ToBech32;

    /// Deterministic pubkey fixtures (hex + npub for the same key), derived
    /// from a fixed secret key so no hard-coded constants can drift.
    fn fixture_pubkey() -> (String, String) {
        let keys = Keys::parse("0000000000000000000000000000000000000000000000000000000000000001")
            .expect("fixture secret key is valid");
        let hex = keys.public_key().to_hex();
        let npub = keys
            .public_key()
            .to_bech32()
            .expect("bech32 encoding of a valid pubkey cannot fail");
        (hex, npub)
    }

    // ---- clap definition ----

    /// Smoke test: the CLI definition is internally consistent (same pattern
    /// as buzz-cli's `cli_definition_is_valid`).
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn add_member_defaults_to_member_role() {
        let (hex, _) = fixture_pubkey();
        let cli = Cli::try_parse_from(["buzz-admin", "add-member", "--pubkey", hex.as_str()])
            .expect("add-member with only --pubkey parses");
        match cli.command {
            Command::AddMember { pubkey, role } => {
                assert_eq!(pubkey, hex);
                assert_eq!(role, "member", "role must default to 'member'");
            }
            _ => panic!("expected AddMember"),
        }
    }

    #[test]
    fn add_member_accepts_explicit_role() {
        let (hex, _) = fixture_pubkey();
        let cli = Cli::try_parse_from([
            "buzz-admin",
            "add-member",
            "--pubkey",
            hex.as_str(),
            "--role",
            "admin",
        ])
        .expect("add-member with --role parses");
        match cli.command {
            Command::AddMember { role, .. } => assert_eq!(role, "admin"),
            _ => panic!("expected AddMember"),
        }
    }

    #[test]
    fn add_member_requires_pubkey_flag() {
        assert!(Cli::try_parse_from(["buzz-admin", "add-member"]).is_err());
    }

    #[test]
    fn remove_member_role_filter_is_optional() {
        let (hex, _) = fixture_pubkey();

        let cli = Cli::try_parse_from(["buzz-admin", "remove-member", "--pubkey", hex.as_str()])
            .expect("remove-member without --role parses");
        match cli.command {
            Command::RemoveMember { role, .. } => assert_eq!(role, None),
            _ => panic!("expected RemoveMember"),
        }

        let cli = Cli::try_parse_from([
            "buzz-admin",
            "remove-member",
            "--pubkey",
            hex.as_str(),
            "--role",
            "admin",
        ])
        .expect("remove-member with --role parses");
        match cli.command {
            Command::RemoveMember { role, .. } => assert_eq!(role.as_deref(), Some("admin")),
            _ => panic!("expected RemoveMember"),
        }
    }

    #[test]
    fn product_feedback_list_defaults_to_100() {
        let cli = Cli::try_parse_from(["buzz-admin", "product-feedback", "list"])
            .expect("product-feedback list parses");
        match cli.command {
            Command::ProductFeedback {
                command: ProductFeedbackCommand::List { limit },
            } => assert_eq!(limit, 100),
            _ => panic!("expected ProductFeedback::List"),
        }
    }

    #[test]
    fn product_feedback_list_enforces_limit_range() {
        // In range: 1 and 1000 are accepted.
        for ok in ["1", "1000"] {
            assert!(
                Cli::try_parse_from(["buzz-admin", "product-feedback", "list", "--limit", ok])
                    .is_ok(),
                "--limit {ok} should be accepted"
            );
        }
        // Out of range: 0 and 1001 are rejected at parse time.
        for bad in ["0", "1001"] {
            assert!(
                Cli::try_parse_from(["buzz-admin", "product-feedback", "list", "--limit", bad])
                    .is_err(),
                "--limit {bad} should be rejected"
            );
        }
        // The subcommand itself is required.
        assert!(Cli::try_parse_from(["buzz-admin", "product-feedback"]).is_err());
    }

    #[test]
    fn reconcile_channels_relay_key_is_optional() {
        let cli = Cli::try_parse_from(["buzz-admin", "reconcile-channels"])
            .expect("reconcile-channels without --relay-key parses");
        match cli.command {
            Command::ReconcileChannels { relay_key } => assert_eq!(relay_key, None),
            _ => panic!("expected ReconcileChannels"),
        }

        let cli = Cli::try_parse_from(["buzz-admin", "reconcile-channels", "--relay-key", "abcd"])
            .expect("reconcile-channels with --relay-key parses");
        match cli.command {
            Command::ReconcileChannels { relay_key } => {
                assert_eq!(relay_key.as_deref(), Some("abcd"));
            }
            _ => panic!("expected ReconcileChannels"),
        }
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["buzz-admin", "no-such-command"]).is_err());
    }

    // ---- validate_role ----

    #[test]
    fn validate_role_accepts_member_and_admin() {
        assert_eq!(validate_role("member"), Ok(()));
        assert_eq!(validate_role("admin"), Ok(()));
    }

    #[test]
    fn validate_role_rejects_owner_with_config_hint() {
        let err = validate_role("owner").expect_err("'owner' must be rejected");
        assert!(
            err.contains("RELAY_OWNER_PUBKEY"),
            "owner rejection should point at RELAY_OWNER_PUBKEY config, got: {err}"
        );
    }

    #[test]
    fn validate_role_rejects_unknown_roles() {
        let err = validate_role("moderator").expect_err("unknown role must be rejected");
        assert!(
            err.contains("moderator"),
            "error should name the offending role, got: {err}"
        );
        assert!(validate_role("").is_err());
        // Roles are case-sensitive: "Admin" is not "admin".
        assert!(validate_role("Admin").is_err());
    }

    // ---- parse_pubkey_hex ----

    #[test]
    fn parse_pubkey_hex_accepts_hex() {
        let (hex, _) = fixture_pubkey();
        assert_eq!(parse_pubkey_hex(&hex), Ok(hex.clone()));
    }

    #[test]
    fn parse_pubkey_hex_accepts_npub() {
        let (hex, npub) = fixture_pubkey();
        assert_eq!(
            parse_pubkey_hex(&npub),
            Ok(hex),
            "npub must decode to the same lowercase hex"
        );
    }

    #[test]
    fn parse_pubkey_hex_normalizes_uppercase_hex() {
        let (hex, _) = fixture_pubkey();
        assert_eq!(
            parse_pubkey_hex(&hex.to_uppercase()),
            Ok(hex),
            "uppercase hex input must normalize to lowercase"
        );
    }

    #[test]
    fn parse_pubkey_hex_rejects_garbage() {
        let err = parse_pubkey_hex("not-a-pubkey").expect_err("garbage must be rejected");
        assert!(
            err.contains("not-a-pubkey"),
            "error should echo the offending input, got: {err}"
        );
    }

    #[test]
    fn parse_pubkey_hex_rejects_truncated_hex() {
        let (hex, _) = fixture_pubkey();
        // 63 chars — one nibble short of a pubkey.
        assert!(parse_pubkey_hex(&hex[..63]).is_err());
    }

    // ---- bumped_created_at (same-second domination guard) ----

    #[test]
    fn bump_uses_now_when_no_existing_list() {
        assert_eq!(bumped_created_at(1_700_000_000, None), 1_700_000_000);
    }

    #[test]
    fn bump_uses_now_when_existing_is_older() {
        assert_eq!(
            bumped_created_at(1_700_000_000, Some(1_699_999_998)),
            1_700_000_000
        );
    }

    #[test]
    fn bump_advances_one_second_on_same_second_collision() {
        // Serial invocation within the same second: without the bump the new
        // 13534 would be dominated and never dispatched to Redis.
        assert_eq!(
            bumped_created_at(1_700_000_000, Some(1_700_000_000)),
            1_700_000_001
        );
    }

    #[test]
    fn bump_stays_monotonic_under_backwards_clock_skew() {
        // Existing event is newer than "now" (clock stepped back): the new
        // snapshot must still dominate the old one.
        assert_eq!(
            bumped_created_at(1_700_000_000, Some(1_700_000_005)),
            1_700_000_006
        );
    }
}
