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

mod deletions;

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use buzz_core::kind::KIND_NIP43_MEMBERSHIP_LIST;
use buzz_core::tenant::{relay_url_authority, TenantContext};
use buzz_db::{Db, DbConfig};
use buzz_media::{BucketSnapshot, MediaConfig, MediaStorage, S3AddressingStyle, SweepError};
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
    /// Compute one complete S3 storage snapshot and persist it for relay readers.
    StorageSnapshot {
        /// Abort before folding a page that would exceed this object count.
        #[arg(long, default_value_t = 10_000_000)]
        max_objects: u64,
    },
    /// Inspect deployment-wide Buzz product feedback.
    ProductFeedback {
        #[command(subcommand)]
        command: ProductFeedbackCommand,
    },
    /// Durable CLI-only whole-community deletion control plane.
    Deletions {
        #[command(subcommand)]
        command: deletions::DeletionsCommand,
    },
    /// Emit missing kind:39000/39001/39002 channel discovery events, or
    /// republish only a targeted channel's kind:39002 roster.
    ///
    /// Without `--channel`, only channels missing discovery metadata are
    /// reconciled. With `--channel`, only that channel's member snapshot is
    /// replaced; canonical metadata and admin events remain untouched.
    ReconcileChannels {
        /// Optional channel UUID to force-republish.
        #[arg(long)]
        channel: Option<String>,

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
        Command::StorageSnapshot { max_objects } => cmd_storage_snapshot(max_objects).await,
        Command::AddMember { pubkey, role } => cmd_add_member(pubkey, role).await,
        Command::RemoveMember { pubkey, role } => cmd_remove_member(pubkey, role).await,
        Command::ListMembers => cmd_list_members().await,
        Command::ProductFeedback {
            command: ProductFeedbackCommand::List { limit },
        } => cmd_list_product_feedback(limit).await,
        Command::Deletions { command } => deletions::run(command).await,
        Command::ReconcileChannels { channel, relay_key } => {
            reconcile_channels(channel, relay_key).await?;
            Ok(0)
        }
    }
}

async fn cmd_storage_snapshot(max_objects: u64) -> Result<i32> {
    let max_objects_db = i64::try_from(max_objects)
        .map_err(|_| anyhow::anyhow!("--max-objects must be at most {}", i64::MAX))?;
    if max_objects == 0 {
        return Err(anyhow::anyhow!("--max-objects must be greater than zero"));
    }

    let db = connect_db().await?;
    let mut leader = db.try_lock_storage_accounting().await?.ok_or_else(|| {
        anyhow::anyhow!("another storage-snapshot worker already holds the lease")
    })?;
    let storage = Arc::new(MediaStorage::new(&storage_config_from_env()?)?);
    let code_sha =
        std::env::var("BUZZ_STORAGE_SNAPSHOT_CODE_SHA").unwrap_or_else(|_| "unknown".to_string());
    if code_sha.is_empty() || code_sha.len() > 128 {
        return Err(anyhow::anyhow!(
            "BUZZ_STORAGE_SNAPSHOT_CODE_SHA must contain 1 to 128 bytes"
        ));
    }

    println!(
        "{}",
        serde_json::json!({
            "event": "storage_snapshot_started",
            "max_objects": max_objects,
            "code_sha": code_sha,
        })
    );
    let run_started = Instant::now();
    let listed_objects = Arc::new(AtomicU64::new(0));
    let fold = buzz_media::fold_bucket_listing(max_objects, move |token| {
        let storage = Arc::clone(&storage);
        let listed_objects = Arc::clone(&listed_objects);
        async move {
            let page = storage.list_page(token, 1000).await?;
            let page_objects = u64::try_from(page.objects.len()).unwrap_or(u64::MAX);
            let before = listed_objects.fetch_add(page_objects, Ordering::Relaxed);
            let after = before.saturating_add(page_objects);
            if before / 100_000 != after / 100_000 {
                println!(
                    "{}",
                    serde_json::json!({
                        "event": "storage_snapshot_progress",
                        "listed_objects": after,
                        "max_objects": max_objects,
                    })
                );
            }
            Ok(page)
        }
    });
    let snapshot_code_sha = code_sha.clone();
    let persisted = persist_completed_fold(fold, move |encoded, duration_ms| async move {
        leader
            .save_snapshot(&encoded, duration_ms, max_objects_db, &snapshot_code_sha)
            .await?;
        Ok(())
    })
    .await;
    let (snapshot, duration_ms) = match persisted {
        Ok(completed) => completed,
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({
                    "event": "storage_snapshot_failed",
                    "duration_ms": i64::try_from(run_started.elapsed().as_millis()).unwrap_or(i64::MAX),
                    "max_objects": max_objects,
                    "code_sha": code_sha,
                    "error": error.to_string(),
                })
            );
            return Err(error);
        }
    };
    println!(
        "{}",
        serde_json::json!({
            "event": "storage_snapshot_completed",
            "duration_ms": duration_ms,
            "listed_objects": snapshot.physical_objects,
            "listed_bytes": snapshot.physical_bytes,
            "logical_objects": snapshot.logical_objects,
            "logical_bytes": snapshot.logical_bytes,
            "max_objects": max_objects,
            "code_sha": code_sha,
        })
    );
    Ok(0)
}

async fn persist_completed_fold<FoldFuture, Persist, PersistFuture>(
    fold: FoldFuture,
    persist: Persist,
) -> Result<(BucketSnapshot, i64)>
where
    FoldFuture: Future<Output = std::result::Result<BucketSnapshot, SweepError>>,
    Persist: FnOnce(serde_json::Value, i64) -> PersistFuture,
    PersistFuture: Future<Output = Result<()>>,
{
    let started = Instant::now();
    let snapshot = fold.await?;
    let duration_ms = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    let encoded = serde_json::to_value(&snapshot)?;
    persist(encoded, duration_ms).await?;
    Ok((snapshot, duration_ms))
}

fn storage_config_from_env() -> Result<MediaConfig> {
    let required = |name: &str| {
        std::env::var(name).map_err(|_| anyhow::anyhow!("{name} must be set for storage-snapshot"))
    };
    let addressing_style = std::env::var("BUZZ_S3_ADDRESSING_STYLE")
        .unwrap_or_else(|_| "path".to_string())
        .parse::<S3AddressingStyle>()
        .map_err(anyhow::Error::msg)?;
    Ok(MediaConfig {
        s3_endpoint: std::env::var("BUZZ_S3_ENDPOINT").unwrap_or_default(),
        s3_access_key: std::env::var("BUZZ_S3_ACCESS_KEY").unwrap_or_default(),
        s3_secret_key: std::env::var("BUZZ_S3_SECRET_KEY").unwrap_or_default(),
        s3_bucket: required("BUZZ_S3_BUCKET")?,
        s3_region: std::env::var("BUZZ_S3_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        s3_addressing_style: addressing_style,
        max_image_bytes: 1,
        max_gif_bytes: 1,
        max_video_bytes: 1,
        max_file_bytes: 1,
        public_base_url: "http://storage-snapshot.invalid/media".to_string(),
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    })
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
    let db = Db::new(
        &DbConfig {
            database_url: db_url,
            ..DbConfig::default()
        }
        .with_session_timeouts_from_env(),
    )
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

async fn reconcile_channels(
    channel_arg: Option<String>,
    relay_key_arg: Option<String>,
) -> Result<()> {
    use buzz_core::kind::KIND_NIP29_GROUP_ADMINS;
    use buzz_db::event::EventQuery;

    let db = connect_db().await?;

    // Resolve relay signing key: arg > env > ephemeral. Force-republish must
    // never use an ephemeral key because it replaces an existing authoritative
    // snapshot.
    let configured_relay_key =
        relay_key_arg.or_else(|| std::env::var("BUZZ_RELAY_PRIVATE_KEY").ok());
    if channel_arg.is_some() && configured_relay_key.is_none() {
        return Err(anyhow::anyhow!(
            "--channel requires --relay-key or BUZZ_RELAY_PRIVATE_KEY"
        ));
    }
    let relay_keys = match configured_relay_key {
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
    let target_channel = channel_arg
        .as_deref()
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid --channel UUID: {e}"))?;
    let channels = if let Some(target) = target_channel {
        vec![db
            .get_channel(tenant.community(), target)
            .await
            .map_err(|_| {
                anyhow::anyhow!("channel {target} not found in community {}", tenant.host())
            })?]
    } else {
        db.list_channels(tenant.community(), None).await?
    };
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

        if !existing.is_empty() && target_channel.is_none() {
            skipped += 1;
            continue;
        }

        let members = db.get_members(tenant.community(), channel.id).await?;

        // A targeted repair is deliberately roster-only. kind:39000 metadata
        // is richer than this legacy backfill builder, and kind:39001 is not
        // part of the stale-roster incident; replacing either can destroy
        // canonical state. Full backfill still creates all three event kinds
        // for channels with no discovery metadata.
        if target_channel.is_none() {
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
mod storage_snapshot_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[tokio::test]
    async fn failed_fold_never_invokes_snapshot_persistence() {
        let persist_calls = Arc::new(AtomicUsize::new(0));
        let observed_calls = Arc::clone(&persist_calls);
        let result = persist_completed_fold(
            async { Err::<BucketSnapshot, _>(SweepError::MalformedPage) },
            move |_, _| async move {
                observed_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(persist_calls.load(Ordering::SeqCst), 0);
    }
}
