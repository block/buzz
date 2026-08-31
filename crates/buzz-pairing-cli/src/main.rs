//! `buzz-pair` — NIP-AB device pairing interop testing CLI.
//!
//! # Usage
//!
//! ```text
//! buzz-pair source --relay wss://relay.example.com [--nsec nsec1...]
//!                  [--envelope-relay https://relay.example.com]
//! buzz-pair target [--relay wss://relay.example.com]
//! buzz-pair test-vectors
//! ```
//!
//! # Payload shape
//!
//! By default `source` transfers a bare bech32 `nsec1...` string
//! ([`PayloadType::Nsec`]). The Buzz **mobile** app does not accept that: its
//! `_processPayload` begins with `jsonDecode(payload) as Map<String, dynamic>`,
//! so a bare nsec dies on the leading `n` with
//! `FormatException: Unexpected character (at character 1)`.
//!
//! Passing `--envelope-relay <https url>` switches the payload to the JSON
//! envelope the mobile app — and the desktop app, its real counterpart —
//! actually decodes: `{"relayUrl","pubkey","nsec"}` as [`PayloadType::Custom`].
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

        /// Emit the mobile/desktop JSON envelope `{relayUrl,pubkey,nsec}` instead
        /// of a bare nsec. The value is the `https://` URL of the Buzz relay the
        /// paired device should join; it becomes the envelope's `relayUrl`, and is
        /// distinct from `--relay`, which is the ephemeral pairing relay.
        #[arg(long, value_name = "HTTPS_URL")]
        envelope_relay: Option<String>,
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

#[tokio::main]
async fn main() {
    // Without this, every wss:// pairing relay panics inside
    // rustls ("Could not automatically determine the process-level
    // CryptoProvider") because both ring and aws-lc-rs are in the workspace
    // tree. Plain ws:// never reaches this code path, which is why interop
    // testing against a plaintext spike relay could not have found it.
    // Idempotent: the Err just means a provider was already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();
    if let Err(e) = run(cli.command).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cmd: Cmd) -> Result<(), CliError> {
    match cmd {
        Cmd::Source {
            relay,
            nsec,
            envelope_relay,
        } => cmd_source(relay, nsec, envelope_relay).await,
        Cmd::Target { relay, show_secret } => cmd_target(relay, show_secret).await,
        Cmd::TestVectors => cmd_test_vectors(),
    }
}

async fn cmd_source(
    relay_url: String,
    nsec: Option<String>,
    envelope_relay: Option<String>,
) -> Result<(), CliError> {
    // Resolve the payload to transfer.
    let (payload_str, payload_type) = resolve_payload(nsec, envelope_relay)?;

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

/// Build the JSON pairing envelope the Buzz apps decode.
///
/// Exactly the three fields `_processPayload` reads
/// (`mobile/lib/features/pairing/pairing_provider.dart`), in the same shape the
/// desktop app sends (`desktop/src-tauri/src/commands/pairing.rs`):
///
/// ```json
/// { "relayUrl": "https://…", "pubkey": "<64-char hex>", "nsec": "nsec1…" }
/// ```
///
/// `pubkey` is derived from `nsec` rather than taken separately — the two must
/// describe one identity, and deriving it removes the chance of them drifting.
fn build_envelope(relay_url: &str, nsec: &str) -> Result<Zeroizing<String>, CliError> {
    let sk = SecretKey::parse(nsec).map_err(|e| CliError::InvalidNsec(e.to_string()))?;
    let keys = Keys::new(sk);
    Ok(Zeroizing::new(
        serde_json::json!({
            "relayUrl": relay_url,
            "pubkey": keys.public_key().to_hex(),
            "nsec": nsec,
        })
        .to_string(),
    ))
}

/// Resolve the payload to send.
///
/// If `nsec` is provided, parse it as bech32; otherwise generate a fresh test key.
///
/// With `envelope_relay` set, the payload is the JSON envelope
/// ([`PayloadType::Custom`]) the Buzz apps decode. Without it, the payload stays
/// the bare bech32 nsec ([`PayloadType::Nsec`]) — unchanged upstream behaviour,
/// which is correct for CLI-to-CLI interop testing and wrong for the apps.
fn resolve_payload(
    nsec: Option<String>,
    envelope_relay: Option<String>,
) -> Result<(Zeroizing<String>, PayloadType), CliError> {
    let nsec = match nsec {
        Some(s) => {
            // Validate it parses as a secret key.
            let _sk = SecretKey::parse(&s).map_err(|e| CliError::InvalidNsec(e.to_string()))?;
            Zeroizing::new(s)
        }
        None => {
            let keys = Keys::generate();
            let nsec_str = keys
                .secret_key()
                .to_bech32()
                .map_err(|e| CliError::InvalidNsec(e.to_string()))?;
            println!("(no --nsec provided; using generated test key)");
            Zeroizing::new(nsec_str)
        }
    };

    match envelope_relay {
        Some(relay) => {
            // Gate 2 of `_processPayload` rejects any non-https URL in a release
            // build. Warn at mint time rather than let it surface on a handset as
            // a second, unrelated-looking failure.
            if !relay.starts_with("https://") {
                eprintln!(
                    "warning: --envelope-relay {relay} is not https:// — \
                     release builds of the Buzz app will reject it (debug builds allow it)"
                );
            }
            Ok((build_envelope(&relay, &nsec)?, PayloadType::Custom))
        }
        None => Ok((nsec, PayloadType::Nsec)),
    }
}

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

/// Decode a 64-char hex string into a `[u8; 32]`.
fn hex_to_32(s: &str) -> Result<[u8; 32], CliError> {
    let bytes = hex::decode(s).map_err(|e| CliError::Other(format!("invalid hex '{s}': {e}")))?;
    bytes
        .try_into()
        .map_err(|_| CliError::Other(format!("expected 32 bytes, got wrong length for '{s}'")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};

    const RELAY: &str = "https://relay.example.com";

    fn fresh_nsec() -> String {
        Keys::generate().secret_key().to_bech32().unwrap()
    }

    /// Stand-in for gate 1 of the mobile app's `_processPayload`:
    /// `jsonDecode(payload) as Map<String, dynamic>`. Dart throws
    /// `FormatException` where this returns `Err`.
    fn json_decode_as_map(payload: &str) -> Result<Map<String, Value>, serde_json::Error> {
        serde_json::from_str::<Map<String, Value>>(payload)
    }

    /// The defect this change exists to fix, pinned so it cannot come back
    /// silently: the default payload is a bare bech32 nsec, and the app's very
    /// first step cannot decode it. Character 1 is the `n` of `nsec1`.
    #[test]
    fn bare_nsec_payload_fails_the_apps_first_gate() {
        let nsec = fresh_nsec();
        let (payload, ty) = resolve_payload(Some(nsec.clone()), None).unwrap();

        assert!(matches!(ty, PayloadType::Nsec));
        assert!(payload.starts_with("nsec1"));
        assert!(
            json_decode_as_map(&payload).is_err(),
            "a bare nsec must not be JSON — this is the FormatException the mobile app raises"
        );
    }

    /// The fix: with `--envelope-relay`, gate 1 passes and the map carries
    /// exactly the three fields `_processPayload` reads.
    #[test]
    fn envelope_payload_clears_the_apps_first_gate() {
        let nsec = fresh_nsec();
        let (payload, ty) = resolve_payload(Some(nsec.clone()), Some(RELAY.to_string())).unwrap();

        assert!(matches!(ty, PayloadType::Custom));
        let map = json_decode_as_map(&payload).expect("envelope must jsonDecode as a map");

        assert_eq!(map.len(), 3, "exactly three fields, no extras: {map:?}");
        assert_eq!(map["relayUrl"], Value::String(RELAY.to_string()));
        assert_eq!(map["nsec"], Value::String(nsec.clone()));

        // `relayUrl` non-null is the app's own second check inside gate 1.
        assert!(map["relayUrl"].is_string());
    }

    /// `pubkey` must belong to the transferred `nsec`. A fresh or stale key here
    /// decodes fine and then strands the device on a community it cannot sign for.
    #[test]
    fn envelope_pubkey_is_derived_from_the_transferred_nsec() {
        let nsec = fresh_nsec();
        let expected = Keys::new(SecretKey::parse(&nsec).unwrap())
            .public_key()
            .to_hex();

        let (payload, _) = resolve_payload(Some(nsec), Some(RELAY.to_string())).unwrap();
        let map = json_decode_as_map(&payload).unwrap();

        assert_eq!(map["pubkey"], Value::String(expected));
        assert_eq!(
            map["pubkey"].as_str().unwrap().len(),
            64,
            "pubkey is 64-char hex, not npub — the app stores it as Community.pubkey"
        );
    }

    /// The generated-key path (no `--nsec`) must produce a coherent envelope too,
    /// not just the explicit-key path.
    #[test]
    fn generated_key_envelope_is_internally_consistent() {
        let (payload, ty) = resolve_payload(None, Some(RELAY.to_string())).unwrap();

        assert!(matches!(ty, PayloadType::Custom));
        let map = json_decode_as_map(&payload).unwrap();
        let nsec = map["nsec"].as_str().unwrap();
        let derived = Keys::new(SecretKey::parse(nsec).unwrap())
            .public_key()
            .to_hex();

        assert_eq!(map["pubkey"], Value::String(derived));
    }

    /// Upstream CLI-to-CLI interop behaviour must be untouched when the flag is
    /// absent — this crate is still the NIP-AB interop tool.
    #[test]
    fn absent_flag_leaves_upstream_behaviour_unchanged() {
        let nsec = fresh_nsec();
        let (payload, ty) = resolve_payload(Some(nsec.clone()), None).unwrap();

        assert!(matches!(ty, PayloadType::Nsec));
        assert_eq!(&*payload, &nsec);
    }

    /// An unparseable nsec must be rejected at mint time, not shipped inside a
    /// well-formed envelope that only fails three gates later on the handset.
    #[test]
    fn invalid_nsec_is_rejected_before_an_envelope_is_built() {
        assert!(build_envelope(RELAY, "nsec1notarealkey").is_err());
        assert!(resolve_payload(Some("definitely-not-bech32".into()), Some(RELAY.into())).is_err());
    }

    /// Unicode and empty relay URLs must not corrupt the JSON — serde escapes
    /// them, and Dart's `jsonDecode` reads them back byte-identically.
    #[test]
    fn odd_relay_urls_stay_well_formed_json() {
        let nsec = fresh_nsec();
        for relay in [
            "",
            "https://relay.exämple.com/pä†h",
            "https://a.com/\"quote\"",
        ] {
            let payload = build_envelope(relay, &nsec).unwrap();
            let map = json_decode_as_map(&payload)
                .unwrap_or_else(|e| panic!("relay {relay:?} broke the envelope: {e}"));
            assert_eq!(map["relayUrl"], Value::String(relay.to_string()));
            assert_eq!(map.len(), 3);
        }
    }
}
