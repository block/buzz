//! Owner-side simulator for exercising a spawner end to end without the
//! desktop app.
//!
//! Stands in for the React attestation prompt: publishes an agent spec, answers
//! the spawner's NIP-OA attestation request, and prints status as it arrives.
//! It signs every request automatically — the real client asks a human first,
//! which is exactly the difference this simulator exists to skip.
//!
//! ```sh
//! cargo run -p buzz-spawner --example owner_sim -- \
//!   --relay ws://127.0.0.1:3000 \
//!   --spawner <spawner-pubkey-hex> \
//!   --owner-nsec <nsec> \
//!   --slug fizz-prod
//! ```

use std::{collections::HashMap, time::Duration};

use anyhow::{bail, Context, Result};
use buzz_core::kind::{KIND_SPAWNER_AGENT_STATUS, KIND_SPAWNER_ATTESTATION};
use buzz_sdk::nip_oa;
use buzz_sdk::spawner::{
    build_spawner_agent_spec, build_spawner_attestation, status_from_event, AttestationFrame,
    RespondTo, SpawnerAgentSpec,
};
use buzz_ws_client::{connection::NostrWsConnection, message::RelayMessage};
use nostr::{Keys, PublicKey};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse()?;

    let keys = match &args.owner_nsec {
        Some(nsec) => Keys::parse(nsec).context("invalid --owner-nsec")?,
        None => Keys::generate(),
    };
    let spawner = PublicKey::parse(&args.spawner).context("invalid --spawner")?;

    println!("owner   : {}", keys.public_key().to_hex());
    println!("spawner : {}", spawner.to_hex());
    println!("slug    : {}", args.slug);
    println!();

    let mut conn = NostrWsConnection::connect_authenticated(&args.relay, &keys, None)
        .await
        .with_context(|| format!("failed to authenticate to {}", args.relay))?;
    println!("✓ authenticated to {}", args.relay);

    // Watch for the handshake and for status, before publishing the spec — a
    // spawner can react faster than a second round trip.
    conn.send_raw(&json!([
        "REQ", "attest",
        { "kinds": [KIND_SPAWNER_ATTESTATION], "#p": [keys.public_key().to_hex()] }
    ]))
    .await?;
    conn.send_raw(&json!([
        "REQ", "status",
        { "kinds": [KIND_SPAWNER_AGENT_STATUS], "#p": [keys.public_key().to_hex()] }
    ]))
    .await?;

    // Optionally publish an unshared persona and point the spec at it, to
    // exercise the relay's persona read-gate from the spawner's side.
    if let Some(persona_id) = &args.persona {
        let content = serde_json::json!({
            "display_name": args.name,
            "system_prompt": "You are a persona-backed test agent.",
        })
        .to_string();
        let mut tags = vec![nostr::Tag::parse(["d", persona_id.as_str()])?];
        if args.share_persona {
            tags.push(nostr::Tag::parse(["shared", "true"])?);
        }
        let event = nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core::kind::KIND_PERSONA as u16),
            content,
        )
        .tags(tags)
        .sign_with_keys(&keys)?;
        let ok = conn.send_event(event).await?;
        if !ok.accepted {
            bail!("relay rejected the persona: {}", ok.message);
        }
        println!(
            "✓ published persona {persona_id} (shared={})",
            args.share_persona
        );
    }

    if args.delete {
        // An emptied replacement is the tombstone convention for a
        // parameterized-replaceable kind; a kind:5 deletion would leave the
        // spawner nothing to fan out. Built directly because the SDK builder
        // validates the spec body, which a tombstone deliberately lacks.
        let event = nostr::EventBuilder::new(
            nostr::Kind::Custom(buzz_core::kind::KIND_SPAWNER_AGENT_SPEC as u16),
            "",
        )
        .tags(vec![
            nostr::Tag::parse(["d", args.slug.as_str()])?,
            nostr::Tag::parse(["spawner", spawner.to_hex().as_str()])?,
            nostr::Tag::parse(["p", spawner.to_hex().as_str()])?,
        ])
        .sign_with_keys(&keys)?;
        let ok = conn.send_event(event).await?;
        println!(
            "✓ published tombstone for {} (accepted={})",
            args.slug, ok.accepted
        );
    } else {
        let event = build_spawner_agent_spec(&args.slug, &spawner.to_hex(), &spec_for(&args))?
            .sign_with_keys(&keys)?;
        let ok = conn.send_event(event).await?;
        if !ok.accepted {
            bail!("relay rejected the spec: {}", ok.message);
        }
        println!("✓ published spec {} → {}", args.slug, spawner.to_hex());
    }

    println!("\nwatching (Ctrl-C to stop)…\n");

    let mut seen_status: HashMap<String, String> = HashMap::new();
    loop {
        let msg = match conn.next_event(Duration::from_secs(30)).await {
            Ok(msg) => msg,
            Err(buzz_ws_client::error::WsClientError::Timeout) => continue,
            Err(e) => return Err(e.into()),
        };

        let RelayMessage::Event { event, .. } = msg else {
            continue;
        };
        let kind = event.kind.as_u16() as u32;

        if kind == KIND_SPAWNER_ATTESTATION {
            // Skip our own outbound frames echoed back off the ephemeral stream.
            if event.pubkey == keys.public_key() {
                continue;
            }
            match handle_attestation(&mut conn, &keys, &event).await {
                Ok(Some(agent)) => println!("✓ attested agent {agent}"),
                Ok(None) => {}
                Err(e) => eprintln!("✗ attestation failed: {e:#}"),
            }
            continue;
        }

        if kind == KIND_SPAWNER_AGENT_STATUS {
            let Ok(status) = status_from_event(&event) else {
                continue;
            };
            let line = format!(
                "{:?}{}{}",
                status.phase,
                status
                    .agent_pubkey
                    .as_deref()
                    .map(|a| format!(" agent={}", &a[..12]))
                    .unwrap_or_default(),
                status
                    .error
                    .as_deref()
                    .map(|e| format!(" error={e}"))
                    .unwrap_or_default(),
            );
            // Status is replaceable; only print genuine transitions.
            if seen_status.get(&args.slug) != Some(&line) {
                println!("  status: {line}");
                seen_status.insert(args.slug.clone(), line);
            }
        }
    }
}

/// Answer a spawner's attestation request. Returns the attested agent pubkey.
async fn handle_attestation(
    conn: &mut NostrWsConnection,
    keys: &Keys,
    event: &nostr::Event,
) -> Result<Option<String>> {
    let plaintext =
        nostr::nips::nip44::decrypt(keys.secret_key(), &event.pubkey, event.content.as_str())
            .context("decrypt")?;
    let frame: AttestationFrame = serde_json::from_str(&plaintext).context("parse frame")?;
    frame.validate().context("validate frame")?;

    let AttestationFrame::Request {
        spec_slug,
        agent_pubkey,
        conditions,
        nonce,
    } = frame
    else {
        // Our own response echoed back, or a reject. Nothing to do.
        return Ok(None);
    };

    println!(
        "→ attestation request for agent {} (slug {spec_slug})",
        &agent_pubkey[..12]
    );

    let agent = PublicKey::parse(&agent_pubkey).context("invalid agent pubkey in request")?;
    let auth_tag = nip_oa::compute_auth_tag(keys, &agent, &conditions).context("compute tag")?;

    let response = AttestationFrame::Response {
        spec_slug,
        agent_pubkey: agent_pubkey.clone(),
        nonce,
        auth_tag,
        // Deliver the prompt over the encrypted channel, which is the whole
        // point of the handshake carrying it: the spec stays free of it and the
        // agent's instructions never become public.
        // Only set when relocating an existing agent; this simulator mints.
        private_key_nsec: None,
        prompt: Some(buzz_sdk::spawner::PromptMaterial {
            system_prompt: Some("You are a test agent running on a Buzz spawner.".into()),
            team_instructions: None,
            model: None,
            provider: None,
        }),
    };
    let ciphertext = nostr::nips::nip44::encrypt(
        keys.secret_key(),
        &event.pubkey,
        serde_json::to_string(&response)?,
        nostr::nips::nip44::Version::V2,
    )
    .context("encrypt response")?;

    let out =
        build_spawner_attestation(&event.pubkey.to_hex(), &ciphertext)?.sign_with_keys(keys)?;
    let ok = conn.send_event(out).await?;
    if !ok.accepted {
        bail!("relay rejected the attestation response: {}", ok.message);
    }
    Ok(Some(agent_pubkey))
}

fn spec_for(args: &Args) -> SpawnerAgentSpec {
    SpawnerAgentSpec {
        name: args.name.clone(),
        // This simulator always mints a fresh identity rather than relocating
        // an existing agent.
        agent_pubkey: None,
        // Inline prompt rather than a persona id: this simulator has no persona
        // store, and a definition-less spec is the path that does not depend on
        // the relay's persona read-delegation.
        persona_id: args.persona.clone(),
        // A persona-backed spec omits the prompt, matching how kind:30177 slims
        // against kind:30175 — the spawner must resolve it from the relay.
        system_prompt: if args.persona.is_some() {
            None
        } else {
            Some("You are a test agent running on a Buzz spawner.".into())
        },
        model: None,
        provider: None,
        parallelism: 1,
        respond_to: RespondTo::Anyone,
        respond_to_allowlist: Vec::new(),
        resources: None,
        enabled: !args.disabled,
    }
}

struct Args {
    relay: String,
    spawner: String,
    owner_nsec: Option<String>,
    slug: String,
    name: String,
    disabled: bool,
    delete: bool,
    persona: Option<String>,
    share_persona: bool,
}

impl Args {
    fn parse() -> Result<Self> {
        let mut relay = "ws://127.0.0.1:3000".to_string();
        let mut spawner = None;
        let mut owner_nsec = std::env::var("OWNER_NSEC").ok();
        let mut slug = "test-agent".to_string();
        let mut name = "Test Agent".to_string();
        let mut disabled = false;
        let mut delete = false;
        let mut persona = None;
        let mut share_persona = false;

        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--relay" => relay = args.next().context("--relay needs a value")?,
                "--spawner" => spawner = Some(args.next().context("--spawner needs a value")?),
                "--owner-nsec" => {
                    owner_nsec = Some(args.next().context("--owner-nsec needs a value")?)
                }
                "--slug" => slug = args.next().context("--slug needs a value")?,
                "--name" => name = args.next().context("--name needs a value")?,
                "--disabled" => disabled = true,
                "--delete" => delete = true,
                "--persona" => persona = Some(args.next().context("--persona needs a value")?),
                "--share-persona" => share_persona = true,
                other => bail!("unknown argument {other}"),
            }
        }

        Ok(Self {
            relay,
            spawner: spawner.context("--spawner <pubkey-hex> is required")?,
            owner_nsec,
            slug,
            name,
            disabled,
            delete,
            persona,
            share_persona,
        })
    }
}
