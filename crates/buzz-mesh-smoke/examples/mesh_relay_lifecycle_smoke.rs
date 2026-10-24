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
//!      status notes ∩ membership roster, and requires the *exact* expected
//!      owner set before starting with `TrustPolicy::Allowlist`.
//!   4. JOIN — the client node discovers the serve target from the relay,
//!      verifies both bindings and membership, and dials the advertised
//!      endpoint. No token is ever handed over out-of-band.
//!   5. INFER — a chat completion against the client's local OpenAI endpoint
//!      routes over QUIC to the serve node's model.
//!   6. DENY — the stranger's NIP-42 auth must fail with the relay's
//!      membership rejection, and even when handed the leaked endpoint
//!      address directly it must not complete an inference — *while the
//!      trusted client re-verifies inference immediately afterwards*, so a
//!      sick serve node cannot masquerade as an admission denial.
//!
//! ## Scope: an independent protocol harness
//!
//! This harness speaks the same wire protocol as the desktop
//! (`desktop/src-tauri/src/mesh_llm/{identity,discovery,coordinator}.rs`) but
//! deliberately re-implements the binding/verification logic rather than
//! linking desktop code (the desktop crate is outside this workspace). The
//! payloads and canonical binding bytes are kept byte-identical — see the
//! keep-in-sync comments below. A regression inside the desktop's own
//! discovery filtering is covered by the desktop unit tests, not this smoke;
//! what this smoke proves is that the relay + mesh-llm SDK + admission stack
//! actually support the lifecycle end to end.
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
use std::io::{BufRead, Write};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc;
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
/// the stranger's chance to (fail to) see it. Both windows are overridable
/// via env (`MESH_CLIENT_WINDOW_SECS` / `MESH_STRANGER_WINDOW_SECS`) so CI
/// can pin longer windows on slow shared runners instead of re-running the
/// whole job.
const CLIENT_WINDOW_SECS: u64 = 180;
const STRANGER_WINDOW_SECS: u64 = 60;

fn window_secs(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn client_window() -> Duration {
    Duration::from_secs(window_secs("MESH_CLIENT_WINDOW_SECS", CLIENT_WINDOW_SECS))
}

fn stranger_window() -> Duration {
    Duration::from_secs(window_secs(
        "MESH_STRANGER_WINDOW_SECS",
        STRANGER_WINDOW_SECS,
    ))
}

/// Marker the orchestrator writes to the client child's stdin to request the
/// post-attack inference re-verification.
const VERIFY_AGAIN: &str = "VERIFY_AGAIN";

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
fn member_owner_ids(events: &[Event]) -> BTreeSet<String> {
    let Some(members) = membership_set(events) else {
        return BTreeSet::new();
    };
    events
        .iter()
        .filter(|event| event.kind.as_u16() == KIND_MESH_STATUS)
        .filter(|event| members.contains(&event.pubkey.to_hex().to_ascii_lowercase()))
        .filter_map(verified_owner_id)
        .collect()
}

// ── Roles ────────────────────────────────────────────────────────────────────

/// SERVE (member A): publish presence, derive the allowlist from the relay,
/// require the exact expected owner set, start an allowlist serve node,
/// publish the endpoint, park.
async fn role_serve() -> anyhow::Result<()> {
    init_native_runtime().await?;
    let model = std::env::var("MESH_SMOKE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let keys = Keys::parse(&env("BUZZ_MEMBER_NSEC")?)?;
    let owner = load_keystore(std::path::Path::new(&env("MESH_OWNER_KEY")?), None)
        .map_err(|error| anyhow::anyhow!("loading serve owner keystore: {error}"))?;
    // The exact owner ids the orchestrator provisioned for members A and B.
    // Waiting for this exact set (not a count) means the allowlist can only
    // ever contain the intended identities.
    let expected_owners: BTreeSet<String> = env("MESH_EXPECTED_OWNERS")?
        .split(',')
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    anyhow::ensure!(
        expected_owners.contains(&owner.owner_id()),
        "serve owner id is not in MESH_EXPECTED_OWNERS"
    );

    let mut relay = BuzzTestClient::connect(&relay_ws_url(), &keys)
        .await
        .map_err(|error| anyhow::anyhow!("serve member relay connect: {error}"))?;
    publish_status(&mut relay, &keys, &owner, &[]).await?;
    println!("STATUS_PUBLISHED");

    // TRUST: wait until every expected member owner is visible via the relay
    // (statuses ∩ roster), then admit exactly those owners.
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let events = query_events(&mut relay, vec![status_filter(), membership_filter()]).await?;
        let mut visible = member_owner_ids(&events);
        visible.insert(owner.owner_id());
        if visible.is_superset(&expected_owners) {
            break;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for expected owners {expected_owners:?}; saw {visible:?}"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    let allowlist: Vec<String> = expected_owners.iter().cloned().collect();
    println!("ALLOWLIST:{}", allowlist.join(","));
    // The upcoming serve::start() blocks through a possibly multi-minute model
    // download; an idle relay socket gets closed under it. Reconnect after.
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
/// from the relay, dial it, prove inference routes over the mesh — then wait
/// for the orchestrator's `VERIFY_AGAIN` and re-prove inference after the
/// stranger's admission attack, so denial is differential, not absence.
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
            let owners: Vec<String> = member_owner_ids(&events).into_iter().collect();
            break (endpoint, owners);
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
    // desktop's watcher retries every 15s (a first QUIC dial can time out
    // while the serve node's endpoint is still warming up). mesh-llm itself
    // retries internally per attempt, so keep the outer budget small.
    let mut dial_result = Ok(());
    for attempt in 1..=3u32 {
        dial_result = node.join_token(&endpoint).await;
        match &dial_result {
            Ok(()) => break,
            Err(error) => {
                eprintln!("[client] dial attempt {attempt}/3 failed: {error:#}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
    dial_result?;

    let http = reqwest::Client::new();
    let base = node.api_base_url().to_string();
    let Some(model) = wait_for_model(&http, &base, client_window()).await? else {
        println!("NONE");
        let _ = node.stop().await;
        std::process::exit(0);
    };
    println!("SEEN:{model}");
    match try_completion(&http, &base, &model).await {
        Ok(content) => println!("INFER_OK:{content}"),
        Err(error) => {
            println!("INFER_FAIL:{error}");
            let _ = node.stop().await;
            std::process::exit(0);
        }
    }

    // Post-attack health proof: hold the mesh session open until the
    // orchestrator has run the stranger, then prove the serve node still
    // routes trusted inference. This is what makes the stranger's failure an
    // admission denial rather than a dead server.
    let line = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map(|_| line)
    })
    .await??;
    if line.trim() == VERIFY_AGAIN {
        match try_completion(&http, &base, &model).await {
            Ok(content) => println!("INFER_AGAIN_OK:{content}"),
            Err(error) => println!("INFER_AGAIN_FAIL:{error}"),
        }
    }
    let _ = node.stop().await;
    // Skip C++ static destructors (ggml aborts in global teardown).
    std::process::exit(0);
}

/// STRANGER (non-member C): NIP-42 auth must fail with the relay's membership
/// rejection, and the mesh must not route inference for it even with the
/// leaked endpoint address.
async fn role_stranger() -> anyhow::Result<()> {
    let keys = Keys::parse(&env("BUZZ_MEMBER_NSEC")?)?;
    let leaked_endpoint = env("MESH_LEAKED_ENDPOINT")?;

    // DENY (relay read): the membership-gated relay must reject the
    // stranger's NIP-42 auth with its membership error specifically. Any
    // other failure (relay down, timeout) is inconclusive and fails the
    // test; a successful auth is a gating regression and also fails.
    match BuzzTestClient::connect(&relay_ws_url(), &keys).await {
        Err(error) => {
            let message = error.to_string();
            if message.contains("not a relay member") {
                println!("RELAY_DENIED_MEMBERSHIP");
            } else {
                println!("RELAY_ERR:{message}");
            }
        }
        Ok(mut relay) => {
            let statuses = query_events(&mut relay, vec![status_filter()])
                .await
                .map(|events| {
                    events
                        .iter()
                        .filter(|event| event.kind.as_u16() == KIND_MESH_STATUS)
                        .count()
                })
                .unwrap_or(usize::MAX);
            println!("RELAY_AUTH_OK:{statuses}");
            let _ = relay.disconnect().await;
        }
    }

    // DENY (admission): dial the serve node directly with the leaked endpoint.
    // The stranger's owner id is not on the allowlist, so the mesh must refuse
    // to route anything to it. Note the dial itself may locally "succeed" —
    // mesh-llm applies the receiving node's owner policy after the handshake —
    // so the decisive probe is routed inference, cross-checked against the
    // trusted client's post-attack inference by the orchestrator.
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
    match wait_for_model(&http, &base, stranger_window()).await? {
        Some(model) => {
            println!("SEEN:{model}");
            match try_completion(&http, &base, &model).await {
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

    // MeshLLM owner keystores, one per role. The orchestrator keeps the owner
    // ids so the serve role can gate on the exact expected identity set.
    let make_owner = |name: &str| -> anyhow::Result<(String, String)> {
        let keypair = OwnerKeypair::generate();
        let path = scratch.join(format!("{name}.keystore.json"));
        save_keystore(&path, &keypair, None, true)
            .map_err(|error| anyhow::anyhow!("saving {name} keystore: {error}"))?;
        Ok((path.display().to_string(), keypair.owner_id()))
    };
    let (serve_key, serve_owner_id) = make_owner("serve")?;
    let (client_key, client_owner_id) = make_owner("client")?;
    let (stranger_key, _stranger_owner_id) = make_owner("stranger")?;
    let expected_owners = format!("{serve_owner_id},{client_owner_id}");

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
        .env("MESH_EXPECTED_OWNERS", &expected_owners)
        .env("HOME", role_home("serve")?)
        .env("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR", &native_cache)
        .env("HF_HUB_CACHE", &hf_cache)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let serve_lines = spawn_line_reader(
        serve_child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no serve stdout"))?,
    );
    let serve_guard = KillOnDrop(&mut serve_child);
    expect_line(&serve_lines, "STATUS_PUBLISHED", Duration::from_secs(180))?;
    eprintln!("[lifecycle] serve member published its discovery note");

    // CLIENT child (member B) — started now so the serve node can see B's
    // owner binding on the relay and admit it. stdin stays piped for the
    // post-attack VERIFY_AGAIN request.
    eprintln!("[lifecycle] starting CLIENT member (relay-driven join)...");
    let mut client_child = Command::new(&exe)
        .env("MESH_ROLE", "client")
        .env("BUZZ_MEMBER_NSEC", secret_hex(&member_b))
        .env("MESH_OWNER_KEY", &client_key)
        .env("HOME", role_home("client")?)
        .env("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR", &native_cache)
        .env("HF_HUB_CACHE", &hf_cache)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let client_lines = spawn_line_reader(
        client_child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no client stdout"))?,
    );
    let mut client_stdin = client_child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("no client stdin"))?;
    let client_guard = KillOnDrop(&mut client_child);

    let allowlist = expect_line(&serve_lines, "ALLOWLIST:", Duration::from_secs(300))?;
    anyhow::ensure!(
        allowlist.split(',').map(str::trim).collect::<BTreeSet<_>>()
            == BTreeSet::from([serve_owner_id.as_str(), client_owner_id.as_str()]),
        "LIFECYCLE FAIL: serve allowlist {allowlist} is not exactly the expected member owners"
    );
    eprintln!("[lifecycle] PASS 1/6: relay-derived allowlist is exactly {{A, B}}: {allowlist}");
    let endpoint = expect_line(&serve_lines, "ENDPOINT:", Duration::from_secs(600))?;
    eprintln!("[lifecycle] serve endpoint acquired (relay advertisement lands with READY)");
    let served = expect_line(&serve_lines, "READY:", Duration::from_secs(900))?;
    eprintln!("[lifecycle] PASS 2/6: serve member ready + advertised model: {served}");

    // Client verdict: discovery + join + first inference.
    let (which, seen) = expect_one_of(&client_lines, &["SEEN:", "NONE"], Duration::from_secs(900))?;
    anyhow::ensure!(
        which == "SEEN:",
        "LIFECYCLE FAIL: client member never saw the model via relay-driven join"
    );
    eprintln!("[lifecycle] PASS 3/6: client member discovered + joined via relay, sees: {seen}");
    let (which, detail) = expect_one_of(
        &client_lines,
        &["INFER_OK:", "INFER_FAIL:"],
        Duration::from_secs(180),
    )?;
    anyhow::ensure!(
        which == "INFER_OK:",
        "LIFECYCLE FAIL: client saw the model but inference did not route: {detail}"
    );
    eprintln!("[lifecycle] PASS 4/6: inference routed over the mesh: {detail:?}");

    // STRANGER child (C): must be denied by the relay's membership gate and
    // must not route inference through the mesh.
    eprintln!("[lifecycle] starting STRANGER (non-member, leaked endpoint)...");
    let mut stranger_child = Command::new(&exe)
        .env("MESH_ROLE", "stranger")
        .env("BUZZ_MEMBER_NSEC", secret_hex(&stranger))
        .env("MESH_OWNER_KEY", &stranger_key)
        .env("MESH_LEAKED_ENDPOINT", &endpoint)
        .env("HOME", role_home("stranger")?)
        .env("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR", &native_cache)
        .env("HF_HUB_CACHE", &hf_cache)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stranger_lines = spawn_line_reader(
        stranger_child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stranger stdout"))?,
    );
    let stranger_guard = KillOnDrop(&mut stranger_child);

    // Relay leg: only the relay's own membership rejection counts as denied.
    let (which, detail) = expect_one_of(
        &stranger_lines,
        &["RELAY_DENIED_MEMBERSHIP", "RELAY_AUTH_OK:", "RELAY_ERR:"],
        Duration::from_secs(120),
    )?;
    match which {
        "RELAY_DENIED_MEMBERSHIP" => {
            eprintln!("[lifecycle] PASS 5/6: relay rejected the stranger's NIP-42 auth (membership gate)");
        }
        "RELAY_AUTH_OK:" => anyhow::bail!(
            "LIFECYCLE FAIL: membership-gated relay authenticated a non-member (saw {detail} statuses)"
        ),
        _ => anyhow::bail!(
            "LIFECYCLE INCONCLUSIVE: stranger relay connect failed for a non-membership reason: {detail}"
        ),
    }

    // Mesh leg: the stranger must not complete an inference.
    let (which, detail) = expect_one_of(
        &stranger_lines,
        &["SEEN:", "NONE"],
        stranger_window() + Duration::from_secs(300),
    )?;
    let stranger_infer = if which == "SEEN:" {
        let model = detail;
        let (verdict, body) = expect_one_of(
            &stranger_lines,
            &["INFER_OK:", "INFER_FAIL:"],
            Duration::from_secs(180),
        )?;
        anyhow::ensure!(
            verdict != "INFER_OK:",
            "LIFECYCLE FAIL: stranger reused the leaked endpoint and inferred through {model}: {body:?}"
        );
        format!("saw gossip for {model} but inference was rejected: {body}")
    } else {
        "saw no routed model".to_string()
    };
    // Defuse the kill-guard (the stranger exits on its own after its verdict);
    // dropping it here would SIGKILL the child before we can read its status.
    std::mem::forget(stranger_guard);
    let stranger_status = wait_child(&mut stranger_child, Duration::from_secs(60), "stranger")?;
    anyhow::ensure!(
        stranger_status.success(),
        "LIFECYCLE INCONCLUSIVE: stranger child exited with {stranger_status}"
    );

    // Differential health proof: the trusted client must still route
    // inference *after* the stranger's attempt. Without this, a serve node
    // that died mid-run would make the stranger's failure look like a denial.
    client_stdin.write_all(format!("{VERIFY_AGAIN}\n").as_bytes())?;
    client_stdin.flush()?;
    let (which, detail) = expect_one_of(
        &client_lines,
        &["INFER_AGAIN_OK:", "INFER_AGAIN_FAIL:"],
        Duration::from_secs(180),
    )?;
    anyhow::ensure!(
        which == "INFER_AGAIN_OK:",
        "LIFECYCLE FAIL: trusted client could not infer after the stranger's attempt \
         (serve node unhealthy — stranger denial is inconclusive): {detail}"
    );
    eprintln!(
        "[lifecycle] PASS 6/6: stranger denied ({stranger_infer}) while trusted inference \
         still routes: {detail:?}"
    );

    eprintln!("[lifecycle] PASS: full relay-driven mesh lifecycle verified");
    drop(client_guard);
    let _ = wait_child(&mut client_child, Duration::from_secs(60), "client");
    drop(serve_guard);
    let _ = serve_child.wait();
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

// ── Child-process plumbing ───────────────────────────────────────────────────

/// Lines from a child's stdout, pumped by a dedicated reader thread so waits
/// can enforce hard deadlines (`BufRead::lines` alone blocks indefinitely).
struct ChildLines {
    rx: mpsc::Receiver<std::io::Result<String>>,
}

fn spawn_line_reader(stdout: ChildStdout) -> ChildLines {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    ChildLines { rx }
}

/// Wait (with a hard deadline) for a line starting with `prefix`; returns the
/// suffix. Non-matching lines are skipped.
fn expect_line(lines: &ChildLines, prefix: &str, timeout: Duration) -> anyhow::Result<String> {
    expect_one_of(lines, &[prefix], timeout).map(|(_, rest)| rest)
}

/// Wait (with a hard deadline) for a line starting with any of `prefixes`;
/// returns the matched prefix and the suffix.
fn expect_one_of<'a>(
    lines: &ChildLines,
    prefixes: &[&'a str],
    timeout: Duration,
) -> anyhow::Result<(&'a str, String)> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| anyhow::anyhow!("timed out waiting for one of {prefixes:?}"))?;
        match lines.rx.recv_timeout(remaining) {
            Ok(Ok(line)) => {
                for prefix in prefixes {
                    if let Some(rest) = line.strip_prefix(prefix) {
                        return Ok((prefix, rest.to_string()));
                    }
                }
            }
            Ok(Err(error)) => {
                anyhow::bail!("child stdout read error before {prefixes:?}: {error}")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                anyhow::bail!("timed out waiting for one of {prefixes:?}")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("child exited before printing one of {prefixes:?}")
            }
        }
    }
}

/// Wait for a child to exit, killing it if the deadline passes.
fn wait_child(child: &mut Child, timeout: Duration, label: &str) -> anyhow::Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("{label} child exceeded {timeout:?} and was killed");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Kill the child on drop so a failed assertion never leaks a process.
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
