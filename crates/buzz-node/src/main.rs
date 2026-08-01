use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use buzz_core::kind::{KIND_EXECUTION_NODE_COMMAND, KIND_PAIRING};
use buzz_core::pairing::session::PairingSession;
use buzz_core::pairing::{qr::decode_qr, types::PayloadType, PairingError};
use buzz_node::{
    build_announcement_with_workloads, parse_desktop_pairing_payload, DesktopPairingPayload,
    ExecutionController, NodeConfig, NodeError, NodeIdentity, OwnerStore,
};
use clap::{Parser, Subcommand};
use futures_util::stream::FuturesUnordered;
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, EventId, RelayUrl};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use zeroize::Zeroizing;

#[cfg(unix)]
use tokio::signal::unix::{signal, SignalKind};

#[derive(Debug, Parser)]
#[command(
    name = "buzz-node",
    about = "Standalone relay-native Buzz execution node"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Connect to the relay and publish the node announcement.
    Run,
    /// Complete an owner pairing session from a Desktop QR URI.
    Pair {
        /// Read the QR URI from this argument instead of stdin.
        #[arg(long)]
        qr: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Run);
    let result = match command {
        Command::Run => run_node().await,
        Command::Pair { qr } => pair_node(qr).await,
    };
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run_node() -> Result<(), NodeError> {
    let config = NodeConfig::from_env()?;
    let identity = NodeIdentity::load_or_create(&config.data_dir)?;
    let owners = OwnerStore::load(&config.data_dir)?;
    let mut controller = ExecutionController::load_with_concurrency(
        &config.data_dir,
        config.max_concurrent_commands,
    )?;
    let relay_connected = Arc::new(AtomicBool::new(false));
    let health_listener = TcpListener::bind(config.health_addr)
        .await
        .map_err(|error| NodeError::InvalidConfiguration(format!("health listener: {error}")))?;
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
                changed.map_err(|_| NodeError::InvalidConfiguration("shutdown signal closed".into()))?;
                break Ok(());
            }
            result = run_connection(
                &config,
                &identity,
                &owners,
                &mut controller,
                &relay_connected,
                connection_shutdown,
            ) => {
                relay_connected.store(false, Ordering::Release);
                if let Err(error) = result {
                    warn!(%error, ?retry_delay, "execution node relay connection ended");
                    tokio::select! {
                        changed = shutdown_rx.changed() => {
                            changed.map_err(|_| NodeError::InvalidConfiguration("shutdown signal closed".into()))?;
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

async fn run_connection(
    config: &NodeConfig,
    identity: &NodeIdentity,
    owners: &OwnerStore,
    controller: &mut ExecutionController,
    relay_connected: &AtomicBool,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), NodeError> {
    let mut connection = buzz_ws_client::NostrWsConnection::connect(&config.relay_url)
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    connection
        .authenticate(&identity.keys, config.auth_tag.as_ref())
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;

    let workloads = controller.workload_statuses().await;
    let announcement =
        build_announcement_with_workloads(identity, &config.display_name, &workloads)?;
    let response = connection
        .send_event(announcement)
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    if !response.accepted {
        return Err(NodeError::InvalidConfiguration(format!(
            "relay rejected node announcement: {}",
            response.message
        )));
    }

    let node_pubkey = identity.keys.public_key().to_hex();
    connection
        .send_raw(&serde_json::json!([
            "REQ",
            "execution-node",
            { "kinds": [KIND_EXECUTION_NODE_COMMAND], "#p": [node_pubkey] }
        ]))
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    connection
        .wait_for_eose("execution-node", Duration::from_secs(10))
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    info!(node = %identity.keys.public_key().to_hex(), "execution node connected");
    relay_connected.store(true, Ordering::Release);

    let mut work = FuturesUnordered::new();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                changed.map_err(|_| NodeError::InvalidConfiguration("shutdown signal closed".into()))?;
                while let Some(result) = work.next().await {
                    publish_work_result(&mut connection, controller, identity, config, result).await?;
                }
                connection
                    .disconnect()
                    .await
                    .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
                return Ok(());
            }
            result = connection.next_event(Duration::from_secs(300)) => match result {
                Ok(buzz_ws_client::RelayMessage::Event { event, .. }) => {
                    if work.len() >= config.max_concurrent_commands {
                        if let Some(result) = work.next().await {
                            publish_work_result(&mut connection, controller, identity, config, result)
                                .await?;
                        }
                    }
                    let controller = controller.clone();
                    let identity = identity.clone();
                    let owners = owners.clone();
                    work.push(tokio::spawn(async move {
                        controller
                            .handle_command_event(&identity, &owners, &event, chrono::Utc::now())
                            .await
                            .map(|receipts| (event.id, receipts))
                    }));
                }
                Ok(buzz_ws_client::RelayMessage::Closed { message, .. }) => {
                    while work.next().await.is_some() {}
                    return Err(NodeError::InvalidConfiguration(format!(
                        "execution subscription closed: {message}"
                    )));
                }
                Ok(_) => {}
                Err(error) => {
                    while work.next().await.is_some() {}
                    return Err(NodeError::InvalidConfiguration(error.to_string()));
                }
            },
            Some(result) = work.next(), if !work.is_empty() => {
                publish_work_result(&mut connection, controller, identity, config, result).await?;
            }
        }
    }
}

async fn publish_work_result(
    connection: &mut buzz_ws_client::NostrWsConnection,
    controller: &ExecutionController,
    identity: &NodeIdentity,
    config: &NodeConfig,
    result: Result<Result<(EventId, Vec<Event>), NodeError>, tokio::task::JoinError>,
) -> Result<(), NodeError> {
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
            .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
        if !response.accepted {
            warn!(event_id = %response.event_id, message = %response.message, "relay rejected execution receipt");
        }
    }
    let workloads = controller.workload_statuses().await;
    let announcement =
        build_announcement_with_workloads(identity, &config.display_name, &workloads)?;
    let response = connection
        .send_event(announcement)
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
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

async fn pair_node(qr_arg: Option<String>) -> Result<(), NodeError> {
    let config = NodeConfig::from_env()?;
    let identity = NodeIdentity::load_or_create(&config.data_dir)?;
    let qr_uri = match qr_arg {
        Some(value) => value,
        None => {
            print!("Paste the Desktop pairing QR URI: ");
            io::stdout().flush()?;
            read_line().map_err(NodeError::Storage)?
        }
    };
    let qr =
        decode_qr(qr_uri.trim()).map_err(|error| NodeError::PairingPayload(error.to_string()))?;
    let relay_url = qr
        .relays
        .first()
        .ok_or_else(|| NodeError::PairingPayload("QR URI contains no relay URL".into()))?
        .clone();
    let (mut session, offer) = PairingSession::new_target(&qr)
        .map_err(|error| NodeError::PairingPayload(error.to_string()))?;
    let (ws, _) = connect_async(&relay_url)
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    let (mut write, mut read) = ws.split();
    handle_pairing_auth(&mut read, &mut write, &session, &relay_url).await?;

    let subscription = serde_json::json!([
        "REQ",
        "pair",
        { "kinds": [KIND_PAIRING], "#p": [session.pubkey().to_hex()] }
    ]);
    write
        .send(Message::Text(subscription.to_string().into()))
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    wait_for_eose(&mut read, "pair").await?;
    publish_event(&mut write, &offer).await?;

    let sas = session
        .sas_code()
        .ok_or_else(|| NodeError::PairingPayload("pairing SAS was not derived".into()))?;
    println!("Pairing offer sent. SAS code: {sas}");
    print!("Does Desktop show the same SAS? [y/n]: ");
    io::stdout().flush()?;

    loop {
        let event = next_pairing_event(&mut read, "pair").await?;
        if session.handle_abort(&event).is_ok() {
            return Err(NodeError::PairingPayload("Desktop aborted pairing".into()));
        }
        match session.handle_sas_confirm(&event) {
            Ok(_) => break,
            Err(PairingError::TranscriptMismatch) => {
                return Err(NodeError::PairingPayload(
                    "pairing transcript mismatch".into(),
                ));
            }
            Err(_) => {}
        }
    }

    if !read_yes_no().map_err(NodeError::Storage)? {
        return Err(NodeError::PairingPayload(
            "SAS mismatch — pairing aborted".into(),
        ));
    }
    session
        .confirm_target_sas()
        .map_err(|error| NodeError::PairingPayload(error.to_string()))?;

    let payload: Zeroizing<String> = loop {
        let event = next_pairing_event(&mut read, "pair").await?;
        match session.handle_payload(&event) {
            Ok((PayloadType::Custom, payload)) => break payload,
            Ok(_) => {
                return Err(NodeError::PairingPayload(
                    "expected custom pairing payload".into(),
                ))
            }
            Err(_) => {}
        }
    };
    let DesktopPairingPayload { owner_pubkey, .. } = parse_desktop_pairing_payload(&payload)?;
    let mut owners = OwnerStore::load(&config.data_dir)?;
    owners.add(&owner_pubkey, &config.data_dir)?;
    publish_event(
        &mut write,
        &session
            .send_complete()
            .map_err(|error| NodeError::PairingPayload(error.to_string()))?,
    )
    .await?;
    println!(
        "Paired owner {} with node {}.",
        owner_pubkey,
        identity.keys.public_key()
    );
    Ok(())
}

async fn handle_pairing_auth<R, W>(
    read: &mut R,
    write: &mut W,
    session: &PairingSession,
    relay_url: &str,
) -> Result<(), NodeError>
where
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    W: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let challenge = match timeout(Duration::from_secs(3), async {
        loop {
            let message = read
                .next()
                .await
                .ok_or_else(|| NodeError::InvalidConfiguration("relay closed during auth".into()))?
                .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text)?;
                if value[0] == "AUTH" {
                    if let Some(challenge) = value[1].as_str() {
                        break Ok::<String, NodeError>(challenge.to_string());
                    }
                }
            }
        }
    })
    .await
    {
        Ok(result) => result?,
        Err(_) => return Ok(()),
    };
    let relay = RelayUrl::parse(relay_url)
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    let auth = session
        .sign_event(EventBuilder::auth(challenge, relay))
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    write
        .send(Message::Text(
            serde_json::json!(["AUTH", auth]).to_string().into(),
        ))
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
    Ok(())
}

async fn wait_for_eose<R>(read: &mut R, subscription: &str) -> Result<(), NodeError>
where
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let event = read
            .next()
            .await
            .ok_or_else(|| NodeError::InvalidConfiguration("relay closed before EOSE".into()))?
            .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
        if let Message::Text(text) = event {
            let value: Value = serde_json::from_str(&text)?;
            if value[0] == "EOSE" && value[1] == subscription {
                return Ok(());
            }
        }
    }
}

async fn next_pairing_event<R>(read: &mut R, subscription: &str) -> Result<Event, NodeError>
where
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let message = read
            .next()
            .await
            .ok_or_else(|| NodeError::InvalidConfiguration("relay closed during pairing".into()))?
            .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(&text)?;
            if value[0] == "EVENT" && value[1] == subscription {
                return serde_json::from_value(value[2].clone()).map_err(NodeError::from);
            }
        }
    }
}

async fn publish_event<W>(write: &mut W, event: &Event) -> Result<(), NodeError>
where
    W: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    write
        .send(Message::Text(
            serde_json::json!(["EVENT", event]).to_string().into(),
        ))
        .await
        .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))
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
