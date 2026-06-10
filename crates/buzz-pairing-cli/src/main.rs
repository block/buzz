//! `buzz-pair` — NIP-AB device pairing interop testing CLI.
//!
//! # Usage
//!
//! ```text
//! buzz-pair source --relay wss://relay.example.com [--nsec nsec1...]
//! buzz-pair target [--relay wss://relay.example.com]
//! buzz-pair test-vectors
//! ```
//!
//! The `source` subcommand acts as the secret-holding device; `target` acts
//! as the receiving device. Together they exercise the full NIP-AB protocol
//! over a live Nostr relay.

use std::io::{self, BufRead, Write};
use std::time::Duration;

use buzz_core::kind::KIND_PAIRING;
use buzz_core::pairing::session::PairingSession;
use buzz_core::pairing::{
    crypto::{derive_sas, derive_session_id, derive_transcript_hash, format_sas},
    qr::{decode_qr, encode_qr},
    types::PayloadType,
    PairingError,
};
use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, Keys, RelayUrl, SecretKey, ToBech32};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use zeroize::Zeroizing;

// ── CLI definition ────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "buzz-pair",
    about = "NIP-AB device pairing interop testing tool",
    long_about = "Test the NIP-AB device pairing protocol end-to-end.\n\
                  Run 'source' on one terminal and 'target' on another."
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Act as the source device (holds the secret, displays QR code).
    Source {
        /// Relay WebSocket URL to use for pairing.
        #[arg(long, default_value = "wss://relay.damus.io")]
        relay: String,

        /// nsec (bech32) of the key to transfer. If omitted, generates a test key.
        #[arg(long)]
        nsec: Option<String>,
    },

    /// Act as the target device (scans QR code, receives the secret).
    Target {
        /// Override relay URL (default: read from QR URI).
        #[arg(long)]
        relay: Option<String>,

        /// Print received secrets to stdout. Off by default.
        #[arg(long, default_value_t = false)]
        show_secret: bool,
    },

    /// Print NIP-AB test vectors derived from the spec's fixed keys.
    TestVectors,
}

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("pairing error: {0}")]
    Pairing(#[from] PairingError),

    #[error("WebSocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("invalid nsec: {0}")]
    InvalidNsec(String),

    #[error("timeout waiting for peer")]
    Timeout,

    #[error("{0}")]
    Other(String),
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli.command).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cmd: Cmd) -> Result<(), CliError> {
    match cmd {
        Cmd::Source { relay, nsec } => cmd_source(relay, nsec).await,
        Cmd::Target { relay, show_secret } => cmd_target(relay, show_secret).await,
        Cmd::TestVectors => cmd_test_vectors(),
    }
}

// ── source subcommand ─────────────────────────────────────────────────────────

async fn cmd_source(relay_url: String, nsec: Option<String>) -> Result<(), CliError> {
    // Resolve the payload to transfer.
    let (payload_str, payload_type) = resolve_payload(nsec)?;

    // Create pairing session.
    let (mut session, qr) = PairingSession::new_source(relay_url.clone());
    let qr_uri = encode_qr(&qr);

    println!("QR URI (contains session secret — do not share beyond the target device):");
    println!("{qr_uri}");
    println!("Waiting for target to scan QR code...");

    // Connect to relay and handle NIP-42 auth if required.
    // Auth uses the session's ephemeral keys so the relay accepts our events.
    let (ws, _) = connect_async(&relay_url).await?;
    let (mut write, mut read) = ws.split();
    handle_nip42_auth(&mut read, &mut write, &session, &relay_url).await?;

    // Subscribe for events tagged to our ephemeral pubkey.
    let our_pk = session.pubkey().to_hex();
    let sub_msg = serde_json::json!([
        "REQ",
        "pair",
        { "kinds": [KIND_PAIRING], "#p": [our_pk] }
    ]);
    write
        .send(Message::Text(sub_msg.to_string().into()))
        .await?;

    // Wait for EOSE to confirm the subscription is registered on the relay
    // before the target can race us with an offer we'd miss.
    wait_for_eose(&mut read, "pair", Duration::from_secs(10)).await?;

    // Wait for a valid offer event (silently discard junk per NIP-AB §Event Validation).
    let sas = loop {
        let event = wait_for_event(&mut read, "pair", Duration::from_secs(120)).await?;
        check_for_abort(&mut session, &event)?;
        match session.handle_offer(&event) {
            Ok(sas) => break sas,
            Err(_) => continue, // silently discard per NIP-AB §Event Validation item 7
        }
    };
    println!("Offer received from target.");
    println!("SAS code: {sas}");
    print!("Does your other device show {sas}? [y/n]: ");
    io::stdout().flush()?;

    let confirmed = read_yes_no()?;
    if !confirmed {
        // Send abort and exit.
        if let Some(abort_event) =
            session.abort(buzz_core::pairing::types::AbortReason::SasMismatch)?
        {
            publish_event(&mut write, &abort_event).await?;
        }
        return Err(CliError::Other("SAS mismatch — session aborted".into()));
    }

    // Send sas-confirm.
    let sas_confirm_event = session.confirm_sas()?;
    publish_event(&mut write, &sas_confirm_event).await?;
    println!("Sending identity...");

    // Send payload.
    let payload_event = session.send_payload(payload_type, payload_str)?;
    publish_event(&mut write, &payload_event).await?;

    // Wait for a valid complete event (skip junk; exit on peer abort).
    // Surface complete(success=false) explicitly instead of swallowing it.
    loop {
        let event = wait_for_event(&mut read, "pair", Duration::from_secs(60)).await?;
        check_for_abort(&mut session, &event)?;
        match session.handle_complete(&event) {
            Ok(()) => break,
            Err(PairingError::UnexpectedMessage { ref got, .. })
                if got.contains("success=false") =>
            {
                return Err(CliError::Other(
                    "target reported failure importing the key — check the other device".into(),
                ));
            }
            Err(_) => continue, // silently discard per NIP-AB §Event Validation item 7
        }
    }

    println!("Transfer complete! ✓");
    Ok(())
}

// ── target subcommand ─────────────────────────────────────────────────────────

async fn cmd_target(relay_override: Option<String>, show_secret: bool) -> Result<(), CliError> {
    // Read QR URI from stdin.
    print!("Paste the QR URI: ");
    io::stdout().flush()?;
    let qr_uri = read_line()?;
    let qr_uri = qr_uri.trim();

    // Decode QR.
    let mut qr = decode_qr(qr_uri)?;

    // Apply relay override if provided.
    if let Some(relay) = relay_override {
        qr.relays = vec![relay];
    }

    let relay_url = qr
        .relays
        .first()
        .cloned()
        .ok_or_else(|| CliError::Other("QR URI contains no relay URL".into()))?;

    println!("Connecting to {relay_url}...");

    // Create target session + offer event.
    let (mut session, offer_event) = PairingSession::new_target(&qr)?;

    // Connect to relay and handle NIP-42 auth if required.
    let (ws, _) = connect_async(&relay_url).await?;
    let (mut write, mut read) = ws.split();
    handle_nip42_auth(&mut read, &mut write, &session, &relay_url).await?;

    // Subscribe BEFORE publishing the offer so we don't miss a fast
    // sas-confirm from the source (fixes a race condition).
    let our_pk = session.pubkey().to_hex();
    let sub_msg = serde_json::json!([
        "REQ",
        "pair",
        { "kinds": [KIND_PAIRING], "#p": [our_pk] }
    ]);
    write
        .send(Message::Text(sub_msg.to_string().into()))
        .await?;

    // Wait for EOSE to confirm the subscription is registered on the relay
    // before publishing the offer. Without this, the relay may process our
    // EVENT before our REQ, causing us to miss the source's response.
    wait_for_eose(&mut read, "pair", Duration::from_secs(10)).await?;

    // Now publish the offer event.
    publish_event(&mut write, &offer_event).await?;

    // Target already knows the SAS from the QR scan — display it now so
    // the user can compare while the source is also displaying its code.
    let sas = session
        .sas_code()
        .ok_or_else(|| CliError::Other("no SAS code".into()))?;
    println!("SAS code: {sas}");
    println!("Verify this matches your source device.");
    println!("Offer sent. Waiting for source to confirm SAS...");

    // Wait for a valid sas-confirm event (skip junk; exit on peer abort).
    // TranscriptMismatch is a hard security failure (possible MITM) —
    // surface it immediately rather than swallowing it in the generic handler.
    loop {
        let event = wait_for_event(&mut read, "pair", Duration::from_secs(120)).await?;
        check_for_abort(&mut session, &event)?;
        match session.handle_sas_confirm(&event) {
            Ok(_) => break,
            Err(PairingError::TranscriptMismatch) => {
                // NIP-AB §Step 3: target MUST send abort with reason
                // "sas_mismatch" on transcript hash mismatch.
                if let Ok(Some(abort_event)) =
                    session.abort(buzz_core::pairing::types::AbortReason::SasMismatch)
                {
                    let _ = publish_event(&mut write, &abort_event).await;
                }
                return Err(CliError::Other(
                    "SECURITY: transcript hash mismatch — possible MITM attack. Session aborted."
                        .into(),
                ));
            }
            Err(_) => continue, // silently discard per NIP-AB §Event Validation item 7
        }
    }

    // Explicit target-side confirmation: the user must approve.
    print!("Does your source device show {sas}? [y/n]: ");
    io::stdout().flush()?;
    let confirmed = read_yes_no()?;
    if !confirmed {
        if let Some(abort_event) =
            session.abort(buzz_core::pairing::types::AbortReason::SasMismatch)?
        {
            publish_event(&mut write, &abort_event).await?;
        }
        return Err(CliError::Other("SAS mismatch — session aborted".into()));
    }
    session.confirm_target_sas()?;
    println!("SAS confirmed. Waiting for payload...");

    // Wait for a valid payload event (silently discard junk; exit on peer abort).
    let (payload_type, payload) = loop {
        let event = wait_for_event(&mut read, "pair", Duration::from_secs(60)).await?;
        check_for_abort(&mut session, &event)?;
        match session.handle_payload(&event) {
            Ok(result) => break result,
            Err(_) => continue, // silently discard per NIP-AB §Event Validation item 7
        }
    };

    // Display received payload (secrets gated behind --show-secret).
    let kind_label = match payload_type {
        PayloadType::Nsec => "nsec",
        PayloadType::Bunker => "bunker",
        PayloadType::Connect => "nostrconnect",
        PayloadType::Custom => "custom",
    };
    println!("Received {kind_label} payload!");
    if show_secret {
        println!("{kind_label}: {}", &*payload);
    } else {
        println!("(use --show-secret to display the received secret)");
    }

    // Send complete event.
    let complete_event = session.send_complete()?;
    publish_event(&mut write, &complete_event).await?;

    println!("Transfer complete! ✓");
    Ok(())
}

// ── test-vectors subcommand ───────────────────────────────────────────────────

fn cmd_test_vectors() -> Result<(), CliError> {
    // Fixed test keys from the NIP-AB spec.
    let session_secret: [u8; 32] =
        hex_to_32("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2")?;
    let source_priv: [u8; 32] =
        hex_to_32("7f4c11a9c9d1e3b5a7f2e4d6c8b0a2f4e6d8c0b2a4f6e8d0c2b4a6f8e0d2c4b5")?;
    let target_priv: [u8; 32] =
        hex_to_32("3a5b7c9d1e3f5a7b9c1d3e5f7a9b1c3d5e7f9a1b3c5d7e9f1a3b5c7d9e1f3a5b")?;

    // Derive keys.
    let src_sk =
        SecretKey::from_slice(&source_priv).map_err(|e| CliError::InvalidNsec(e.to_string()))?;
    let tgt_sk =
        SecretKey::from_slice(&target_priv).map_err(|e| CliError::InvalidNsec(e.to_string()))?;
    let src_keys = Keys::new(src_sk);
    let tgt_keys = Keys::new(tgt_sk);

    let source_pubkey: [u8; 32] = src_keys.public_key().to_bytes();
    let target_pubkey: [u8; 32] = tgt_keys.public_key().to_bytes();

    // Derive all values.
    let session_id = derive_session_id(&session_secret);
    let ecdh_shared =
        nostr::util::generate_shared_key(src_keys.secret_key(), &tgt_keys.public_key())
            .map_err(|e| CliError::Other(e.to_string()))?;
    let (sas_code_u32, sas_input) = derive_sas(&ecdh_shared, &session_secret);
    let sas_code = format_sas(sas_code_u32);
    let transcript_hash = derive_transcript_hash(
        &session_id,
        &source_pubkey,
        &target_pubkey,
        &sas_input,
        &session_secret,
    );

    // Print as a table suitable for pasting into the NIP spec.
    let col_w = 20usize;
    let val_w = 66usize;
    let sep = format!("+-{:-<col_w$}-+-{:-<val_w$}-+", "", "");

    println!("{sep}");
    println!("| {:<col_w$} | {:<val_w$} |", "Field", "Value");
    println!("{sep}");

    let rows: &[(&str, String)] = &[
        ("session_secret", hex::encode(session_secret)),
        ("source_priv", hex::encode(source_priv)),
        ("target_priv", hex::encode(target_priv)),
        ("source_pubkey", hex::encode(source_pubkey)),
        ("target_pubkey", hex::encode(target_pubkey)),
        ("ecdh_shared", hex::encode(ecdh_shared)),
        ("session_id", hex::encode(session_id)),
        ("sas_input", hex::encode(sas_input)),
        ("sas_code", sas_code),
        ("transcript_hash", hex::encode(transcript_hash)),
    ];

    for (field, value) in rows {
        println!("| {field:<col_w$} | {value:<val_w$} |");
    }
    println!("{sep}");

    Ok(())
}

// ── Abort-aware event helpers ─────────────────────────────────────────────────

/// Check whether `event` is an abort from the peer. If so, transition the
/// session and return an error the caller can propagate. Otherwise return
/// `Ok(())` so the caller can proceed with its own handler.
fn check_for_abort(session: &mut PairingSession, event: &Event) -> Result<(), CliError> {
    match session.handle_abort(event) {
        Ok(reason) => Err(CliError::Other(format!(
            "peer aborted the session: {reason:?}"
        ))),
        Err(_) => Ok(()), // not an abort — caller should try its own handler
    }
}

// ── NIP-42 auth helper ────────────────────────────────────────────────────────

/// Handle NIP-42 authentication if the relay requires it.
///
/// Uses the pairing session's ephemeral keys to authenticate, ensuring the
/// relay accepts events signed by those same keys.
async fn handle_nip42_auth<R, W>(
    read: &mut R,
    write: &mut W,
    session: &PairingSession,
    relay_url: &str,
) -> Result<(), CliError>
where
    R: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    W: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    // Wait up to 3 seconds for an AUTH challenge. Many relays don't require
    // auth at all, so a timeout here is normal (not an error).
    let auth_result = timeout(Duration::from_secs(3), async {
        loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| CliError::Other("relay closed during auth".into()))??;

            if let Message::Text(text) = msg {
                if let Some(challenge) = parse_auth_challenge(text.as_str()) {
                    return Ok(challenge);
                }
            }
        }
    })
    .await;

    let challenge = match auth_result {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Ok(()), // No AUTH challenge — relay doesn't require it
    };

    // Build and send the NIP-42 auth response using the session's ephemeral keys.
    let relay_url_parsed = RelayUrl::parse(relay_url)
        .map_err(|e| CliError::Other(format!("invalid relay URL: {e}")))?;
    let auth_event = session
        .sign_event(EventBuilder::auth(challenge, relay_url_parsed))
        .map_err(|e| CliError::Other(format!("failed to sign auth event: {e}")))?;

    let msg = serde_json::json!(["AUTH", auth_event]);
    write.send(Message::Text(msg.to_string().into())).await?;

    // Wait for OK response (up to 5 seconds).
    let _ = timeout(Duration::from_secs(5), async {
        loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| CliError::Other("relay closed during auth".into()))??;
            if let Message::Text(text) = msg {
                if text.contains("\"OK\"") || text.contains("[\"OK\"") {
                    return Ok::<(), CliError>(());
                }
            }
        }
    })
    .await;

    Ok(())
}

/// Parse an `["AUTH", "<challenge>"]` relay message.
fn parse_auth_challenge(text: &str) -> Option<String> {
    let arr: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = arr.as_array()?;
    if arr.len() >= 2 && arr[0].as_str()? == "AUTH" {
        return arr[1].as_str().map(|s| s.to_string());
    }
    None
}

// ── WebSocket helpers ─────────────────────────────────────────────────────────

/// Publish a Nostr event to the relay.
async fn publish_event<S>(write: &mut S, event: &Event) -> Result<(), CliError>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let msg = serde_json::json!(["EVENT", event]);
    write.send(Message::Text(msg.to_string().into())).await?;
    Ok(())
}

/// Wait for the next [`Event`] from the relay on a given subscription ID.
///
/// Skips `OK`, `EOSE`, and non-EVENT messages. Returns [`CliError::Timeout`]
/// if no event arrives within `dur`.
async fn wait_for_event<S>(read: &mut S, sub_id: &str, dur: Duration) -> Result<Event, CliError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    timeout(dur, async {
        loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| CliError::Other("relay connection closed".into()))??;

            if let Message::Text(text) = msg {
                if let Some(event) = parse_relay_event(text.as_str(), sub_id) {
                    return Ok(event);
                }
            }
        }
    })
    .await
    .map_err(|_| CliError::Timeout)?
}

/// Wait for an EOSE message from the relay for the given subscription ID.
///
/// EOSE (`["EOSE", "<sub_id>"]`) confirms the subscription is registered and
/// all historical events have been delivered. Skips non-EOSE messages.
async fn wait_for_eose<S>(read: &mut S, sub_id: &str, dur: Duration) -> Result<(), CliError>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    timeout(dur, async {
        loop {
            let msg = read
                .next()
                .await
                .ok_or_else(|| CliError::Other("relay closed while waiting for EOSE".into()))??;
            if let Message::Text(text) = msg {
                if let Ok(arr) = serde_json::from_str::<serde_json::Value>(text.as_str()) {
                    if let Some(arr) = arr.as_array() {
                        if arr.len() >= 2
                            && arr[0].as_str() == Some("EOSE")
                            && arr[1].as_str() == Some(sub_id)
                        {
                            return Ok(());
                        }
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| CliError::Timeout)?
}

/// Parse a relay message of the form `["EVENT", "<sub_id>", <event_json>]`.
///
/// Returns `None` for any other message type.
fn parse_relay_event(text: &str, sub_id: &str) -> Option<Event> {
    let arr: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = arr.as_array()?;

    if arr.len() < 3 {
        return None;
    }
    if arr[0].as_str()? != "EVENT" {
        return None;
    }
    if arr[1].as_str()? != sub_id {
        return None;
    }

    serde_json::from_value(arr[2].clone()).ok()
}

// ── Payload helpers ───────────────────────────────────────────────────────────

/// Resolve the payload to send.
///
/// If `nsec` is provided, parse it as bech32 and return the raw nsec string.
/// Otherwise generate a fresh test key and return its nsec.
fn resolve_payload(nsec: Option<String>) -> Result<(Zeroizing<String>, PayloadType), CliError> {
    match nsec {
        Some(s) => {
            // Validate it parses as a secret key.
            let _sk = SecretKey::parse(&s).map_err(|e| CliError::InvalidNsec(e.to_string()))?;
            Ok((Zeroizing::new(s), PayloadType::Nsec))
        }
        None => {
            let keys = Keys::generate();
            let nsec_str = keys
                .secret_key()
                .to_bech32()
                .map_err(|e| CliError::InvalidNsec(e.to_string()))?;
            println!("(no --nsec provided; using generated test key)");
            Ok((Zeroizing::new(nsec_str), PayloadType::Nsec))
        }
    }
}

// ── I/O helpers ───────────────────────────────────────────────────────────────

/// Read a single line from stdin (trims trailing newline).
fn read_line() -> Result<String, CliError> {
    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    Ok(line
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string())
}

/// Prompt for y/n and return true for 'y'/'Y'.
fn read_yes_no() -> Result<bool, CliError> {
    let line = read_line()?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes" | "Yes" | "YES"))
}

// ── Crypto helpers ────────────────────────────────────────────────────────────

/// Decode a 64-char hex string into a `[u8; 32]`.
fn hex_to_32(s: &str) -> Result<[u8; 32], CliError> {
    let bytes = hex::decode(s).map_err(|e| CliError::Other(format!("invalid hex '{s}': {e}")))?;
    bytes
        .try_into()
        .map_err(|_| CliError::Other(format!("expected 32 bytes, got wrong length for '{s}'")))
}
