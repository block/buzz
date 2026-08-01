use std::io::{self, BufRead, Write};
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
use nostr::{Event, EventBuilder, RelayUrl};
use serde_json::Value;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use zeroize::Zeroizing;

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
    let mut controller = ExecutionController::load(&config.data_dir)?;
    let mut shutdown = Box::pin(tokio::signal::ctrl_c());
    let mut retry_delay = Duration::from_secs(1);

    loop {
        tokio::select! {
            result = &mut shutdown => {
                result.map_err(NodeError::Storage)?;
                return Ok(());
            }
            result = run_connection(&config, &identity, &owners, &mut controller) => {
                if let Err(error) = result {
                    warn!(%error, ?retry_delay, "execution node relay connection ended");
                    sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                } else {
                    retry_delay = Duration::from_secs(1);
                }
            }
        }
    }
}

async fn run_connection(
    config: &NodeConfig,
    identity: &NodeIdentity,
    owners: &OwnerStore,
    controller: &mut ExecutionController,
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
    info!(node = %identity.keys.public_key().to_hex(), "execution node connected");

    let mut work = FuturesUnordered::new();
    loop {
        tokio::select! {
            result = connection.next_event(Duration::from_secs(300)) => match result {
                Ok(buzz_ws_client::RelayMessage::Event { event, .. }) => {
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
                let (event_id, receipts) = match result {
                    Ok(Ok(result)) => result,
                    Ok(Err(error)) => {
                        warn!(%error, "execution command rejected");
                        continue;
                    }
                    Err(error) => {
                        warn!(%error, "execution command task failed");
                        continue;
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
                let announcement = build_announcement_with_workloads(
                    identity,
                    &config.display_name,
                    &workloads,
                )?;
                let response = connection
                    .send_event(announcement)
                    .await
                    .map_err(|error| NodeError::InvalidConfiguration(error.to_string()))?;
                if !response.accepted {
                    warn!(event_id = %event_id, message = %response.message, "relay rejected execution-node status announcement");
                }
            }
        }
    }
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
