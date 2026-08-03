#![deny(unsafe_code)]

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use buzz_core::kind::{KIND_EXECUTION_NODE_COMMAND, KIND_PAIRING};
use buzz_core::pairing::session::PairingSession;
use buzz_core::pairing::{qr::decode_qr, types::PayloadType, PairingError};
use buzz_core::tenant::relay_url_authority;
use buzz_node::{
    build_announcement_with_workloads_and_attestations, build_presence_event,
    parse_desktop_pairing_payload, DesktopPairingPayload, DockerSubstrate, DockerSubstrateConfig,
    ExecutionController, InertSubstrate, NodeConfig, NodeError, NodeIdentity, OwnerStore,
    ProcessSubstrate, ProcessSubstrateConfig, Substrate, WorkloadExit,
};
use clap::{Parser, Subcommand, ValueEnum};
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt;
use nostr::{Event, EventBuilder, EventId, RelayUrl};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::sleep;
use tracing::{info, warn};
use zeroize::Zeroizing;

#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};

/// How long to wait for each relay message during the interactive pairing
/// flow — Desktop-side SAS confirmation involves a human, so be generous.
const PAIRING_EVENT_TIMEOUT: Duration = Duration::from_secs(300);

/// How often a running node re-reads `owners.json` to pick up pairings
/// completed by a separate `buzz-node pair` process without a restart.
const OWNER_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// How often a connected node refreshes its kind:20001 presence heartbeat.
/// Matches the cadence members and managed agents use; the relay keeps
/// presence in Redis with a 180-second TTL, so 60 seconds keeps a healthy
/// node online with two missed ticks of margin.
const PRESENCE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Parser)]
#[command(
    name = "buzz-node",
    about = "Standalone relay-native Buzz execution node"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// Default agent body image run by the docker substrate. Built locally via
/// `just agent-image` (Dockerfile.agent); per-runtime variants are built via
/// `just agent-image goose|claude|codex` and resolved from this image's
/// repository (see the `--image` flag).
const DEFAULT_AGENT_IMAGE: &str = "buzz-agent:local";

#[derive(Debug, Subcommand)]
enum Command {
    /// Connect to the relay and publish the node announcement.
    Run {
        /// Workload substrate that runs deployed agent bodies. `process`
        /// spawns the sprig ACP harness as a supervised child process;
        /// `docker` runs each body as a container of the agent image;
        /// `inert` accepts commands without launching anything.
        #[arg(long, env = "BUZZ_NODE_SUBSTRATE", value_enum, default_value_t = SubstrateChoice::Process)]
        substrate: SubstrateChoice,
        /// Explicit path to the `buzz-acp` harness binary used by the
        /// process substrate. Defaults to a `buzz-acp` sibling of this
        /// executable, then `PATH` lookup.
        #[arg(long, env = "BUZZ_NODE_HARNESS_PATH")]
        harness_path: Option<std::path::PathBuf>,
        /// Agent body image used by the docker substrate for the bundled
        /// `buzz-agent` runtime and unknown runtimes (built from
        /// Dockerfile.agent, e.g. via `just agent-image`). Catalog runtimes
        /// with their own image variant (goose/claude/codex) run
        /// `<repository>:<runtime>` instead, derived by replacing this
        /// image's tag — `--image myrepo/buzz-agent:v3` resolves the goose
        /// runtime to `myrepo/buzz-agent:goose`. Images must already be
        /// present on the node (`just agent-image <runtime>`); the node
        /// never pulls or builds them.
        #[arg(long, env = "BUZZ_NODE_AGENT_IMAGE", default_value = DEFAULT_AGENT_IMAGE)]
        image: String,
        /// Docker CLI used by the docker substrate.
        #[arg(long, env = "BUZZ_NODE_DOCKER_PATH", default_value = "docker")]
        docker_path: std::path::PathBuf,
        /// Relay URL as reachable from inside agent containers. When absent,
        /// loopback relay hosts are rewritten to `host.docker.internal`.
        #[arg(long, env = "BUZZ_NODE_CONTAINER_RELAY_URL")]
        container_relay_url: Option<String>,
    },
    /// Complete an owner pairing session from a Desktop QR URI.
    Pair {
        /// Read the QR URI from this argument instead of stdin.
        #[arg(long)]
        qr: Option<String>,
    },
}

/// Substrate selection for `buzz-node run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SubstrateChoice {
    /// Supervised child processes running the sprig ACP harness.
    Process,
    /// Docker containers running the agent body image.
    Docker,
    /// No-op substrate: reconciliation bookkeeping only.
    Inert,
}

/// Options collected from the `run` subcommand (or its environment-variable
/// defaults).
#[derive(Debug)]
struct RunOptions {
    substrate: SubstrateChoice,
    harness_path: Option<std::path::PathBuf>,
    image: String,
    docker_path: std::path::PathBuf,
    container_relay_url: Option<String>,
}

/// Default `run` invocation used when no subcommand is given on the command
/// line. Reads the same environment variables as the clap definitions so
/// `BUZZ_NODE_SUBSTRATE` and its companions keep working without an explicit
/// `run`.
fn default_run_command() -> Result<Command> {
    let substrate = match std::env::var("BUZZ_NODE_SUBSTRATE") {
        Ok(value) => SubstrateChoice::from_str(value.trim(), true)
            .map_err(|error| anyhow::anyhow!("BUZZ_NODE_SUBSTRATE: {error}"))?,
        Err(_) => SubstrateChoice::Process,
    };
    Ok(Command::Run {
        substrate,
        harness_path: std::env::var_os("BUZZ_NODE_HARNESS_PATH").map(std::path::PathBuf::from),
        image: std::env::var("BUZZ_NODE_AGENT_IMAGE")
            .unwrap_or_else(|_| DEFAULT_AGENT_IMAGE.to_string()),
        docker_path: std::env::var_os("BUZZ_NODE_DOCKER_PATH").map_or_else(
            || std::path::PathBuf::from("docker"),
            std::path::PathBuf::from,
        ),
        container_relay_url: std::env::var("BUZZ_NODE_CONTAINER_RELAY_URL").ok(),
    })
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let result = match cli.command.map_or_else(default_run_command, Ok) {
        Ok(Command::Run {
            substrate,
            harness_path,
            image,
            docker_path,
            container_relay_url,
        }) => {
            run_node(RunOptions {
                substrate,
                harness_path,
                image,
                docker_path,
                container_relay_url,
            })
            .await
        }
        Ok(Command::Pair { qr }) => pair_node(qr).await,
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run_node(options: RunOptions) -> Result<()> {
    let config = NodeConfig::from_env()?;
    let identity = NodeIdentity::load_or_create(&config.data_dir)?;
    let mut owners = OwnerStore::load(&config.data_dir)?;
    let relay_authority = relay_url_authority(&config.relay_url);
    if relay_authority.is_empty() {
        bail!("configured relay URL has no valid authority");
    }
    // Keep an exit-channel sender alive for the inert case so the receiver
    // never reports closure while the connection loop is selecting on it.
    let (substrate, mut exit_events, _inert_exit_keepalive): (
        Arc<dyn Substrate>,
        tokio::sync::mpsc::UnboundedReceiver<WorkloadExit>,
        Option<tokio::sync::mpsc::UnboundedSender<WorkloadExit>>,
    ) = match options.substrate {
        SubstrateChoice::Process => {
            let mut substrate_config =
                ProcessSubstrateConfig::new(config.data_dir.clone(), config.relay_url.clone());
            substrate_config.harness_path = options.harness_path;
            let (substrate, exit_events) = ProcessSubstrate::new(substrate_config);
            (Arc::new(substrate), exit_events, None)
        }
        SubstrateChoice::Docker => {
            let mut substrate_config = DockerSubstrateConfig::new(
                config.data_dir.clone(),
                config.relay_url.clone(),
                options.image,
            );
            substrate_config.docker_path = options.docker_path;
            substrate_config.container_relay_url = options.container_relay_url;
            // Fail fast: refuse to announce a docker substrate the daemon
            // cannot honor.
            let (substrate, exit_events) = DockerSubstrate::connect(substrate_config)
                .await
                .context("initialize docker substrate")?;
            (Arc::new(substrate), exit_events, None)
        }
        SubstrateChoice::Inert => {
            let (exit_tx, exit_events) = tokio::sync::mpsc::unbounded_channel();
            (Arc::new(InertSubstrate), exit_events, Some(exit_tx))
        }
    };
    info!(substrate = ?options.substrate, "execution node substrate selected");
    let mut controller = ExecutionController::load_with_concurrency(
        &config.data_dir,
        config.max_concurrent_commands,
    )?
    .with_substrate(substrate);
    let relay_connected = Arc::new(AtomicBool::new(false));
    let health_listener = TcpListener::bind(config.health_addr)
        .await
        .with_context(|| format!("bind health listener on {}", config.health_addr))?;
    let health_task = tokio::spawn(serve_health(health_listener, relay_connected.clone()));
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let shutdown_task = tokio::spawn(async move {
        if let Err(error) = shutdown_signal().await {
            warn!(%error, "shutdown signal listener failed");
        }
        let _ = shutdown_tx.send(true);
    });
    let mut retry_delay = Duration::from_secs(1);

    let result = loop {
        let connection_shutdown = shutdown_rx.clone();
        tokio::select! {
            changed = shutdown_rx.changed() => {
                changed.context("shutdown signal listener closed")?;
                break Ok(());
            }
            result = run_connection(
                &config,
                &identity,
                &mut owners,
                &mut controller,
                &mut exit_events,
                &relay_authority,
                &relay_connected,
                connection_shutdown,
            ) => {
                relay_connected.store(false, Ordering::Release);
                if let Err(error) = result {
                    warn!(error = format!("{error:#}"), ?retry_delay, "execution node relay connection ended");
                    tokio::select! {
                        changed = shutdown_rx.changed() => {
                            changed.context("shutdown signal listener closed")?;
                            break Ok(());
                        }
                        _ = sleep(retry_delay) => {
                            retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                        }
                    }
                } else {
                    retry_delay = Duration::from_secs(1);
                }
            }
        }
    };
    relay_connected.store(false, Ordering::Release);
    shutdown_task.abort();
    health_task.abort();
    result
}

async fn shutdown_signal() -> io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate = signal(SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_connection(
    config: &NodeConfig,
    identity: &NodeIdentity,
    owners: &mut OwnerStore,
    controller: &mut ExecutionController,
    exit_events: &mut tokio::sync::mpsc::UnboundedReceiver<WorkloadExit>,
    relay_authority: &str,
    relay_connected: &AtomicBool,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    refresh_owner_store(owners, config);
    let mut connection = buzz_ws_client::NostrWsConnection::connect(&config.relay_url)
        .await
        .with_context(|| format!("connect to relay {}", config.relay_url))?;
    connection
        .authenticate(&identity.keys, config.auth_tag.as_ref())
        .await
        .context("authenticate with relay")?;

    let response =
        publish_node_announcement(&mut connection, controller, identity, owners, config).await?;
    if !response.accepted {
        bail!("relay rejected node announcement: {}", response.message);
    }
    // Establish presence immediately so Desktop sees the node as connected
    // before the first heartbeat tick fires.
    publish_node_presence(&mut connection, identity, "online").await?;

    let node_pubkey = identity.keys.public_key().to_hex();
    connection
        .send_raw(&serde_json::json!([
            "REQ",
            "execution-node",
            { "kinds": [KIND_EXECUTION_NODE_COMMAND], "#p": [node_pubkey] }
        ]))
        .await
        .context("subscribe to execution commands")?;
    connection
        .wait_for_eose("execution-node", Duration::from_secs(10))
        .await
        .context("wait for execution subscription EOSE")?;
    info!(node = %identity.keys.public_key().to_hex(), "execution node connected");
    relay_connected.store(true, Ordering::Release);

    let mut owner_refresh = tokio::time::interval(OWNER_REFRESH_INTERVAL);
    owner_refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Start one full interval out — the initial "online" above already covers
    // the first window.
    let mut presence_heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + PRESENCE_HEARTBEAT_INTERVAL,
        PRESENCE_HEARTBEAT_INTERVAL,
    );
    presence_heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut work = FuturesUnordered::new();
    loop {
        tokio::select! {
            _ = presence_heartbeat.tick() => {
                publish_node_presence(&mut connection, identity, "online").await?;
            }
            _ = owner_refresh.tick() => {
                if refresh_owner_store(owners, config) {
                    let response =
                        publish_node_announcement(&mut connection, controller, identity, owners, config)
                            .await
                            .context("publish refreshed node announcement")?;
                    if !response.accepted {
                        warn!(message = %response.message, "relay rejected refreshed node announcement");
                    }
                }
            }
            // A workload body exited on its own: it was finished, not killed.
            // Record the outcome in the durable ledger (never respawn) and
            // re-announce so paired Desktops observe the new lifecycle. The
            // channel cannot report closure while this loop runs — the
            // controller keeps the substrate (and its sender) alive, and the
            // inert case parks a keepalive sender in `run_node`.
            Some(exit) = exit_events.recv() => {
                info!(
                    workload = exit.workload_id.as_str(),
                    clean = exit.clean,
                    "workload body exited on its own"
                );
                match controller.record_workload_exit(&exit).await {
                    Ok(true) => {
                        let response =
                            publish_node_announcement(&mut connection, controller, identity, owners, config)
                                .await
                                .context("publish node announcement after workload exit")?;
                        if !response.accepted {
                            warn!(message = %response.message, "relay rejected post-exit node announcement");
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        warn!(%error, workload = exit.workload_id.as_str(), "failed to record workload exit");
                    }
                }
            }
            changed = shutdown.changed() => {
                changed.context("shutdown signal listener closed")?;
                while let Some(result) = work.next().await {
                    publish_work_result(&mut connection, controller, identity, owners, config, result).await?;
                }
                // Best-effort: the relay also clears presence on the clean
                // disconnect below, so a failure here only delays the flip
                // until the Redis TTL expires.
                if let Err(error) = publish_node_presence(&mut connection, identity, "offline").await {
                    warn!(error = format!("{error:#}"), "failed to publish offline presence");
                }
                connection
                    .disconnect()
                    .await
                    .context("close relay connection")?;
                return Ok(());
            }
            result = connection.next_event(Duration::from_secs(300)) => match result {
                Ok(buzz_ws_client::RelayMessage::Event { event, .. }) => {
                    if work.len() >= config.max_concurrent_commands {
                        if let Some(result) = work.next().await {
                            publish_work_result(&mut connection, controller, identity, owners, config, result)
                                .await?;
                        }
                    }
                    let controller = controller.clone();
                    let identity = identity.clone();
                    let owners = owners.clone();
                    let relay_authority = relay_authority.to_string();
                    work.push(tokio::spawn(async move {
                        controller
                            .handle_command_event(
                                &identity,
                                &owners,
                                &relay_authority,
                                &event,
                                chrono::Utc::now(),
                            )
                            .await
                            .map(|receipts| (event.id, receipts))
                    }));
                }
                Ok(buzz_ws_client::RelayMessage::Closed { message, .. }) => {
                    while work.next().await.is_some() {}
                    bail!("execution subscription closed: {message}");
                }
                Ok(_) => {}
                Err(error) => {
                    while work.next().await.is_some() {}
                    return Err(anyhow::Error::new(error).context("receive relay message"));
                }
            },
            Some(result) = work.next(), if !work.is_empty() => {
                publish_work_result(&mut connection, controller, identity, owners, config, result).await?;
            }
        }
    }
}

/// Reload the owner store from disk, replacing `owners` when a separate
/// `buzz-node pair` process persisted a new pairing. Returns whether the
/// store changed; load failures keep the current store and are logged.
fn refresh_owner_store(owners: &mut OwnerStore, config: &NodeConfig) -> bool {
    match owners.reload_if_changed(&config.data_dir) {
        Ok(Some(latest)) => {
            *owners = latest;
            info!(
                owner_count = owners.owners().len(),
                "owner store changed on disk; refreshing node announcement with updated attestations"
            );
            true
        }
        Ok(None) => false,
        Err(error) => {
            warn!(%error, "failed to reload owner store from disk");
            false
        }
    }
}

/// Build and publish the node's replaceable announcement (NIP-33 LWW on the
/// node's `d` tag) reflecting current workloads and owner attestations.
async fn publish_node_announcement(
    connection: &mut buzz_ws_client::NostrWsConnection,
    controller: &ExecutionController,
    identity: &NodeIdentity,
    owners: &OwnerStore,
    config: &NodeConfig,
) -> Result<buzz_ws_client::OkResponse> {
    let workloads = controller.workload_statuses().await;
    let announcement = build_announcement_with_workloads_and_attestations(
        identity,
        &config.display_name,
        &workloads,
        owners.attestations(),
    )?;
    connection
        .send_event(announcement)
        .await
        .context("publish node announcement")
}

/// Publish the node's ephemeral kind:20001 presence heartbeat over the
/// relay WebSocket (the HTTP bridge rejects ephemeral kinds).
///
/// Transport failures propagate so the connection loop reconnects; a relay
/// rejection is only logged because presence is best-effort and the next
/// heartbeat tick retries within the Redis presence TTL.
async fn publish_node_presence(
    connection: &mut buzz_ws_client::NostrWsConnection,
    identity: &NodeIdentity,
    status: &str,
) -> Result<()> {
    let event = build_presence_event(identity, status)?;
    let response = connection
        .send_event(event)
        .await
        .context("publish node presence")?;
    if !response.accepted {
        warn!(status, message = %response.message, "relay rejected node presence update");
    }
    Ok(())
}

async fn publish_work_result(
    connection: &mut buzz_ws_client::NostrWsConnection,
    controller: &ExecutionController,
    identity: &NodeIdentity,
    owners: &OwnerStore,
    config: &NodeConfig,
    result: Result<Result<(EventId, Vec<Event>), NodeError>, tokio::task::JoinError>,
) -> Result<()> {
    let (event_id, receipts) = match result {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            warn!(%error, "execution command rejected");
            return Ok(());
        }
        Err(error) => {
            warn!(%error, "execution command task failed");
            return Ok(());
        }
    };
    for receipt in receipts {
        let response = connection
            .send_event(receipt)
            .await
            .context("publish execution receipt")?;
        if !response.accepted {
            warn!(event_id = %response.event_id, message = %response.message, "relay rejected execution receipt");
        }
    }
    let response = publish_node_announcement(connection, controller, identity, owners, config)
        .await
        .context("publish node status announcement")?;
    if !response.accepted {
        warn!(event_id = %event_id, message = %response.message, "relay rejected execution-node status announcement");
    }
    Ok(())
}

async fn serve_health(listener: TcpListener, relay_connected: Arc<AtomicBool>) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let relay_connected = relay_connected.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_health_request(stream, relay_connected).await {
                warn!(%error, "health probe failed");
            }
        });
    }
}

async fn handle_health_request(
    mut stream: tokio::net::TcpStream,
    relay_connected: Arc<AtomicBool>,
) -> io::Result<()> {
    let mut request = [0_u8; 1024];
    let bytes = stream.read(&mut request).await?;
    let request = String::from_utf8_lossy(&request[..bytes]);
    let path = request.split_whitespace().nth(1).unwrap_or("/");
    let (status, body) = match path {
        "/health" | "/healthz" | "/_liveness" => ("200 OK", r#"{"status":"ok"}"#),
        "/ready" | "/readiness" | "/_readiness" if relay_connected.load(Ordering::Acquire) => {
            ("200 OK", r#"{"status":"ready","relayConnected":true}"#)
        }
        "/ready" | "/readiness" | "/_readiness" => (
            "503 Service Unavailable",
            r#"{"status":"starting","relayConnected":false}"#,
        ),
        _ => ("404 Not Found", r#"{"status":"not_found"}"#),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await
}

async fn pair_node(qr_arg: Option<String>) -> Result<()> {
    let config = NodeConfig::from_env()?;
    let identity = NodeIdentity::load_or_create(&config.data_dir)?;
    let qr_uri = match qr_arg {
        Some(value) => value,
        None => {
            print!("Paste the Desktop pairing QR URI: ");
            io::stdout().flush()?;
            read_line()?
        }
    };
    let qr = decode_qr(qr_uri.trim()).context("invalid pairing QR URI")?;
    let relay_url = qr
        .relays
        .first()
        .context("QR URI contains no relay URL")?
        .clone();
    let (mut session, offer) =
        PairingSession::new_target(&qr).context("initialize pairing session")?;
    let mut connection = connect_pairing_relay(&session, &relay_url).await?;

    connection
        .send_raw(&serde_json::json!([
            "REQ",
            "pair",
            { "kinds": [KIND_PAIRING], "#p": [session.pubkey().to_hex()] }
        ]))
        .await
        .context("subscribe to pairing events")?;
    connection
        .wait_for_eose("pair", Duration::from_secs(10))
        .await
        .context("wait for pairing subscription EOSE")?;
    publish_pairing_event(&mut connection, offer).await?;

    let sas = session.sas_code().context("pairing SAS was not derived")?;
    println!("Pairing offer sent. SAS code: {sas}");
    print!("Does Desktop show the same SAS? [y/n]: ");
    io::stdout().flush()?;

    loop {
        let event = next_pairing_event(&mut connection, "pair").await?;
        if session.handle_abort(&event).is_ok() {
            bail!("Desktop aborted pairing");
        }
        match session.handle_sas_confirm(&event) {
            Ok(_) => break,
            Err(PairingError::TranscriptMismatch) => {
                bail!("pairing transcript mismatch");
            }
            Err(_) => {}
        }
    }

    if !read_yes_no()? {
        bail!("SAS mismatch — pairing aborted");
    }
    session
        .confirm_target_sas()
        .context("confirm pairing SAS")?;

    let payload: Zeroizing<String> = loop {
        let event = next_pairing_event(&mut connection, "pair").await?;
        match session.handle_payload(&event) {
            Ok((PayloadType::Custom, payload)) => break payload,
            Ok(_) => bail!("expected custom pairing payload"),
            Err(_) => {}
        }
    };
    let DesktopPairingPayload {
        owner_pubkey,
        relay_url,
        mut nsec,
    } = parse_desktop_pairing_payload(&payload)?;
    let expected_relay_authority = relay_url_authority(&config.relay_url);
    if expected_relay_authority.is_empty()
        || relay_url_authority(&relay_url) != expected_relay_authority
    {
        bail!("pairing payload relay does not match the configured relay");
    }
    let mut nsec = nsec.take().context("pairing payload is missing nsec")?;
    let owner_keys = match nostr::Keys::parse(&nsec) {
        Ok(keys) => keys,
        Err(error) => {
            zeroize::Zeroize::zeroize(&mut nsec);
            return Err(anyhow::Error::new(error).context("parse pairing payload nsec"));
        }
    };
    zeroize::Zeroize::zeroize(&mut nsec);
    if owner_keys.public_key().to_hex() != owner_pubkey {
        bail!("pairing payload owner identity does not match nsec");
    }
    let node_id = identity.node_id()?;
    let node_attestation = buzz_core::execution::ExecutionNodeAttestation::sign(
        &owner_keys,
        &node_id,
        expected_relay_authority.clone(),
    )?;
    let mut owners = OwnerStore::load(&config.data_dir)?;
    owners.add_attestation(
        node_attestation,
        &node_id,
        &expected_relay_authority,
        &config.data_dir,
    )?;
    let complete = session
        .send_complete()
        .context("build pairing complete event")?;
    publish_pairing_event(&mut connection, complete).await?;
    println!(
        "Paired owner {} with node {}.",
        owner_pubkey,
        identity.keys.public_key()
    );
    println!(
        "A running `buzz-node run` process picks up the new pairing within {} seconds and re-announces automatically; no restart needed.",
        OWNER_REFRESH_INTERVAL.as_secs()
    );
    Ok(())
}

/// Connects to the pairing relay and completes NIP-42 authentication with the
/// ephemeral pairing session key instead of the node's own identity.
async fn connect_pairing_relay(
    session: &PairingSession,
    relay_url: &str,
) -> Result<buzz_ws_client::NostrWsConnection> {
    let mut connection = buzz_ws_client::NostrWsConnection::connect(relay_url)
        .await
        .with_context(|| format!("connect to pairing relay {relay_url}"))?;
    let challenge = connection
        .auth_challenge(Duration::from_secs(
            buzz_ws_client::connection::AUTH_CHALLENGE_TIMEOUT_SECS,
        ))
        .await
        .context("wait for relay AUTH challenge")?;
    let relay = RelayUrl::parse(relay_url).context("parse pairing relay URL")?;
    let auth = session
        .sign_event(EventBuilder::auth(challenge, relay))
        .context("sign pairing AUTH event")?;
    connection
        .authenticate_with_event(auth)
        .await
        .context("authenticate pairing session with relay")?;
    Ok(connection)
}

async fn next_pairing_event(
    connection: &mut buzz_ws_client::NostrWsConnection,
    subscription: &str,
) -> Result<Event> {
    loop {
        match connection
            .next_event(PAIRING_EVENT_TIMEOUT)
            .await
            .context("receive pairing event")?
        {
            buzz_ws_client::RelayMessage::Event {
                subscription_id,
                event,
            } if subscription_id == subscription => return Ok(*event),
            buzz_ws_client::RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == subscription => {
                bail!("pairing subscription closed: {message}");
            }
            _ => {}
        }
    }
}

async fn publish_pairing_event(
    connection: &mut buzz_ws_client::NostrWsConnection,
    event: Event,
) -> Result<()> {
    let response = connection
        .send_event(event)
        .await
        .context("publish pairing event")?;
    if !response.accepted {
        bail!("relay rejected pairing event: {}", response.message);
    }
    Ok(())
}

fn read_line() -> io::Result<String> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    Ok(line)
}

fn read_yes_no() -> io::Result<bool> {
    Ok(read_line()?.trim().eq_ignore_ascii_case("y"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;

    async fn probe(address: std::net::SocketAddr, path: &str) -> String {
        let mut stream = TcpStream::connect(address).await.expect("health listener");
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .await
            .expect("health request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("health response");
        String::from_utf8(response).expect("health response utf8")
    }

    #[tokio::test]
    async fn health_endpoints_distinguish_liveness_and_relay_readiness() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("health address");
        let address = listener.local_addr().expect("health local address");
        let connected = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(serve_health(listener, connected.clone()));

        assert!(probe(address, "/_liveness")
            .await
            .starts_with("HTTP/1.1 200"));
        assert!(probe(address, "/_readiness")
            .await
            .starts_with("HTTP/1.1 503"));
        connected.store(true, Ordering::Release);
        let ready = probe(address, "/_readiness").await;
        assert!(ready.starts_with("HTTP/1.1 200"));
        assert!(ready.contains("relayConnected"));

        task.abort();
    }
}
