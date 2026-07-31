//! Relay-driven mesh lifecycle smoke — the full Buzz join story, CI-shaped.
//!
//! Unlike `mesh_serve_client_smoke` (Mdns + hand-carried invite token) and
//! `mesh_admission_smoke` (allowlist mechanics, token passed out-of-band),
//! this harness exercises the *relay as the control plane*, the way the
//! desktop app actually joins a mesh:
//!
//!   1. MEMBERSHIP — two Nostr identities are added to a membership-gated
//!      buzz-relay (kind:13534 roster via buzz-admin); a third is not.
//!   2. ADVERTISE — each member process publishes a client-signed kind:30003
//!      status note carrying its MeshLLM owner binding
//!      (`ownerId`/`ownerVerifyingKey`/`ownerBindingSig`) and, for the serve
//!      node, `serveTargets[].endpointAddr` covered by an endpoint binding
//!      signature — the exact payload shape the desktop coordinator publishes.
//!   3. TRUST — the serve node derives its admission allowlist from the relay:
//!      status notes ∩ membership roster (`owner_ids_from_events` semantics),
//!      then starts with `TrustPolicy::Allowlist`.
//!   4. JOIN — the client node discovers the serve target from the relay,
//!      verifies both bindings and membership, and dials the advertised
//!      endpoint. No token is ever handed over out-of-band.
//!   5. INFER — a chat completion against the client's local OpenAI endpoint
//!      routes over QUIC to the serve node's model.
//!   6. DENY — the stranger gets zero kind:30003 events from the relay, and
//!      even when handed the leaked endpoint address directly it is refused
//!      admission (its owner id is not on the allowlist).
//!
//! One process per node is load-bearing: mesh-llm keeps process-global state
//! (node endpoint key, ownership attestation under `~/.mesh-llm`), so each
//! role runs with an isolated HOME — exactly how the desktop runs it (one
//! machine = one node).
//!
//! Run in CI via `scripts/ci-mesh-lifecycle-smoke.sh` (which provisions the
//! membership-gated relay), or locally:
//!
//! ```text
//! ./scripts/start-relay-for-tests.sh            # with membership env set
//! cargo build --profile ci -p buzz-admin
//! BUZZ_ADMIN_BIN=target/ci/buzz-admin \
//!   cargo run --profile ci -p buzz-relay --example mesh_relay_lifecycle_smoke
//! ```
use std::collections::BTreeSet;
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use buzz_test_client::BuzzTestClient;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use mesh_llm_host_runtime::crypto::{load_keystore, save_keystore, OwnerKeypair};
use mesh_llm_sdk::{client, serve, MeshDiscoveryMode, TrustPolicy};
use nostr::{Alphabet, Event, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
use sha2::{Digest, Sha256};

/// NIP-51 bookmark set reused for client-owned mesh discovery notes
/// (`KIND_BUZZ_MESH_MEMBER_STATUS` in the desktop coordinator).
const KIND_MESH_STATUS: u16 = 30_003;
/// NIP-43 membership roster snapshot.
const KIND_MEMBERSHIP: u16 = 13_534;
const STATUS_D_TAG_PREFIX: &str = "buzz-mesh-member-status";
const STATUS_K_TAG: &str = "buzz-mesh-status";

/// Small, real instruct model; same ref the sibling mesh examples use.
const DEFAULT_MODEL: &str = "jc-builds/SmolLM2-135M-Instruct-Q4_K_M-GGUF:Q4_K_M";

const SERVE_API_PORT: u16 = 19_537;
const SERVE_CONSOLE_PORT: u16 = 13_331;
const CLIENT_API_PORT: u16 = 19_538;
const CLIENT_CONSOLE_PORT: u16 = 13_332;
const STRANGER_API_PORT: u16 = 19_539;
const STRANGER_CONSOLE_PORT: u16 = 13_333;

/// The trusted client sees the model within seconds on one box; this bounds
/// the stranger's chance to (fail to) see it.
const CLIENT_WINDOW_SECS: u64 = 180;
const STRANGER_WINDOW_SECS: u64 = 60;

fn main() -> anyhow::Result<()> {
    match std::env::var("MESH_ROLE").ok().as_deref() {
        Some("serve") => run_role(role_serve()),
        Some("client") => run_role(role_client()),
        Some("stranger") => run_role(role_stranger()),
        _ => orchestrate(),
    }
}

/// Run a role future and exit without unwinding through C++ static
/// destructors: once the native runtime has initialized, normal process exit
/// aborts inside ggml's Metal/CPU device teardown, which would mask the real
/// error under a GGML_ASSERT backtrace.
fn run_role(role: impl std::future::Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    match runtime()?.block_on(role) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("[role] FAILED: {error:#}");
            std::process::exit(1);
        }
    }
}

/// mesh-llm's async chains overflow tokio's default 2 MiB worker stacks; the
/// desktop and the mesh binary itself both run 8 MiB workers for this reason.
fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(8 * 1024 * 1024)
        .build()?)
}

fn env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("{name} is required for this role"))
}

fn relay_ws_url() -> String {
    std::env::var("RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".to_string())
}

async fn init_native_runtime() -> anyhow::Result<()> {
    // The dynamic host runtime installs the recommended signed native runtime
    // on first use when none is cached — the same SDK-owned path the desktop
    // relies on. CI caches the install dir across runs.
    mesh_llm_host_runtime::initialize_host_runtime()
        .await
        .map_err(|error| anyhow::anyhow!("MeshLLM host runtime init failed: {error:#}"))
}

// ── Owner binding payloads ───────────────────────────────────────────────────
// Byte-for-byte the desktop's `identity::member_binding_bytes` /
// `member_endpoint_binding_bytes`; the client role verifies exactly what the
// desktop coordinator publishes. Keep in sync with
// `desktop/src-tauri/src/mesh_llm/identity.rs`.

fn member_binding_bytes(member_pubkey: &str) -> Vec<u8> {
    format!(
        "buzz-mesh-owner-binding-v1:{}",
        member_pubkey.trim().to_ascii_lowercase()
    )
    .into_bytes()
}

fn member_endpoint_binding_bytes(member_pubkey: &str, endpoint_tokens: &[String]) -> Vec<u8> {
    let mut endpoints = endpoint_tokens
        .iter()
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    endpoints.sort_unstable();
    endpoints.dedup();

    let mut digest = Sha256::new();
    for endpoint in endpoints {
        digest.update((endpoint.len() as u64).to_be_bytes());
        digest.update(endpoint.as_bytes());
    }
    format!(
        "buzz-mesh-owner-endpoint-binding-v1:{}:{}",
        member_pubkey.trim().to_ascii_lowercase(),
        hex::encode(digest.finalize())
    )
    .into_bytes()
}

// ── Relay I/O ────────────────────────────────────────────────────────────────

fn status_filter() -> Filter {
    Filter::new()
        .kind(Kind::Custom(KIND_MESH_STATUS))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::K), STATUS_K_TAG)
        .limit(100)
}

fn membership_filter() -> Filter {
    Filter::new().kind(Kind::Custom(KIND_MEMBERSHIP)).limit(1)
}

async fn query_events(
    relay: &mut BuzzTestClient,
    filters: Vec<Filter>,
) -> anyhow::Result<Vec<Event>> {
    let sid = format!("mesh-lifecycle-{}", uuid::Uuid::new_v4().simple());
    relay.subscribe(&sid, filters).await?;
    let events = relay
        .collect_until_eose(&sid, Duration::from_secs(10))
        .await?;
    relay.close_subscription(&sid).await?;
    Ok(events)
}

/// Publish this member's client-signed kind:30003 discovery note — the same
/// payload the desktop coordinator's `bind_payload_to_member` +
/// `build_status_report_event` produce.
async fn publish_status(
    relay: &mut BuzzTestClient,
    keys: &Keys,
    owner: &OwnerKeypair,
    serve_targets: &[(String, String)],
) -> anyhow::Result<()> {
    let member_pubkey = keys.public_key().to_hex();
    let endpoint_tokens: Vec<String> = serve_targets
        .iter()
        .map(|(_, endpoint)| endpoint.clone())
        .collect();
    let targets_json: Vec<serde_json::Value> = serve_targets
        .iter()
        .map(|(model, endpoint)| serde_json::json!({ "modelId": model, "endpointAddr": endpoint }))
        .collect();
    let models_json: Vec<serde_json::Value> = serve_targets
        .iter()
        .map(|(model, _)| serde_json::json!({ "id": model }))
        .collect();
    let payload = serde_json::json!({
        "ownerId": owner.owner_id(),
        "ownerVerifyingKey": hex::encode(owner.verifying_key().as_bytes()),
        "ownerBindingSig":
            hex::encode(owner.sign_bytes(&member_binding_bytes(&member_pubkey))),
        "ownerEndpointBindingSig": hex::encode(owner.sign_bytes(
            &member_endpoint_binding_bytes(&member_pubkey, &endpoint_tokens),
        )),
        "serveTargets": targets_json,
        "models": models_json,
    });
    let d_tag = format!("{STATUS_D_TAG_PREFIX}:{}", owner.owner_id());
    let d = Tag::parse(["d", d_tag.as_str()]).map_err(|error| anyhow::anyhow!("{error}"))?;
    let k = Tag::parse(["k", STATUS_K_TAG]).map_err(|error| anyhow::anyhow!("{error}"))?;
    let event = EventBuilder::new(Kind::Custom(KIND_MESH_STATUS), payload.to_string())
        .tags([d, k])
        .sign_with_keys(keys)?;
    let ok = relay.send_event(event).await?;
    anyhow::ensure!(
        ok.accepted,
        "relay rejected mesh status note: {}",
        ok.message
    );
    Ok(())
}

// ── Discovery verification (mirrors desktop `discovery.rs`) ─────────────────

fn membership_set(events: &[Event]) -> Option<BTreeSet<String>> {
    events
        .iter()
        .filter(|event| event.kind.as_u16() == KIND_MEMBERSHIP)
        .max_by_key(|event| event.created_at)
        .map(|event| {
            event
                .tags
                .iter()
                .filter_map(|tag| {
                    let slice = tag.as_slice();
                    let name = slice.first()?;
                    if name != "member" && name != "p" {
                        return None;
                    }
                    slice
                        .get(1)
                        .map(|pubkey| pubkey.trim().to_ascii_lowercase())
                })
                .filter(|pubkey| !pubkey.is_empty())
                .collect()
        })
}

/// `ownerId` must equal sha256(ownerVerifyingKey) and `ownerBindingSig` must
/// verify against the note's Nostr author — a stored note cannot be re-pointed
/// at someone else's mesh identity.
fn verified_owner_id(event: &Event) -> Option<String> {
    let content = serde_json::from_str::<serde_json::Value>(&event.content).ok()?;
    let owner_id = content.get("ownerId")?.as_str()?.trim();
    let verifying_key_bytes: [u8; 32] =
        hex::decode(content.get("ownerVerifyingKey")?.as_str()?.trim())
            .ok()?
            .try_into()
            .ok()?;
    if owner_id != hex::encode(Sha256::digest(verifying_key_bytes)) {
        return None;
    }
    let signature_bytes = hex::decode(content.get("ownerBindingSig")?.as_str()?.trim()).ok()?;
    let signature = Signature::from_slice(&signature_bytes).ok()?;
    let verifying_key = VerifyingKey::from_bytes(&verifying_key_bytes).ok()?;
    verifying_key
        .verify(&member_binding_bytes(&event.pubkey.to_hex()), &signature)
        .ok()?;
    Some(owner_id.to_string())
}

/// Extract `(model_id, endpoint_addr)` pairs from a status note, but only when
/// the endpoint binding signature covers exactly the advertised tokens.
fn verified_serve_targets(event: &Event) -> Vec<(String, String)> {
    let Ok(content) = serde_json::from_str::<serde_json::Value>(&event.content) else {
        return Vec::new();
    };
    let targets: Vec<(String, String)> = content
        .get("serveTargets")
        .and_then(serde_json::Value::as_array)
        .map(|targets| {
            targets
                .iter()
                .filter_map(|target| {
                    let model = target.get("modelId")?.as_str()?.trim().to_string();
                    let endpoint = target.get("endpointAddr")?.as_str()?.trim().to_string();
                    (!endpoint.is_empty()).then_some((model, endpoint))
                })
                .collect()
        })
        .unwrap_or_default();
    if targets.is_empty() {
        return Vec::new();
    }
    let endpoint_tokens: Vec<String> = targets
        .iter()
        .map(|(_, endpoint)| endpoint.clone())
        .collect();
    let Some(verifying_key) = content
        .get("ownerVerifyingKey")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| hex::decode(value.trim()).ok())
        .and_then(|value| <[u8; 32]>::try_from(value).ok())
        .and_then(|value| VerifyingKey::from_bytes(&value).ok())
    else {
        return Vec::new();
    };
    let Some(signature) = content
        .get("ownerEndpointBindingSig")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| hex::decode(value.trim()).ok())
        .and_then(|value| Signature::from_slice(&value).ok())
    else {
        return Vec::new();
    };
    let bytes = member_endpoint_binding_bytes(&event.pubkey.to_hex(), &endpoint_tokens);
    if verifying_key.verify(&bytes, &signature).is_err() {
        return Vec::new();
    }
    targets
}

/// Owner ids of current members with valid owner bindings — the relay-derived
/// admission roster (`owner_ids_from_events` semantics).
fn member_owner_ids(events: &[Event]) -> Vec<String> {
    let Some(members) = membership_set(events) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = events
        .iter()
        .filter(|event| event.kind.as_u16() == KIND_MESH_STATUS)
        .filter(|event| members.contains(&event.pubkey.to_hex().to_ascii_lowercase()))
        .filter_map(verified_owner_id)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

// ── Roles ────────────────────────────────────────────────────────────────────

/// SERVE (member A): publish presence, derive the allowlist from the relay,
/// start an allowlist serve node, publish the endpoint, park.
async fn role_serve() -> anyhow::Result<()> {
    init_native_runtime().await?;
    let model = std::env::var("MESH_SMOKE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let keys = Keys::parse(&env("BUZZ_MEMBER_NSEC")?)?;
    let owner = load_keystore(std::path::Path::new(&env("MESH_OWNER_KEY")?), None)
        .map_err(|error| anyhow::anyhow!("loading serve owner keystore: {error}"))?;
    let expected_owners: usize = env("MESH_EXPECTED_OWNERS")?.parse()?;

    let mut relay = BuzzTestClient::connect(&relay_ws_url(), &keys)
        .await
        .map_err(|error| anyhow::anyhow!("serve member relay connect: {error}"))?;
    publish_status(&mut relay, &keys, &owner, &[]).await?;
    println!("STATUS_PUBLISHED");
    // The upcoming serve::start() blocks through a possibly multi-minute model
    // download; an idle relay socket gets closed under it. Reconnect after.

    // TRUST: wait for every expected member owner to be visible via the relay
    // (statuses ∩ roster), then admit exactly those owners.
    let deadline = Instant::now() + Duration::from_secs(120);
    let allowlist = loop {
        let events = query_events(&mut relay, vec![status_filter(), membership_filter()]).await?;
        let mut owners = member_owner_ids(&events);
        if !owners.contains(&owner.owner_id()) {
            owners.push(owner.owner_id());
            owners.sort();
        }
        if owners.len() >= expected_owners {
            break owners;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for {expected_owners} member owners; saw {owners:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    println!("ALLOWLIST:{}", allowlist.join(","));
    let _ = relay.disconnect().await;

    let cfg = serve::EmbeddedServeConfig::builder()
        .model(&model)
        .api_port(SERVE_API_PORT)
        .console_port(SERVE_CONSOLE_PORT)
        // Desktop no-leak invariants: never publish mesh presence, never
        // auto-discover. The Buzz relay is the only discovery surface.
        .publish(false)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Nostr)
        .console_ui(true)
        .startup_timeout(Duration::from_secs(600))
        .owner_key(env("MESH_OWNER_KEY")?)
        .owner_required(true)
        .trust_policy(TrustPolicy::Allowlist)
        .trust_owners(allowlist)
        .build();
    let node = serve::start(cfg).await?;
    let endpoint = node
        .invite_token()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("serve node produced no endpoint address"))?;
    println!("ENDPOINT:{endpoint}");

    let http = reqwest::Client::new();
    let base = node.api_base_url().to_string();
    let served = wait_for_model(&http, &base, Duration::from_secs(600))
        .await?
        .ok_or_else(|| anyhow::anyhow!("serve node never loaded the model"))?;

    // ADVERTISE: refresh the status note with the live serve target, exactly
    // what the desktop's 45s heartbeat publishes once serving. Fresh relay
    // connection — the pre-download socket has long been idle-closed.
    let mut relay = BuzzTestClient::connect(&relay_ws_url(), &keys)
        .await
        .map_err(|error| anyhow::anyhow!("serve member relay reconnect: {error}"))?;
    publish_status(
        &mut relay,
        &keys,
        &owner,
        &[(served.clone(), endpoint.clone())],
    )
    .await?;
    println!("READY:{served}");

    // Park; the orchestrator kills this process when the run is over.
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

/// CLIENT (member B): publish presence, discover + verify the serve target
/// from the relay, dial it, and prove inference routes over the mesh.
async fn role_client() -> anyhow::Result<()> {
    init_native_runtime().await?;
    let keys = Keys::parse(&env("BUZZ_MEMBER_NSEC")?)?;
    let owner = load_keystore(std::path::Path::new(&env("MESH_OWNER_KEY")?), None)
        .map_err(|error| anyhow::anyhow!("loading client owner keystore: {error}"))?;

    let mut relay = BuzzTestClient::connect(&relay_ws_url(), &keys)
        .await
        .map_err(|error| anyhow::anyhow!("client member relay connect: {error}"))?;
    publish_status(&mut relay, &keys, &owner, &[]).await?;
    println!("STATUS_PUBLISHED");

    // JOIN: poll the relay until a *verified* serve target from another member
    // appears — membership roster, owner binding, and endpoint binding all
    // checked, mirroring `availability_from_events`.
    let deadline = Instant::now() + Duration::from_secs(900);
    let (endpoint, allowlist) = loop {
        let events = query_events(&mut relay, vec![status_filter(), membership_filter()]).await?;
        let members = membership_set(&events).unwrap_or_default();
        let target = events
            .iter()
            .filter(|event| event.kind.as_u16() == KIND_MESH_STATUS)
            .filter(|event| members.contains(&event.pubkey.to_hex().to_ascii_lowercase()))
            .filter(|event| verified_owner_id(event).is_some_and(|id| id != owner.owner_id()))
            .flat_map(verified_serve_targets)
            .next();
        if let Some((_, endpoint)) = target {
            break (endpoint, member_owner_ids(&events));
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for a verified serve target on the relay"
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
    };
    println!("TARGET_FOUND");

    let cfg = client::EmbeddedClientConfig::builder()
        .api_port(CLIENT_API_PORT)
        .console_port(CLIENT_CONSOLE_PORT)
        .publish(false)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Nostr)
        .console_ui(true)
        .startup_timeout(Duration::from_secs(180))
        .owner_key(env("MESH_OWNER_KEY")?)
        .owner_required(true)
        .trust_policy(TrustPolicy::Allowlist)
        .trust_owners(allowlist)
        .build();
    let node = client::start(cfg).await?;
    // The relay-discovered endpoint is the dial target — the same
    // `dial_endpoint_addr` step the desktop's join watcher performs. The
    // watcher retries every 15s (first QUIC dials can time out while the
    // serve node's endpoint is still warming up); mirror that here.
    let mut dial_result = Ok(());
    for attempt in 1..=5u32 {
        dial_result = node.join_token(&endpoint).await;
        match &dial_result {
            Ok(()) => break,
            Err(error) => {
                eprintln!("[client] dial attempt {attempt}/5 failed: {error:#}");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    }
    dial_result?;

    let http = reqwest::Client::new();
    let base = node.api_base_url().to_string();
    let seen = wait_for_model(&http, &base, Duration::from_secs(CLIENT_WINDOW_SECS)).await?;
    match &seen {
        Some(model) => {
            println!("SEEN:{model}");
            match try_completion(&http, &base, model).await {
                Ok(content) => println!("INFER_OK:{content}"),
                Err(error) => println!("INFER_FAIL:{error}"),
            }
        }
        None => println!("NONE"),
    }
    let _ = node.stop().await;
    // Skip C++ static destructors (ggml aborts in global teardown).
    std::process::exit(0);
}

/// STRANGER (non-member C): must get nothing from the relay, and must be
/// refused admission even with the leaked endpoint address.
async fn role_stranger() -> anyhow::Result<()> {
    let keys = Keys::parse(&env("BUZZ_MEMBER_NSEC")?)?;
    let leaked_endpoint = env("MESH_LEAKED_ENDPOINT")?;

    // DENY (relay read): a membership-gated relay either refuses NIP-42 auth
    // outright or returns zero mesh status notes.
    match BuzzTestClient::connect(&relay_ws_url(), &keys).await {
        Err(_) => println!("RELAY_DENIED"),
        Ok(mut relay) => {
            let events = query_events(&mut relay, vec![status_filter()]).await?;
            let statuses = events
                .iter()
                .filter(|event| event.kind.as_u16() == KIND_MESH_STATUS)
                .count();
            println!("RELAY_STATUSES:{statuses}");
            let _ = relay.disconnect().await;
        }
    }

    // DENY (admission): dial the serve node directly with the leaked endpoint.
    // The stranger's owner id is not on the allowlist, so the mesh must refuse
    // to route anything to it.
    init_native_runtime().await?;
    let cfg = client::EmbeddedClientConfig::builder()
        .api_port(STRANGER_API_PORT)
        .console_port(STRANGER_CONSOLE_PORT)
        .publish(false)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Nostr)
        .console_ui(true)
        .startup_timeout(Duration::from_secs(180))
        .owner_key(env("MESH_OWNER_KEY")?)
        .owner_required(true)
        .build();
    let node = client::start(cfg).await?;
    let _ = node.join_token(&leaked_endpoint).await;

    let http = reqwest::Client::new();
    let base = node.api_base_url().to_string();
    let seen = wait_for_model(&http, &base, Duration::from_secs(STRANGER_WINDOW_SECS)).await?;
    match &seen {
        Some(model) => {
            println!("SEEN:{model}");
            match try_completion(&http, &base, model).await {
                Ok(content) => println!("INFER_OK:{content}"),
                Err(error) => println!("INFER_FAIL:{error}"),
            }
        }
        None => println!("NONE"),
    }
    let _ = node.stop().await;
    std::process::exit(0);
}

// ── Orchestrator ─────────────────────────────────────────────────────────────

fn orchestrate() -> anyhow::Result<()> {
    let model = std::env::var("MESH_SMOKE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    eprintln!("[lifecycle] model: {model}");
    let admin =
        std::env::var("BUZZ_ADMIN_BIN").unwrap_or_else(|_| "target/ci/buzz-admin".to_string());
    anyhow::ensure!(
        std::path::Path::new(&admin).exists(),
        "buzz-admin binary not found at {admin} (set BUZZ_ADMIN_BIN)"
    );

    let scratch = std::env::temp_dir().join(format!("buzz-mesh-lifecycle-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)?;

    // Nostr identities: A (serve member), B (client member), C (stranger).
    let member_a = Keys::generate();
    let member_b = Keys::generate();
    let stranger = Keys::generate();

    // MeshLLM owner keystores, one per role.
    let make_owner = |name: &str| -> anyhow::Result<String> {
        let keypair = OwnerKeypair::generate();
        let path = scratch.join(format!("{name}.keystore.json"));
        save_keystore(&path, &keypair, None, true)
            .map_err(|error| anyhow::anyhow!("saving {name} keystore: {error}"))?;
        Ok(path.display().to_string())
    };
    let serve_key = make_owner("serve")?;
    let client_key = make_owner("client")?;
    let stranger_key = make_owner("stranger")?;

    // MEMBERSHIP: A and B become relay members via buzz-admin (publishes the
    // kind:13534 roster snapshot). C is deliberately not added.
    for (label, keys) in [("A", &member_a), ("B", &member_b)] {
        let status = Command::new(&admin)
            .args(["add-member", "--pubkey", &keys.public_key().to_hex()])
            .status()?;
        anyhow::ensure!(status.success(), "buzz-admin add-member {label} failed");
        eprintln!(
            "[lifecycle] member {label} added: {}",
            keys.public_key().to_hex()
        );
    }

    // Isolated HOMEs (mesh-llm keeps node identity under ~/.mesh-llm), with
    // the native runtime + HF caches resolved from the real environment first.
    let native_cache = std::env::var_os("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or(real_cache_dir()?.join("mesh-llm/native-runtimes"));
    let hf_cache = std::env::var_os("HF_HUB_CACHE")
        .map(std::path::PathBuf::from)
        .unwrap_or(real_cache_dir()?.join("huggingface/hub"));
    let role_home = |name: &str| -> anyhow::Result<String> {
        let home = scratch.join(format!("{name}-home"));
        std::fs::create_dir_all(&home)?;
        Ok(home.display().to_string())
    };

    let exe = std::env::current_exe()?;
    let secret_hex = |keys: &Keys| format!("{}", keys.secret_key().display_secret());

    // SERVE child (member A).
    eprintln!("[lifecycle] starting SERVE member (relay-derived allowlist)...");
    let mut serve_child = Command::new(&exe)
        .env("MESH_ROLE", "serve")
        .env("MESH_SMOKE_MODEL", &model)
        .env("BUZZ_MEMBER_NSEC", secret_hex(&member_a))
        .env("MESH_OWNER_KEY", &serve_key)
        .env("MESH_EXPECTED_OWNERS", "2")
        .env("HOME", role_home("serve")?)
        .env("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR", &native_cache)
        .env("HF_HUB_CACHE", &hf_cache)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let serve_guard = KillOnDrop(&mut serve_child);
    let serve_stdout = serve_guard
        .0
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("no serve stdout"))?;
    let mut serve_lines = std::io::BufReader::new(serve_stdout).lines();
    expect_line(
        &mut serve_lines,
        "STATUS_PUBLISHED",
        Duration::from_secs(180),
    )?;
    eprintln!("[lifecycle] serve member published its discovery note");

    // CLIENT child (member B) — started now so the serve node can see B's
    // owner binding on the relay and admit it.
    eprintln!("[lifecycle] starting CLIENT member (relay-driven join)...");
    let client_child = Command::new(&exe)
        .env("MESH_ROLE", "client")
        .env("BUZZ_MEMBER_NSEC", secret_hex(&member_b))
        .env("MESH_OWNER_KEY", &client_key)
        .env("HOME", role_home("client")?)
        .env("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR", &native_cache)
        .env("HF_HUB_CACHE", &hf_cache)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let allowlist = expect_line(&mut serve_lines, "ALLOWLIST:", Duration::from_secs(300))?;
    eprintln!("[lifecycle] PASS 1/5: relay-derived allowlist resolved: {allowlist}");
    let endpoint = expect_line(&mut serve_lines, "ENDPOINT:", Duration::from_secs(600))?;
    eprintln!("[lifecycle] serve endpoint advertised (via relay status note)");
    let served = expect_line(&mut serve_lines, "READY:", Duration::from_secs(900))?;
    eprintln!("[lifecycle] PASS 2/5: serve member ready with model: {served}");

    // Wait for the client's verdict.
    let client_out = client_child.wait_with_output()?;
    let client_verdict = parse_verdict(&String::from_utf8_lossy(&client_out.stdout))?;
    let seen = client_verdict.seen.as_deref().ok_or_else(|| {
        anyhow::anyhow!("LIFECYCLE FAIL: client member never saw the model via relay-driven join")
    })?;
    eprintln!("[lifecycle] PASS 3/5: client member discovered + joined via relay, sees: {seen}");
    let content = client_verdict.infer_ok.as_deref().ok_or_else(|| {
        anyhow::anyhow!(
            "LIFECYCLE FAIL: client saw the model but inference did not route: {:?}",
            client_verdict.infer_fail
        )
    })?;
    eprintln!("[lifecycle] PASS 4/5: inference routed over the mesh: {content:?}");

    // STRANGER child (C): zero relay visibility and refused admission.
    eprintln!("[lifecycle] starting STRANGER (non-member, leaked endpoint)...");
    let stranger_out = Command::new(&exe)
        .env("MESH_ROLE", "stranger")
        .env("BUZZ_MEMBER_NSEC", secret_hex(&stranger))
        .env("MESH_OWNER_KEY", &stranger_key)
        .env("MESH_LEAKED_ENDPOINT", &endpoint)
        .env("HOME", role_home("stranger")?)
        .env("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR", &native_cache)
        .env("HF_HUB_CACHE", &hf_cache)
        .stderr(Stdio::inherit())
        .output()?;
    let stranger_stdout = String::from_utf8_lossy(&stranger_out.stdout).to_string();
    assert_stranger_denied(&stranger_stdout)?;
    eprintln!("[lifecycle] PASS 5/5: stranger denied by relay and by mesh admission");

    eprintln!("[lifecycle] PASS: full relay-driven mesh lifecycle verified");
    drop(serve_guard);
    let _ = serve_child.wait();
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

/// The stranger passes only if the relay hid the mesh (`RELAY_DENIED` or zero
/// statuses) AND admission refused it (no model, or visible-but-unroutable).
fn assert_stranger_denied(stdout: &str) -> anyhow::Result<()> {
    let relay_hidden = stdout
        .lines()
        .any(|line| line == "RELAY_DENIED" || line == "RELAY_STATUSES:0");
    if !relay_hidden {
        let leaked = stdout
            .lines()
            .find(|line| line.starts_with("RELAY_STATUSES:"))
            .unwrap_or("<no relay verdict>");
        anyhow::bail!("LIFECYCLE FAIL: relay leaked mesh statuses to a non-member ({leaked})");
    }
    let verdict = parse_verdict(stdout)?;
    match (&verdict.seen, &verdict.infer_ok, &verdict.infer_fail) {
        (None, None, _) => Ok(()),
        (Some(model), None, Some(error)) => {
            eprintln!(
                "[lifecycle] stranger saw gossip for {model} but inference was rejected: {error}"
            );
            Ok(())
        }
        (Some(model), Some(content), _) => anyhow::bail!(
            "LIFECYCLE FAIL: stranger joined with a leaked endpoint and inferred through {model}: {content:?}"
        ),
        (Some(model), None, None) => anyhow::bail!(
            "LIFECYCLE INCONCLUSIVE: stranger saw {model} but produced no inference verdict"
        ),
        (None, Some(content), _) => anyhow::bail!(
            "LIFECYCLE FAIL: stranger inferred without model visibility: {content:?}"
        ),
    }
}

// ── Child-process plumbing (shape shared with mesh_admission_smoke) ─────────

/// What a client-shaped child reported on stdout.
#[derive(Debug, Default)]
struct Verdict {
    seen: Option<String>,
    infer_ok: Option<String>,
    infer_fail: Option<String>,
}

fn parse_verdict(stdout: &str) -> anyhow::Result<Verdict> {
    let mut verdict = Verdict::default();
    let mut saw_any = false;
    for line in stdout.lines() {
        if let Some(model) = line.strip_prefix("SEEN:") {
            verdict.seen = Some(model.to_string());
            saw_any = true;
        } else if line == "NONE" {
            saw_any = true;
        } else if let Some(content) = line.strip_prefix("INFER_OK:") {
            verdict.infer_ok = Some(content.to_string());
        } else if let Some(error) = line.strip_prefix("INFER_FAIL:") {
            verdict.infer_fail = Some(error.to_string());
        }
    }
    if !saw_any {
        anyhow::bail!("child produced no SEEN/NONE verdict; stdout: {stdout}");
    }
    Ok(verdict)
}

/// Read child stdout until a line with the given prefix appears. Returns the
/// full line's suffix; for bare marker lines the suffix is empty.
fn expect_line(
    lines: &mut std::io::Lines<std::io::BufReader<std::process::ChildStdout>>,
    prefix: &str,
    timeout: Duration,
) -> anyhow::Result<String> {
    // BufReader::lines blocks; enforce the timeout coarsely via a deadline
    // check between lines (the child prints continuously enough in practice).
    let deadline = Instant::now() + timeout;
    for line in lines.by_ref() {
        let line = line?;
        if let Some(rest) = line.strip_prefix(prefix) {
            return Ok(rest.to_string());
        }
        if Instant::now() > deadline {
            break;
        }
    }
    anyhow::bail!("child ended or timed out before printing {prefix}")
}

/// Kill the serve child on drop so a failed assertion never leaks a process.
struct KillOnDrop<'a>(&'a mut Child);
impl Drop for KillOnDrop<'_> {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

/// The real user's OS cache dir, resolved before HOME is overridden for the
/// child processes.
fn real_cache_dir() -> anyhow::Result<std::path::PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    #[cfg(target_os = "macos")]
    return Ok(std::path::PathBuf::from(home).join("Library/Caches"));
    #[cfg(not(target_os = "macos"))]
    return Ok(std::path::PathBuf::from(home).join(".cache"));
}

/// Poll `/models` until a model id appears or the window closes.
async fn wait_for_model(
    http: &reqwest::Client,
    api_base: &str,
    window: Duration,
) -> anyhow::Result<Option<String>> {
    let url = format!("{api_base}/models");
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Ok(resp) = http.get(&url).send().await {
            let body = resp.text().await.unwrap_or_default();
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(id) = json["data"].get(0).and_then(|m| m["id"].as_str()) {
                    return Ok(Some(id.to_string()));
                }
            }
        }
    }
    Ok(None)
}

/// One chat completion against a node's OpenAI endpoint; Ok(content) only if
/// it really routed and produced non-empty output.
async fn try_completion(
    http: &reqwest::Client,
    api_base: &str,
    model: &str,
) -> anyhow::Result<String> {
    let resp = http
        .post(format!("{api_base}/chat/completions"))
        .timeout(Duration::from_secs(120))
        .json(&serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "Reply with exactly one word: PONG"}],
            "max_tokens": 16,
            "temperature": 0.0
        }))
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("{status}: {body}");
    }
    let content = serde_json::from_str::<serde_json::Value>(&body)?["choices"][0]["message"]
        ["content"]
        .as_str()
        .unwrap_or("")
        .to_string();
    if content.trim().is_empty() {
        anyhow::bail!("empty content");
    }
    Ok(content)
}
