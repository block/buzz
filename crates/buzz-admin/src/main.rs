#![deny(unsafe_code)]

//! Sprout instance administration CLI.
//!
//! In the pure Nostr architecture, API tokens no longer exist.
//! Admin operations are performed via signed Nostr events (NIP-43 relay admin commands).
//! This binary is retained as a placeholder for future admin tooling.

use anyhow::Result;
use buzz_db::{Db, DbConfig};
use clap::{Parser, Subcommand};
use nostr::Keys;

#[derive(Parser)]
#[command(name = "buzz-admin", about = "Buzz instance administration")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Add a pubkey to the relay membership list.
    AddMember {
        /// Nostr public key (hex) to add.
        #[arg(long)]
        pubkey: String,

        /// Role: "admin" or "member" (default: member).
        #[arg(long, default_value = "member")]
        role: String,
    },
    /// List all relay members.
    ListMembers,
    /// Generate a new Nostr keypair (for bootstrapping).
    GenerateKey,
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::GenerateKey => {
            let keys = Keys::generate();
            println!("Public key:  {}", keys.public_key().to_hex());
            println!("Secret key:  {}", keys.secret_key().display_secret());
            println!("\nSet BUZZ_PRIVATE_KEY to the secret key to use this identity.");
        }
        Command::AddMember { pubkey, role } => {
            let db = connect_db().await?;
            let pk_bytes = hex::decode(&pubkey)?;
            if pk_bytes.len() != 32 {
                anyhow::bail!("pubkey must be 32 bytes (64 hex chars)");
            }
            db.ensure_user(&pk_bytes).await?;
            // Add to relay members via DB (admin bootstrap — normally done via kind:9030)
            db.add_relay_member(&pubkey, &role, None).await?;
            println!("Added {} as {} to relay membership list.", pubkey, role);
        }
        Command::ListMembers => {
            let db = connect_db().await?;
            let members = db.list_relay_members().await?;
            if members.is_empty() {
                println!("No relay members found.");
            } else {
                println!("{:<66}  {:<10}", "Pubkey", "Role");
                println!("{}", "-".repeat(78));
                for m in &members {
                    println!("{:<66}  {:<10}", hex::encode(&m.pubkey), m.role);
                }
            }
        }
        Command::ReconcileChannels { relay_key } => {
            reconcile_channels(relay_key).await?;
        }
    }

    Ok(())
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

async fn reconcile_channels(relay_key_arg: Option<String>) -> Result<()> {
    use buzz_core::kind::KIND_NIP29_GROUP_ADMINS;
    use buzz_db::event::EventQuery;
    use nostr::{EventBuilder, Kind, Tag};

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

    let channels = db.list_channels(None).await?;
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
                ..Default::default()
            })
            .await
            .unwrap_or_default();

        if !existing.is_empty() {
            skipped += 1;
            continue;
        }

        let members = db.get_members(channel.id).await?;

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
            db.replace_addressable_event(&event, Some(channel.id))
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
            db.replace_addressable_event(&event, Some(channel.id))
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
            db.replace_addressable_event(&event, Some(channel.id))
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
