//! Owner-allowlist admission smoke test — three nodes, one per process.
//!
//! Proves the security claim of Buzz's membership-derived mesh admission:
//! possession of a dial pointer (invite token / EndpointAddr) admits nobody;
//! only owners on the serve node's allowlist join the mesh.
//!
//!   1. SERVE process: hosts a GGUF with `trust_policy(Allowlist)` +
//!      `owner_required(true)`, trusting exactly one other owner id — the
//!      same builder calls Buzz desktop makes with a resolved member roster.
//!   2. TRUSTED process: client presenting the allowlisted owner key, joins
//!      via the invite token, must see the routed model; the orchestrator
//!      then drives a real inference through it.
//!   3. NON-MEMBER process: client presenting a different owner key with the
//!      same connection information. It must not be admitted or route inference.
//!
//! One process per node is load-bearing: mesh-llm keeps process-global state
//! (ownership attestation at ~/.mesh-llm/node-ownership.json, tracing, the
//! output sink), so multiple owner-keyed embedded nodes in one process
//! corrupt each other — exactly how Buzz runs it in production anyway (one
//! desktop = one node).
//!
//! Hardware-gated, not CI — loads a real model. Run with:
//!   cargo run -p buzz-relay --example mesh_admission_smoke
use std::io::BufRead;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use mesh_llm_host_runtime::crypto::{save_keystore, OwnerKeypair};
use mesh_llm_sdk::{client, serve, MeshDiscoveryMode, TrustPolicy};

const DEFAULT_MODEL: &str = "jc-builds/SmolLM2-135M-Instruct-Q4_K_M-GGUF:Q4_K_M";

const SERVE_API_PORT: u16 = 19347;
const SERVE_CONSOLE_PORT: u16 = 13141;
const TRUSTED_API_PORT: u16 = 19348;
const TRUSTED_CONSOLE_PORT: u16 = 13142;
const STRANGER_API_PORT: u16 = 19349;
const STRANGER_CONSOLE_PORT: u16 = 13143;

/// The trusted client sees the model within seconds on one box; this bounds
/// the stranger's chance to (fail to) see it.
const TRUSTED_WINDOW_SECS: u64 = 120;
const STRANGER_WINDOW_SECS: u64 = 45;

fn main() -> anyhow::Result<()> {
    match std::env::var("MESH_ROLE").ok().as_deref() {
        Some("serve") => tokio::runtime::Runtime::new()?.block_on(role_serve()),
        Some("client") => tokio::runtime::Runtime::new()?.block_on(role_client()),
        _ => orchestrate(),
    }
}

fn env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("{name} is required for this role"))
}

async fn init_native_runtime() -> anyhow::Result<()> {
    let cache = mesh_llm_sdk::native_runtime::native_runtime_cache(None)?;
    let current = mesh_llm_sdk::native_runtime::CURRENT_MESH_VERSION;
    if !cache
        .installed()?
        .iter()
        .any(|runtime| runtime.mesh_version == current)
    {
        anyhow::bail!("MeshLLM native runtime for MeshLLM {current} is not installed; run `just mesh-e2e-hardware` once to prepare it");
    }
    mesh_llm_host_runtime::initialize_host_runtime()
        .await
        .map_err(|error| anyhow::anyhow!("MeshLLM host runtime init failed: {error}"))
}

/// SERVE role: allowlist serve node. Prints `INVITE:<token>` then
/// `READY:<model>` on stdout, then parks until the orchestrator kills it.
async fn role_serve() -> anyhow::Result<()> {
    init_native_runtime().await?;
    let model = env("MESH_SMOKE_MODEL")?;
    let owner_key = env("MESH_OWNER_KEY")?;
    let trust_owners: Vec<String> = env("MESH_TRUST_OWNERS")?
        .split(',')
        .map(str::to_string)
        .collect();

    let cfg = serve::EmbeddedServeConfig::builder()
        .model(&model)
        .api_port(SERVE_API_PORT)
        .console_port(SERVE_CONSOLE_PORT)
        .publish(true)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Mdns)
        .console_ui(true)
        .startup_timeout(Duration::from_secs(600))
        .owner_key(owner_key)
        .owner_required(true)
        .trust_policy(TrustPolicy::Allowlist)
        .trust_owners(trust_owners)
        // Require signed bootstrap tokens so owner admission is enforced from
        // the first connection attempt.
        .signed_join_tokens(true)
        .build();
    let node = serve::start(cfg).await?;
    let invite = node
        .invite_token()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("serve node produced no invite token"))?;
    println!("INVITE:{invite}");

    let http = reqwest::Client::new();
    let base = node.api_base_url().to_string();
    match wait_for_model(&http, &base, Duration::from_secs(600)).await? {
        Some(model) => println!("READY:{model}"),
        None => anyhow::bail!("serve node never loaded the model"),
    }
    // Park; the orchestrator kills this process when the run is over.
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

/// CLIENT role (trusted or stranger — the key decides). Joins via the invite
/// token, polls its own /models for the window, prints `SEEN:<model>` or
/// `NONE`, stops the node, exits 0.
async fn role_client() -> anyhow::Result<()> {
    init_native_runtime().await?;
    let owner_key = env("MESH_OWNER_KEY")?;
    let join_token = env("MESH_JOIN_TOKEN")?;
    let api_port: u16 = env("MESH_API_PORT")?.parse()?;
    let console_port: u16 = env("MESH_CONSOLE_PORT")?.parse()?;
    let window_secs: u64 = env("MESH_WINDOW_SECS")?.parse()?;

    let cfg = client::EmbeddedClientConfig::builder()
        .api_port(api_port)
        .console_port(console_port)
        .publish(false)
        .auto_join(false)
        .discovery_mode(MeshDiscoveryMode::Mdns)
        .join_token(&join_token)
        .startup_timeout(Duration::from_secs(180))
        .console_ui(true)
        // Present the owner attestation; owner_required makes a key-load
        // failure abort loudly instead of silently starting unattested
        // (which the allowlist serve would then reject as NoAttestation).
        .owner_key(owner_key)
        .owner_required(true)
        .build();
    let node = client::start(cfg).await?;
    let http = reqwest::Client::new();
    let base = node.api_base_url().to_string();
    let seen = wait_for_model(&http, &base, Duration::from_secs(window_secs)).await?;
    match &seen {
        Some(model) => {
            println!("SEEN:{model}");
            // Visibility is gossip; admission is routing. The decisive probe
            // is whether an inference actually routes through the mesh.
            match try_completion(&http, &base, model).await {
                Ok(content) => println!("INFER_OK:{content}"),
                Err(error) => println!("INFER_FAIL:{error}"),
            }
        }
        None => println!("NONE"),
    }
    let _ = node.stop().await;
    // Skip C++ static destructors (ggml Metal aborts in global teardown).
    std::process::exit(0);
}

/// Orchestrator: keystores, three child processes, assertions, inference.
fn orchestrate() -> anyhow::Result<()> {
    let model = std::env::var("MESH_SMOKE_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    eprintln!("[admission] model: {model}");

    let scratch = std::env::temp_dir().join(format!("buzz-mesh-admission-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)?;
    let make_owner = |name: &str| -> anyhow::Result<(String, String)> {
        let keypair = OwnerKeypair::generate();
        let path = scratch.join(format!("{name}.keystore.json"));
        save_keystore(&path, &keypair, None, true)
            .map_err(|error| anyhow::anyhow!("saving {name} keystore: {error}"))?;
        Ok((path.display().to_string(), keypair.owner_id()))
    };
    let (serve_key, serve_owner) = make_owner("serve")?;
    let (trusted_key, trusted_owner) = make_owner("trusted")?;
    let (stranger_key, stranger_owner) = make_owner("stranger")?;
    eprintln!("[admission] owners — serve: {serve_owner}, trusted: {trusted_owner}, stranger: {stranger_owner}");

    // Each role gets an isolated HOME: mesh-llm keeps its node endpoint key
    // and node-ownership.json under ~/.mesh-llm, so subprocesses sharing the
    // real HOME would share a node identity and clobber each other's
    // attestations. The native runtime cache must still point at the real
    // one (it resolves via HOME otherwise).
    let real_cache = std::env::var_os("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or(dirs_cache_dir()?.join("mesh-llm/native-runtimes"));
    let role_home = |name: &str| -> anyhow::Result<String> {
        let home = scratch.join(format!("{name}-home"));
        std::fs::create_dir_all(&home)?;
        Ok(home.display().to_string())
    };
    let serve_home = role_home("serve")?;
    let trusted_home = role_home("trusted")?;
    let stranger_home = role_home("stranger")?;

    let exe = std::env::current_exe()?;

    // 1. Serve child with allowlist {serve, trusted}.
    eprintln!("[admission] starting allowlist serve node (subprocess)...");
    let mut serve_child = Command::new(&exe)
        .env("MESH_ROLE", "serve")
        .env("MESH_SMOKE_MODEL", &model)
        .env("MESH_OWNER_KEY", &serve_key)
        .env("HOME", &serve_home)
        .env("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR", &real_cache)
        .env(
            "MESH_TRUST_OWNERS",
            format!("{serve_owner},{trusted_owner}"),
        )
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
    let invite = expect_line(&mut serve_lines, "INVITE:", Duration::from_secs(180))?;
    eprintln!("[admission] serve invite token acquired");
    let served_model = expect_line(&mut serve_lines, "READY:", Duration::from_secs(600))?;
    eprintln!("[admission] serve model ready: {served_model}");

    // 2. Trusted client child: allowlisted owner + the invite token.
    eprintln!("[admission] starting TRUSTED client (allowlisted owner)...");
    let trusted_out = run_client(
        &exe,
        ClientRun {
            owner_key: &trusted_key,
            home: &trusted_home,
            cache_dir: &real_cache,
            invite: &invite,
            api_port: TRUSTED_API_PORT,
            console_port: TRUSTED_CONSOLE_PORT,
            window_secs: TRUSTED_WINDOW_SECS,
        },
    )?;
    let routed = match trusted_out.seen.as_deref() {
        Some(model) => model.to_string(),
        None => anyhow::bail!("ADMISSION FAIL: trusted (allowlisted) client never saw the model"),
    };
    eprintln!("[admission] PASS 1/3: trusted client admitted, sees routed model: {routed}");
    let content = match trusted_out.infer_ok.as_deref() {
        Some(content) => content.to_string(),
        None => anyhow::bail!(
            "ADMISSION FAIL: trusted client saw the model but inference did not route: {:?}",
            trusted_out.infer_fail
        ),
    };
    eprintln!("[admission] PASS 2/3: trusted client inference routed over mesh: {content:?}");
    let _ = served_model; // serve-side id retained for logs only

    // 4. Non-member child: same connection information, non-allowlisted owner key.
    eprintln!("[admission] starting NON-MEMBER client (owner is not allowlisted)...");
    let stranger_out = run_client(
        &exe,
        ClientRun {
            owner_key: &stranger_key,
            home: &stranger_home,
            cache_dir: &real_cache,
            invite: &invite,
            api_port: STRANGER_API_PORT,
            console_port: STRANGER_CONSOLE_PORT,
            window_secs: STRANGER_WINDOW_SECS,
        },
    )?;
    match (
        stranger_out.seen.as_deref(),
        stranger_out.infer_ok.as_deref(),
        stranger_out.infer_fail.as_deref(),
    ) {
        (None, None, _) => eprintln!(
            "[admission] PASS 3/3: non-member saw no model ({STRANGER_WINDOW_SECS}s window)"
        ),
        (Some(model), None, Some(error)) => eprintln!(
            "[admission] PASS 3/3: non-member saw gossip for {model} but inference was rejected: {error}"
        ),
        (Some(model), Some(content), _) => anyhow::bail!(
            "ADMISSION FAIL: non-member reused the invite token and inferred through {model}: {content:?}"
        ),
        (Some(model), None, None) => anyhow::bail!(
            "ADMISSION INCONCLUSIVE: non-member saw {model} but produced no inference verdict"
        ),
        (None, Some(content), _) => anyhow::bail!(
            "ADMISSION FAIL: non-member inferred without model visibility: {content:?}"
        ),
    }
    eprintln!("[admission] PASS: owner allowlist gates mesh membership and inference");

    drop(serve_guard); // kills the serve child
    let _ = serve_child.wait();
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
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
        .timeout(Duration::from_secs(60))
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

/// The real user's OS cache dir (macOS: ~/Library/Caches), resolved before
/// we override HOME for the child processes.
fn dirs_cache_dir() -> anyhow::Result<std::path::PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME is not set"))?;
    #[cfg(target_os = "macos")]
    return Ok(std::path::PathBuf::from(home).join("Library/Caches"));
    #[cfg(not(target_os = "macos"))]
    return Ok(std::path::PathBuf::from(home).join(".cache"));
}

struct ClientRun<'a> {
    owner_key: &'a str,
    home: &'a str,
    cache_dir: &'a std::path::Path,
    invite: &'a str,
    api_port: u16,
    console_port: u16,
    window_secs: u64,
}

/// Spawn a client-role child and return its verdict line (`SEEN:…` / `NONE`).
fn run_client(exe: &std::path::Path, run: ClientRun<'_>) -> anyhow::Result<ClientVerdict> {
    let output = Command::new(exe)
        .env("MESH_ROLE", "client")
        .env("MESH_OWNER_KEY", run.owner_key)
        .env("HOME", run.home)
        .env("MESH_LLM_NATIVE_RUNTIME_CACHE_DIR", run.cache_dir)
        .env("MESH_JOIN_TOKEN", run.invite)
        .env("MESH_API_PORT", run.api_port.to_string())
        .env("MESH_CONSOLE_PORT", run.console_port.to_string())
        .env("MESH_WINDOW_SECS", run.window_secs.to_string())
        .stderr(Stdio::inherit())
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut verdict = ClientVerdict::default();
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
        anyhow::bail!("client child produced no verdict; stdout: {stdout}");
    }
    Ok(verdict)
}

/// What a client-role child reported on stdout.
#[derive(Debug, Default)]
struct ClientVerdict {
    /// Model id if the routed model became visible in the window.
    seen: Option<String>,
    /// Completion content if an inference actually routed over the mesh.
    infer_ok: Option<String>,
    /// Inference error if visibility existed but routing failed.
    infer_fail: Option<String>,
}

/// Read serve-child stdout until a line with the given prefix appears.
fn expect_line(
    lines: &mut std::io::Lines<std::io::BufReader<std::process::ChildStdout>>,
    prefix: &str,
    timeout: Duration,
) -> anyhow::Result<String> {
    // BufReader::lines blocks; enforce the timeout coarsely via a deadline
    // check between lines (the child prints continuously enough in practice).
    let deadline = std::time::Instant::now() + timeout;
    for line in lines.by_ref() {
        let line = line?;
        if let Some(rest) = line.strip_prefix(prefix) {
            return Ok(rest.to_string());
        }
        if std::time::Instant::now() > deadline {
            break;
        }
    }
    anyhow::bail!("serve child ended or timed out before printing {prefix}")
}

/// Kill the serve child on drop so a failed assertion never leaks a process.
struct KillOnDrop<'a>(&'a mut Child);
impl Drop for KillOnDrop<'_> {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

/// Poll `/models` until a model id appears or the window closes.
async fn wait_for_model(
    http: &reqwest::Client,
    api_base: &str,
    window: Duration,
) -> anyhow::Result<Option<String>> {
    let url = format!("{api_base}/models");
    let deadline = std::time::Instant::now() + window;
    while std::time::Instant::now() < deadline {
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
