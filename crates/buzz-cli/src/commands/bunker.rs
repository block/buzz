//! NIP-46 remote signing — bunker connect and serve modes.

use crate::error::CliError;
use crate::OutputFormat;
use buzz_core::kind::KIND_NOSTR_REMOTE_SIGNING;
use buzz_ws_client::{NostrWsConnection, RelayMessage};
use nostr::{
    nips::nip44::{self, Version},
    Event, EventBuilder, Keys, Kind, PublicKey, Tag, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::BufRead;
use std::time::Duration;

/// NIP-46 bunker subcommand.
#[derive(clap::Subcommand)]
pub enum BunkerCmd {
    /// Connect to a remote signer (client mode)
    #[command(
        after_help = "Examples:\n  buzz bunker connect bunker://<pubkey>?relay=wss://relay.example.com&secret=xyz\n  buzz bunker connect bunker://<pubkey>?relay=wss://relay.example.com"
    )]
    Connect {
        /// Bunker URL (bunker://<remote-signer-pubkey>?relay=<url>&secret=<optional>)
        bunker_url: String,
        /// Client name for metadata
        #[arg(long)]
        name: Option<String>,
        /// Client URL for metadata
        #[arg(long)]
        url: Option<String>,
        /// Client image URL for metadata
        #[arg(long)]
        image: Option<String>,
        /// Requested permissions (comma-separated, e.g., "sign_event:1,nip44_encrypt")
        #[arg(long)]
        perms: Option<String>,
    },
    /// Start a remote signer (server mode)
    #[command(
        after_help = "Examples:\n  buzz bunker serve\n  buzz bunker serve --auto-approve --timeout 0"
    )]
    Serve {
        /// Auto-approve all signing requests (insecure, dev only)
        #[arg(long, default_value_t = false)]
        auto_approve: bool,
        /// Relays to announce and listen on (comma-separated wss:// URLs)
        #[arg(long)]
        relays: Option<String>,
        /// Inactivity timeout in seconds (0 = infinite, default 3600)
        #[arg(long, default_value_t = 3600)]
        timeout: u64,
    },
}

/// NIP-46 request payload.
#[derive(Debug, Serialize, Deserialize)]
struct Nip46Request {
    id: String,
    method: String,
    params: Vec<String>,
}

/// NIP-46 response payload.
#[derive(Debug, Serialize, Deserialize)]
struct Nip46Response {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Client metadata for connect requests.
#[derive(Debug, Serialize, Deserialize)]
struct ClientMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

pub async fn handle(
    cmd: BunkerCmd,
    keys: &Keys,
    relay_url: &str,
    _format: OutputFormat,
) -> Result<(), CliError> {
    match cmd {
        BunkerCmd::Connect {
            bunker_url,
            name,
            url,
            image,
            perms,
        } => connect(keys, relay_url, &bunker_url, name, url, image, perms).await,
        BunkerCmd::Serve {
            auto_approve,
            relays,
            timeout,
        } => serve(keys, relay_url, auto_approve, relays, timeout).await,
    }
}

async fn connect(
    client_keys: &Keys,
    _relay_url: &str,
    bunker_url: &str,
    name: Option<String>,
    url: Option<String>,
    image: Option<String>,
    perms: Option<String>,
) -> Result<(), CliError> {
    if !bunker_url.starts_with("bunker://") {
        return Err(CliError::Usage(
            "Bunker URL must start with bunker:// (example: bunker://<pubkey>?relay=wss://relay.example.com)".to_string(),
        ));
    }

    let (remote_signer_pubkey, relays, secret) = parse_bunker_url(bunker_url)?;

    let relay_ws_url = relays[0]
        .replace("http://", "ws://")
        .replace("https://", "wss://");

    // Use NostrWsConnection::connect (not connect_authenticated) because bunker relays
    // often don't implement NIP-42 AUTH challenges — NIP-46 encryption provides auth
    let mut conn = NostrWsConnection::connect(&relay_ws_url)
        .await
        .map_err(|e| CliError::Other(format!("WebSocket connection failed: {}", e)))?;

    let metadata = if name.is_some() || url.is_some() || image.is_some() {
        Some(ClientMetadata { name, url, image })
    } else {
        None
    };

    // NIP-46 connect params per spec: [remote-signer-pubkey, secret, perms, metadata]
    // The pubkey IS included as first param (verified against production signers)
    let mut params = vec![remote_signer_pubkey.to_string()];
    let sent_secret = if let Some(s) = secret.clone() {
        params.push(s.clone());
        Some(s)
    } else {
        params.push(String::new());
        None
    };
    if let Some(p) = perms {
        params.push(p);
    } else {
        params.push(String::new());
    }
    if let Some(m) = metadata {
        params.push(
            serde_json::to_string(&m).map_err(|e| {
                CliError::Other(format!("Failed to serialize client metadata: {}", e))
            })?,
        );
    }

    let request = Nip46Request {
        id: uuid::Uuid::new_v4().to_string(),
        method: "connect".to_string(),
        params,
    };

    let event = build_nip46_request(client_keys, &remote_signer_pubkey, &request)?;

    let sub_id = uuid::Uuid::new_v4().to_string();
    conn.send_raw(&json!([
        "REQ",
        sub_id,
        {
            "kinds": [KIND_NOSTR_REMOTE_SIGNING],
            "#p": [client_keys.public_key().to_hex()],
            "authors": [remote_signer_pubkey.to_hex()],
        }
    ]))
    .await
    .map_err(|e| CliError::Other(format!("Failed to send REQ: {}", e)))?;

    conn.send_event(event)
        .await
        .map_err(|e| CliError::Other(format!("Failed to send connect request: {}", e)))?;

    eprintln!("Connect request sent to {}", remote_signer_pubkey);
    eprintln!("Waiting for response...");

    let response_event = loop {
        match conn.next_event(Duration::from_secs(30)).await {
            Ok(RelayMessage::Event { event, .. }) => {
                if event.kind == Kind::Custom(KIND_NOSTR_REMOTE_SIGNING as u16)
                    && event.pubkey == remote_signer_pubkey
                    && event.tags.iter().any(|t| {
                        matches!(t.as_standardized(), Some(nostr::TagStandard::PublicKey { public_key, .. }) if public_key == &client_keys.public_key())
                    })
                {
                    break *event;
                }
            }
            Ok(_) => continue,
            Err(e) => {
                return Err(CliError::Other(
                    format!(
                    "Failed waiting for response: {}",
                    e
                )))
            }
        }
    };

    let response = decrypt_nip46_response(client_keys, &response_event)?;

    if let Some(err) = response.error {
        return Err(CliError::Other(format!("Remote signer error: {}", err)));
    }

    // Validate returned secret to prevent connection spoofing (NIP-46 security requirement)
    if let Some(expected_secret) = sent_secret {
        match response.result.as_deref() {
            Some(returned) if returned != "ack" && returned != expected_secret => {
                return Err(CliError::Other(
                    "Secret mismatch — possible connection spoofing attempt".to_string(),
                ));
            }
            _ => {}
        }
    }

    match response.result.as_deref() {
        Some("ack") => {
            println!(
                "{{\"status\":\"connected\",\"remote_signer\":\"{}\"}}",
                remote_signer_pubkey
            );
            Ok(())
        }
        Some(result) => {
            println!(
                "{{\"status\":\"connected\",\"remote_signer\":\"{}\",\"result\":\"{}\"}}",
                remote_signer_pubkey, result
            );
            Ok(())
        }
        None => Err(CliError::Other("No result in response".to_string())),
    }
}

async fn serve(
    signer_keys: &Keys,
    relay_url: &str,
    auto_approve: bool,
    relays: Option<String>,
    timeout: u64,
) -> Result<(), CliError> {
    if auto_approve {
        eprintln!("WARNING: Auto-approve enabled — all signing requests will be approved without prompt. This is insecure and should only be used in development.");
    }

    let relay_list = if let Some(r) = relays {
        r.split(',').map(|s| s.trim().to_string()).collect()
    } else {
        vec![relay_url.to_string()]
    };

    let bunker_url = format!(
        "bunker://{}?relay={}",
        signer_keys.public_key(),
        urlencoding::encode(&relay_list[0])
    );

    eprintln!("Remote signer started");
    eprintln!("Bunker URL: {}", bunker_url);
    eprintln!("Public key: {}", signer_keys.public_key());
    eprintln!("Relays: {}", relay_list.join(", "));
    if timeout == 0 {
        eprintln!("Timeout: infinite");
    } else {
        eprintln!("Timeout: {}s inactivity", timeout);
    }
    eprintln!();

    let relay_ws_url = relay_list[0]
        .replace("http://", "ws://")
        .replace("https://", "wss://");

    // Use NostrWsConnection::connect (not connect_authenticated) because bunker relays
    // often don't implement NIP-42 AUTH challenges — NIP-46 encryption provides auth
    let mut conn = NostrWsConnection::connect(&relay_ws_url)
        .await
        .map_err(|e| CliError::Other(format!("WebSocket connection failed: {}", e)))?;

    let sub_id = uuid::Uuid::new_v4().to_string();
    conn.send_raw(&json!([
        "REQ",
        sub_id,
        {
            "kinds": [KIND_NOSTR_REMOTE_SIGNING],
            "#p": [signer_keys.public_key().to_hex()],
        }
    ]))
    .await
    .map_err(|e| CliError::Other(format!("Failed to subscribe: {}", e)))?;

    eprintln!("Listening for requests... (Ctrl+C to stop)");

    // Track connected clients for session management per NIP-46 spec:
    // Clients must call 'connect' before making requests. After 'logout',
    // the signer rejects further requests from that client until re-connection.
    let mut connected_clients: HashSet<PublicKey> = HashSet::new();

    loop {
        let wait_duration = if timeout == 0 {
            Duration::from_secs(u64::MAX)
        } else {
            Duration::from_secs(timeout)
        };

        match conn.next_event(wait_duration).await {
            Ok(RelayMessage::Event { event, .. }) => {
                if event.kind == Kind::Custom(KIND_NOSTR_REMOTE_SIGNING as u16)
                    && event.tags.iter().any(|t| {
                        matches!(t.as_standardized(), Some(nostr::TagStandard::PublicKey { public_key, .. }) if public_key == &signer_keys.public_key())
                    })
                {
                    let client_pubkey = event.pubkey;
                    eprintln!("Request from {}", client_pubkey);

                    match handle_request(
                        signer_keys,
                        &client_pubkey,
                        &event,
                        auto_approve,
                        &mut connected_clients,
                    )
                    .await
                    {
                        Ok(response_event) => {
                            conn.send_event(response_event)
                                .await
                                .map_err(|e| {
                                    CliError::Other(format!("Failed to send response: {}", e))
                                })?;
                            eprintln!("Response sent");
                        }
                        Err(e) => {
                            eprintln!("Error handling request: {}", e);
                        }
                    }
                }
            }
            Ok(_) => continue,
            Err(e) => {
                if timeout > 0 {
                    eprintln!("Timeout after {}s inactivity — shutting down", timeout);
                    return Ok(());
                }
                eprintln!("Error receiving event: {}", e);
                continue;
            }
        }
    }
}

async fn handle_request(
    signer_keys: &Keys,
    client_pubkey: &PublicKey,
    event: &Event,
    auto_approve: bool,
    connected_clients: &mut HashSet<PublicKey>,
) -> Result<Event, CliError> {
    let request = decrypt_nip46_request(signer_keys, event)?;
    eprintln!("Method: {}", request.method);

    let result = match request.method.as_str() {
        "connect" => {
            if !auto_approve {
                eprintln!("Connect request from {}", client_pubkey);
                eprint!("Approve? (y/n): ");
                let stdin = std::io::stdin();
                let mut line = String::new();
                stdin
                    .lock()
                    .read_line(&mut line)
                    .map_err(|e| CliError::Other(format!("Failed to read stdin: {}", e)))?;
                if !line.trim().eq_ignore_ascii_case("y") {
                    return build_nip46_error_response(
                        signer_keys,
                        client_pubkey,
                        &request.id,
                        "Connection denied",
                    );
                }
            }
            connected_clients.insert(*client_pubkey);
            eprintln!("Client {} connected", client_pubkey);
            "ack".to_string()
        }
        "logout" => {
            if connected_clients.remove(client_pubkey) {
                eprintln!("Client {} logged out", client_pubkey);
                "ack".to_string()
            } else {
                return build_nip46_error_response(
                    signer_keys,
                    client_pubkey,
                    &request.id,
                    "Not connected",
                );
            }
        }
        "get_public_key" => {
            // Reject requests from unknown clients (NIP-46 security requirement)
            if !auto_approve && !connected_clients.contains(client_pubkey) {
                return build_nip46_error_response(
                    signer_keys,
                    client_pubkey,
                    &request.id,
                    "Not connected — call connect first",
                );
            }
            signer_keys.public_key().to_string()
        }
        "sign_event" => {
            // Reject requests from unknown clients
            if !auto_approve && !connected_clients.contains(client_pubkey) {
                return build_nip46_error_response(
                    signer_keys,
                    client_pubkey,
                    &request.id,
                    "Not connected — call connect first",
                );
            }
            let unsigned_json = request.params.first().ok_or_else(|| {
                CliError::Usage("sign_event requires event JSON param".to_string())
            })?;

            let unsigned: Value = serde_json::from_str(unsigned_json)
                .map_err(|e| CliError::Usage(format!("Invalid event JSON: {}", e)))?;

            if !auto_approve {
                eprintln!("Sign event request:");
                eprintln!("  Kind: {}", unsigned.get("kind").unwrap_or(&Value::Null));
                eprintln!(
                    "  Content: {}",
                    unsigned.get("content").unwrap_or(&Value::Null)
                );
                eprint!("Approve? (y/n): ");
                let stdin = std::io::stdin();
                let mut line = String::new();
                stdin
                    .lock()
                    .read_line(&mut line)
                    .map_err(|e| CliError::Other(format!("Failed to read stdin: {}", e)))?;
                if !line.trim().eq_ignore_ascii_case("y") {
                    return build_nip46_error_response(
                        signer_keys,
                        client_pubkey,
                        &request.id,
                        "Signing denied",
                    );
                }
            }

            let kind = unsigned
                .get("kind")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| CliError::Usage("Event missing kind".to_string()))?;
            let content = unsigned
                .get("content")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CliError::Usage("Event missing content".to_string()))?;
            let empty_array = Value::Array(vec![]);
            let tags_val = unsigned.get("tags").unwrap_or(&empty_array);
            let tags: Vec<Tag> = serde_json::from_value(tags_val.clone())
                .map_err(|e| CliError::Usage(format!("Invalid tags: {}", e)))?;

            let created_at = unsigned
                .get("created_at")
                .and_then(|v| v.as_u64())
                .map(Timestamp::from);

            let mut builder = EventBuilder::new(Kind::Custom(kind as u16), content).tags(tags);
            if let Some(ts) = created_at {
                builder = builder.custom_created_at(ts);
            }

            let signed = builder
                .sign_with_keys(signer_keys)
                .map_err(|e| CliError::Other(format!("Failed to sign event: {}", e)))?;

            serde_json::to_string(&signed)
                .map_err(|e| CliError::Other(format!("Failed to serialize signed event: {}", e)))?
        }
        "nip44_encrypt" => {
            // Reject requests from unknown clients
            if !auto_approve && !connected_clients.contains(client_pubkey) {
                return build_nip46_error_response(
                    signer_keys,
                    client_pubkey,
                    &request.id,
                    "Not connected — call connect first",
                );
            }
            let third_party_hex = request.params.first().ok_or_else(|| {
                CliError::Usage("nip44_encrypt requires pubkey param".to_string())
            })?;
            let plaintext = request.params.get(1).ok_or_else(|| {
                CliError::Usage("nip44_encrypt requires plaintext param".to_string())
            })?;

            let third_party = PublicKey::from_hex(third_party_hex)
                .map_err(|e| CliError::Usage(format!("Invalid pubkey: {}", e)))?;

            nip44::encrypt(
                signer_keys.secret_key(),
                &third_party,
                plaintext,
                Version::default(),
            )
            .map_err(|e| CliError::Other(format!("Encryption failed: {}", e)))?
        }
        "nip44_decrypt" => {
            // Reject requests from unknown clients
            if !auto_approve && !connected_clients.contains(client_pubkey) {
                return build_nip46_error_response(
                    signer_keys,
                    client_pubkey,
                    &request.id,
                    "Not connected — call connect first",
                );
            }
            let third_party_hex = request.params.first().ok_or_else(|| {
                CliError::Usage("nip44_decrypt requires pubkey param".to_string())
            })?;
            let ciphertext = request.params.get(1).ok_or_else(|| {
                CliError::Usage("nip44_decrypt requires ciphertext param".to_string())
            })?;

            let third_party = PublicKey::from_hex(third_party_hex)
                .map_err(|e| CliError::Usage(format!("Invalid pubkey: {}", e)))?;

            nip44::decrypt(signer_keys.secret_key(), &third_party, ciphertext)
                .map_err(|e| CliError::Other(format!("Decryption failed: {}", e)))?
        }
        "ping" => "pong".to_string(),
        _ => {
            return build_nip46_error_response(
                signer_keys,
                client_pubkey,
                &request.id,
                &format!("Method not supported: {}", request.method),
            );
        }
    };

    build_nip46_success_response(signer_keys, client_pubkey, &request.id, &result)
}

fn parse_bunker_url(url: &str) -> Result<(PublicKey, Vec<String>, Option<String>), CliError> {
    let url = url
        .strip_prefix("bunker://")
        .ok_or_else(|| {
            CliError::Usage(
                "Bunker URL must start with bunker:// (example: bunker://<pubkey>?relay=wss://relay.example.com)".to_string(),
            )
        })?;
    let parts: Vec<&str> = url.split('?').collect();
    let pubkey = PublicKey::from_hex(parts[0])
        .map_err(|e| CliError::Usage(format!("Invalid pubkey in bunker URL: {}", e)))?;

    let mut relays = Vec::new();
    let mut secret = None;

    if parts.len() > 1 {
        for param in parts[1].split('&') {
            let kv: Vec<&str> = param.split('=').collect();
            if kv.len() == 2 {
                match kv[0] {
                    "relay" => {
                        relays.push(
                            urlencoding::decode(kv[1])
                                .map_err(|e| {
                                    CliError::Usage(format!("Invalid relay URL encoding: {}", e))
                                })?
                                .to_string(),
                        );
                    }
                    "secret" => {
                        secret = Some(
                            urlencoding::decode(kv[1])
                                .map_err(|e| {
                                    CliError::Usage(format!("Invalid secret encoding: {}", e))
                                })?
                                .to_string(),
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    if relays.is_empty() {
        return Err(CliError::Usage(
            "Bunker URL must specify at least one relay (example: bunker://<pubkey>?relay=wss://relay.example.com)".to_string(),
        ));
    }

    Ok((pubkey, relays, secret))
}

fn build_nip46_request(
    keys: &Keys,
    remote_signer_pubkey: &PublicKey,
    request: &Nip46Request,
) -> Result<Event, CliError> {
    let plaintext = serde_json::to_string(request)
        .map_err(|e| CliError::Other(format!("Failed to serialize request: {}", e)))?;

    let encrypted = nip44::encrypt(
        keys.secret_key(),
        remote_signer_pubkey,
        &plaintext,
        Version::default(),
    )
    .map_err(|e| CliError::Other(format!("Encryption failed: {}", e)))?;

    EventBuilder::new(Kind::Custom(KIND_NOSTR_REMOTE_SIGNING as u16), encrypted)
        .tag(Tag::public_key(*remote_signer_pubkey))
        .sign_with_keys(keys)
        .map_err(|e| CliError::Other(format!("Failed to sign request: {}", e)))
}

fn build_nip46_success_response(
    keys: &Keys,
    client_pubkey: &PublicKey,
    request_id: &str,
    result: &str,
) -> Result<Event, CliError> {
    let response = Nip46Response {
        id: request_id.to_string(),
        result: Some(result.to_string()),
        error: None,
    };

    let plaintext = serde_json::to_string(&response)
        .map_err(|e| CliError::Other(format!("Failed to serialize response: {}", e)))?;

    let encrypted = nip44::encrypt(
        keys.secret_key(),
        client_pubkey,
        &plaintext,
        Version::default(),
    )
    .map_err(|e| CliError::Other(format!("Encryption failed: {}", e)))?;

    EventBuilder::new(Kind::Custom(KIND_NOSTR_REMOTE_SIGNING as u16), encrypted)
        .tag(Tag::public_key(*client_pubkey))
        .sign_with_keys(keys)
        .map_err(|e| CliError::Other(format!("Failed to sign response: {}", e)))
}

fn build_nip46_error_response(
    keys: &Keys,
    client_pubkey: &PublicKey,
    request_id: &str,
    error: &str,
) -> Result<Event, CliError> {
    let response = Nip46Response {
        id: request_id.to_string(),
        result: None,
        error: Some(error.to_string()),
    };

    let plaintext = serde_json::to_string(&response)
        .map_err(|e| CliError::Other(format!("Failed to serialize error response: {}", e)))?;

    let encrypted = nip44::encrypt(
        keys.secret_key(),
        client_pubkey,
        &plaintext,
        Version::default(),
    )
    .map_err(|e| CliError::Other(format!("Encryption failed: {}", e)))?;

    EventBuilder::new(Kind::Custom(KIND_NOSTR_REMOTE_SIGNING as u16), encrypted)
        .tag(Tag::public_key(*client_pubkey))
        .sign_with_keys(keys)
        .map_err(|e| CliError::Other(format!("Failed to sign error response: {}", e)))
}

fn decrypt_nip46_request(keys: &Keys, event: &Event) -> Result<Nip46Request, CliError> {
    let decrypted = nip44::decrypt(keys.secret_key(), &event.pubkey, &event.content)
        .map_err(|e| CliError::Other(format!("Decryption failed: {}", e)))?;

    serde_json::from_str(&decrypted)
        .map_err(|e| CliError::Other(format!("Failed to parse request: {}", e)))
}

fn decrypt_nip46_response(keys: &Keys, event: &Event) -> Result<Nip46Response, CliError> {
    let decrypted = nip44::decrypt(keys.secret_key(), &event.pubkey, &event.content)
        .map_err(|e| CliError::Other(format!("Decryption failed: {}", e)))?;

    serde_json::from_str(&decrypted)
        .map_err(|e| CliError::Other(format!("Failed to parse response: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bunker_url_basic() {
        let pubkey = "0d15ee9a9712dc0564134b9e32d45a4d4bb38137fb248beb510d2c95ee62804a";
        let url = format!("bunker://{}?relay=wss%3A%2F%2Frelay.example.com", pubkey);
        let result = parse_bunker_url(&url);
        assert!(result.is_ok());
        let (parsed_pubkey, relays, secret) = result.unwrap();
        assert_eq!(parsed_pubkey.to_hex(), pubkey);
        assert_eq!(relays.len(), 1);
        assert_eq!(relays[0], "wss://relay.example.com");
        assert!(secret.is_none());
    }

    #[test]
    fn test_parse_bunker_url_with_secret() {
        let pubkey = "0d15ee9a9712dc0564134b9e32d45a4d4bb38137fb248beb510d2c95ee62804a";
        let url = format!(
            "bunker://{}?relay=wss%3A%2F%2Frelay.example.com&secret=mysecret123",
            pubkey
        );
        let result = parse_bunker_url(&url);
        assert!(result.is_ok());
        let (_, _, secret) = result.unwrap();
        assert_eq!(secret, Some("mysecret123".to_string()));
    }

    #[test]
    fn test_parse_bunker_url_multiple_relays() {
        let pubkey = "0d15ee9a9712dc0564134b9e32d45a4d4bb38137fb248beb510d2c95ee62804a";
        let url = format!(
            "bunker://{}?relay=wss%3A%2F%2Frelay1.com&relay=wss%3A%2F%2Frelay2.com",
            pubkey
        );
        let result = parse_bunker_url(&url);
        assert!(result.is_ok());
        let (_, relays, _) = result.unwrap();
        assert_eq!(relays.len(), 2);
        assert_eq!(relays[0], "wss://relay1.com");
        assert_eq!(relays[1], "wss://relay2.com");
    }

    #[test]
    fn test_parse_bunker_url_missing_prefix() {
        let url = "0d15ee9a9712dc0564134b9e32d45a4d4bb38137fb248beb510d2c95ee62804a?relay=wss://relay.example.com";
        let result = parse_bunker_url(url);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_bunker_url_missing_relay() {
        let pubkey = "0d15ee9a9712dc0564134b9e32d45a4d4bb38137fb248beb510d2c95ee62804a";
        let url = format!("bunker://{}", pubkey);
        let result = parse_bunker_url(&url);
        assert!(result.is_err());
        match result {
            Err(CliError::Usage(msg)) => {
                assert!(msg.contains("relay"));
            }
            _ => panic!("Expected Usage error about missing relay"),
        }
    }

    #[test]
    fn test_nip46_request_serialization() {
        let request = Nip46Request {
            id: "test-id".to_string(),
            method: "ping".to_string(),
            params: vec![],
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"id\":\"test-id\""));
        assert!(json.contains("\"method\":\"ping\""));
        assert!(json.contains("\"params\":[]"));
    }

    #[test]
    fn test_nip46_response_serialization() {
        let response = Nip46Response {
            id: "test-id".to_string(),
            result: Some("pong".to_string()),
            error: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"id\":\"test-id\""));
        assert!(json.contains("\"result\":\"pong\""));
        assert!(!json.contains("error"));
    }

    #[test]
    fn test_nip46_error_response_serialization() {
        let response = Nip46Response {
            id: "test-id".to_string(),
            result: None,
            error: Some("test error".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"error\":\"test error\""));
        assert!(!json.contains("result"));
    }
}
