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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::Result;
use buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST;
use buzz_core::tenant::{relay_url_authority, TenantContext};
use buzz_core::StoredEvent;
use buzz_db::{Db, DbConfig};
use buzz_pubsub::{EventTopic, PubSubManager};
use clap::{Parser, Subcommand};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
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
    /// Emit kind:39000/39001/39002 events for channels missing them.
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
    let ts = match newest_ts {
        Some(existing) => (existing + 1).max(now),
        None => now,
    };

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
    use buzz_core::kind::{
        KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_MEMBERS, KIND_NIP29_GROUP_METADATA,
    };
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

        let discovery_events = db
            .query_events(&EventQuery {
                kinds: Some(vec![
                    KIND_NIP29_GROUP_METADATA as i32,
                    KIND_NIP29_GROUP_ADMINS as i32,
                    KIND_NIP29_GROUP_MEMBERS as i32,
                ]),
                d_tag: Some(channel_id_str.clone()),
                limit: Some(100),
                ..EventQuery::for_community(tenant.community())
            })
            .await
            .unwrap_or_default();

        let members = db.get_members(tenant.community(), channel.id).await?;
        let (admin_roles, member_roles) = discovery_role_maps(
            members
                .iter()
                .map(|member| (hex::encode(&member.pubkey), member.role.clone())),
            hex::encode(&channel.created_by),
        );
        if std::env::var("BUZZ_ADMIN_RECONCILE_DEBUG").as_deref() == Ok("1") {
            eprintln!(
                "reconcile-debug channel={} creator={} admins={:?} members={:?}",
                channel.name,
                short_hex(&channel.created_by),
                prefixed_roles(&admin_roles),
                prefixed_roles(&member_roles)
            );
        }
        let admin_pubkeys: BTreeSet<String> = admin_roles.keys().cloned().collect();
        let member_pubkeys: BTreeSet<String> = member_roles.keys().cloned().collect();

        if discovery_events_are_complete(&discovery_events, &admin_pubkeys, &member_pubkeys) {
            skipped += 1;
            continue;
        }

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

            let event = EventBuilder::new(Kind::Custom(KIND_NIP29_GROUP_METADATA as u16), "")
                .allow_self_tagging()
                .tags(tags)
                .sign_with_keys(&relay_keys)
                .map_err(|e| anyhow::anyhow!("sign kind:39000: {e}"))?;
            db.replace_addressable_event(tenant.community(), &event, Some(channel.id))
                .await?;
        }

        // kind:39001 — admins
        {
            let mut tags: Vec<Tag> = vec![Tag::parse(["d", &channel_id_str])?];
            for (pk, role) in &admin_roles {
                tags.push(Tag::parse(["p", pk, role])?);
            }
            let event = EventBuilder::new(Kind::Custom(KIND_NIP29_GROUP_ADMINS as u16), "")
                .allow_self_tagging()
                .tags(tags)
                .sign_with_keys(&relay_keys)
                .map_err(|e| anyhow::anyhow!("sign kind:39001: {e}"))?;
            db.replace_addressable_event(tenant.community(), &event, Some(channel.id))
                .await?;
        }

        // kind:39002 — members
        {
            let mut tags: Vec<Tag> = vec![Tag::parse(["d", &channel_id_str])?];
            for (pk, role) in &member_roles {
                tags.push(Tag::parse(["p", pk, "", role])?);
            }
            let event = EventBuilder::new(Kind::Custom(KIND_NIP29_GROUP_MEMBERS as u16), "")
                .allow_self_tagging()
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

fn discovery_role_maps<I>(
    members: I,
    channel_creator_pubkey: String,
) -> (BTreeMap<String, String>, BTreeMap<String, String>)
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut admin_roles = BTreeMap::new();
    let mut member_roles = BTreeMap::new();

    for (pubkey, role) in members {
        let pubkey = pubkey.to_ascii_lowercase();
        if role == "owner" || role == "admin" {
            admin_roles.insert(pubkey.clone(), role.clone());
        }
        member_roles.insert(pubkey, role);
    }

    if admin_roles.is_empty() && !channel_creator_pubkey.is_empty() {
        let pubkey = channel_creator_pubkey.to_ascii_lowercase();
        admin_roles.insert(pubkey.clone(), "owner".to_string());
        member_roles
            .entry(pubkey)
            .or_insert_with(|| "owner".to_string());
    }

    (admin_roles, member_roles)
}

fn short_hex(bytes: &[u8]) -> String {
    hex::encode(bytes).chars().take(8).collect()
}

fn prefixed_roles(roles: &BTreeMap<String, String>) -> Vec<String> {
    roles
        .iter()
        .map(|(pubkey, role)| format!("{}:{role}", pubkey.chars().take(8).collect::<String>()))
        .collect()
}

fn p_tag_pubkeys(event: &Event) -> BTreeSet<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            if parts.len() >= 2 && parts[0] == "p" {
                Some(parts[1].to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect()
}

fn discovery_event_for_kind(events: &[StoredEvent], kind: u32) -> Option<&Event> {
    // EventQuery returns newest first, so the first matching addressable head is
    // the one clients will resolve. If that head is malformed, reconcile.
    events
        .iter()
        .find(|event| event.event.kind.as_u16() as u32 == kind)
        .map(|event| &event.event)
}

fn discovery_event_has_exact_p_tags(event: Option<&Event>, expected: &BTreeSet<String>) -> bool {
    event.is_some_and(|event| p_tag_pubkeys(event) == *expected)
}

fn discovery_events_are_complete(
    events: &[StoredEvent],
    admin_pubkeys: &BTreeSet<String>,
    member_pubkeys: &BTreeSet<String>,
) -> bool {
    discovery_event_for_kind(events, buzz_core::kind::KIND_NIP29_GROUP_METADATA).is_some()
        && discovery_event_has_exact_p_tags(
            discovery_event_for_kind(events, buzz_core::kind::KIND_NIP29_GROUP_ADMINS),
            admin_pubkeys,
        )
        && discovery_event_has_exact_p_tags(
            discovery_event_for_kind(events, buzz_core::kind::KIND_NIP29_GROUP_MEMBERS),
            member_pubkeys,
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::{
        KIND_NIP29_GROUP_ADMINS, KIND_NIP29_GROUP_MEMBERS, KIND_NIP29_GROUP_METADATA,
    };

    fn signed_stored_event(kind: u32, tags: Vec<Tag>) -> StoredEvent {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(kind as u16), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign event");
        StoredEvent::new(event, None)
    }

    fn p_tag(pubkey: &str, role: &str) -> Tag {
        Tag::parse(["p", pubkey, "", role]).expect("p tag")
    }

    fn d_tag(channel_id: &str) -> Tag {
        Tag::parse(["d", channel_id]).expect("d tag")
    }

    #[test]
    fn discovery_role_maps_preserve_explicit_owner_and_members() {
        let owner = "a".repeat(64);
        let member = "b".repeat(64);

        let (admin_roles, member_roles) = discovery_role_maps(
            vec![
                (owner.clone(), "owner".to_string()),
                (member.clone(), "member".to_string()),
            ],
            "c".repeat(64),
        );

        assert_eq!(admin_roles.get(&owner).map(String::as_str), Some("owner"));
        assert_eq!(member_roles.get(&owner).map(String::as_str), Some("owner"));
        assert_eq!(
            member_roles.get(&member).map(String::as_str),
            Some("member")
        );
        assert_eq!(admin_roles.len(), 1);
    }

    #[test]
    fn discovery_role_maps_fall_back_to_creator_when_no_admin_role_exists() {
        let creator = "a".repeat(64);
        let member = "b".repeat(64);

        let (admin_roles, member_roles) = discovery_role_maps(
            vec![(member.clone(), "member".to_string())],
            creator.clone(),
        );

        assert_eq!(admin_roles.get(&creator).map(String::as_str), Some("owner"));
        assert_eq!(
            member_roles.get(&creator).map(String::as_str),
            Some("owner")
        );
        assert_eq!(
            member_roles.get(&member).map(String::as_str),
            Some("member")
        );
    }

    #[test]
    fn discovery_complete_requires_exact_admin_and_member_p_tags() {
        let owner = "a".repeat(64);
        let member = "b".repeat(64);
        let channel_id = "channel-1";

        let events = vec![
            signed_stored_event(KIND_NIP29_GROUP_METADATA, vec![d_tag(channel_id)]),
            signed_stored_event(
                KIND_NIP29_GROUP_ADMINS,
                vec![d_tag(channel_id), p_tag(&owner, "owner")],
            ),
            signed_stored_event(
                KIND_NIP29_GROUP_MEMBERS,
                vec![
                    d_tag(channel_id),
                    p_tag(&owner, "owner"),
                    p_tag(&member, "member"),
                ],
            ),
        ];

        let admin_pubkeys = BTreeSet::from([owner.clone()]);
        let member_pubkeys = BTreeSet::from([owner, member]);

        assert!(discovery_events_are_complete(
            &events,
            &admin_pubkeys,
            &member_pubkeys
        ));
    }

    #[test]
    fn discovery_incomplete_when_owner_p_tag_is_missing() {
        let owner = "a".repeat(64);
        let member = "b".repeat(64);
        let channel_id = "channel-1";

        let events = vec![
            signed_stored_event(KIND_NIP29_GROUP_METADATA, vec![d_tag(channel_id)]),
            signed_stored_event(KIND_NIP29_GROUP_ADMINS, vec![d_tag(channel_id)]),
            signed_stored_event(
                KIND_NIP29_GROUP_MEMBERS,
                vec![d_tag(channel_id), p_tag(&member, "member")],
            ),
        ];

        let admin_pubkeys = BTreeSet::from([owner.clone()]);
        let member_pubkeys = BTreeSet::from([owner, member]);

        assert!(!discovery_events_are_complete(
            &events,
            &admin_pubkeys,
            &member_pubkeys
        ));
    }

    #[test]
    fn discovery_incomplete_when_newest_head_is_malformed() {
        let owner = "a".repeat(64);
        let member = "b".repeat(64);
        let channel_id = "channel-1";

        let events = vec![
            signed_stored_event(KIND_NIP29_GROUP_METADATA, vec![d_tag(channel_id)]),
            signed_stored_event(KIND_NIP29_GROUP_ADMINS, vec![d_tag(channel_id)]),
            signed_stored_event(
                KIND_NIP29_GROUP_ADMINS,
                vec![d_tag(channel_id), p_tag(&owner, "owner")],
            ),
            signed_stored_event(
                KIND_NIP29_GROUP_MEMBERS,
                vec![
                    d_tag(channel_id),
                    p_tag(&owner, "owner"),
                    p_tag(&member, "member"),
                ],
            ),
        ];

        let admin_pubkeys = BTreeSet::from([owner.clone()]);
        let member_pubkeys = BTreeSet::from([owner, member]);

        assert!(!discovery_events_are_complete(
            &events,
            &admin_pubkeys,
            &member_pubkeys
        ));
    }

    #[test]
    fn discovery_p_tags_preserve_signer_self_reference() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let event = EventBuilder::new(Kind::Custom(KIND_NIP29_GROUP_MEMBERS as u16), "")
            .allow_self_tagging()
            .tags(vec![d_tag("channel-1"), p_tag(&owner, "owner")])
            .sign_with_keys(&keys)
            .expect("sign event");

        assert_eq!(p_tag_pubkeys(&event), BTreeSet::from([owner]));
    }
}
