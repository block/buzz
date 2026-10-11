//! `buzz trust` — NIP-FP work fingerprints (`docs/nips/NIP-FP.md`).
//!
//! `fingerprint` signs a selection and prints it as a transport bundle
//! (pre-registration). `verify` runs the SDK's sans-IO verifier against each
//! origin's `/query` and NIP-11 document, so every signature it reports was
//! checked on bytes fetched by this viewer.

use std::collections::BTreeMap;

use buzz_sdk::trust::{self, Policy, Request, Response, Selected, Verifier};
use nostr::{EventId, PublicKey, Timestamp};

use crate::client::{normalize_relay_url, BuzzClient};
use crate::error::CliError;
use crate::TrustCmd;

/// Upper bound on discovery rounds. The verifier needs four; this guards
/// against a misbehaving state machine, not a relay.
const MAX_ROUNDS: usize = 8;

pub async fn dispatch(cmd: TrustCmd, client: &BuzzClient) -> Result<(), CliError> {
    match cmd {
        TrustCmd::Fingerprint { d, select } => cmd_fingerprint(client, &d, &select),
        TrustCmd::Verify {
            bundle,
            cutoff,
            viewer_known,
        } => cmd_verify(client, &bundle, cutoff, &viewer_known).await,
    }
}

/// Parse `<event-id>[=<origin>]`; the origin defaults to the current relay.
fn parse_selection(raw: &str, default_origin: &str) -> Result<Selected, CliError> {
    let (id, origin) = match raw.split_once('=') {
        Some((id, origin)) => (id, normalize_relay_url(origin)),
        None => (raw, default_origin.to_owned()),
    };
    let id = EventId::from_hex(id.trim())
        .map_err(|e| CliError::Usage(format!("--select {raw}: not an event id: {e}")))?;
    Ok(Selected { id, origin })
}

fn cmd_fingerprint(client: &BuzzClient, d: &str, select: &[String]) -> Result<(), CliError> {
    let selections = select
        .iter()
        .map(|raw| parse_selection(raw, client.relay_url()))
        .collect::<Result<Vec<_>, _>>()?;
    let builder = trust::build_work_fingerprint(d, &selections)
        .map_err(|e| CliError::Usage(e.to_string()))?;
    let event = client.sign_event(builder)?;
    print_json(&trust::transport_bundle(&event))
}

fn read_bundle(path: &str) -> Result<serde_json::Value, CliError> {
    let raw = if path == "-" {
        std::io::read_to_string(std::io::stdin())
            .map_err(|e| CliError::Other(format!("failed to read stdin: {e}")))?
    } else {
        std::fs::read_to_string(path)
            .map_err(|e| CliError::Usage(format!("failed to read {path}: {e}")))?
    };
    serde_json::from_str(&raw).map_err(|e| CliError::Usage(format!("bundle is not JSON: {e}")))
}

async fn cmd_verify(
    client: &BuzzClient,
    bundle: &str,
    cutoff: Option<u64>,
    viewer_known: &[String],
) -> Result<(), CliError> {
    let input = read_bundle(bundle)?;
    let policy = if viewer_known.is_empty() {
        Policy::default()
    } else {
        let keys = viewer_known
            .iter()
            .map(|k| {
                PublicKey::parse(k).map_err(|e| CliError::Usage(format!("--viewer-known {k}: {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Policy::viewer_known(keys)
    };
    let cutoff = cutoff.unwrap_or_else(|| Timestamp::now().as_secs());
    let mut verifier =
        Verifier::new(&input, policy, cutoff).map_err(|e| CliError::Usage(e.to_string()))?;

    // One client per origin. Only the configured relay gets this agent's
    // owner attestation header; other origins see the bare key.
    let mut clients: BTreeMap<String, BuzzClient> = BTreeMap::new();
    for _ in 0..MAX_ROUNDS {
        let queries = verifier.next_queries();
        if queries.is_empty() {
            break;
        }
        for query in queries {
            let origin = normalize_relay_url(&query.origin);
            let target = if origin == client.relay_url() {
                client
            } else {
                if !clients.contains_key(&origin) {
                    let keys = client.keys().clone();
                    clients.insert(
                        origin.clone(),
                        BuzzClient::new(origin.clone(), keys, None, None)?,
                    );
                }
                clients
                    .get(&origin)
                    .ok_or_else(|| CliError::Other("client cache".into()))?
            };
            let response = fetch(target, &query.request).await;
            verifier
                .ingest(query.scope, response)
                .map_err(|e| CliError::Other(e.to_string()))?;
        }
    }
    print_json(&verifier.finish())
}

/// Answer one verifier request. Failures become [`Response::Failed`]: the
/// report marks what they would have found as unknown.
async fn fetch(client: &BuzzClient, request: &Request) -> Response {
    match request {
        Request::RelayInfo => match client.get_public("/").await {
            Ok(raw) => Response::RelayInfo(
                serde_json::from_str::<serde_json::Value>(&raw)
                    .ok()
                    .and_then(|doc| doc.get("self")?.as_str().map(str::to_owned))
                    .and_then(|hex| PublicKey::from_hex(&hex).ok()),
            ),
            Err(e) => Response::Failed(e.to_string()),
        },
        Request::Events(filter) => {
            let filter = match serde_json::to_value(filter) {
                Ok(filter) => filter,
                Err(e) => return Response::Failed(format!("filter serialization failed: {e}")),
            };
            match client.query(&filter).await {
                Ok(raw) => match serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
                    Ok(events) => Response::Events(events),
                    Err(e) => Response::Failed(format!("response is not an event array: {e}")),
                },
                Err(e) => Response::Failed(e.to_string()),
            }
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    let out = serde_json::to_string_pretty(value)
        .map_err(|e| CliError::Other(format!("serialization failed: {e}")))?;
    println!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "c11048f3136e1dfd5c0b79267bcf6ea11630ea64e25bd3808d55a1c337b068db";

    #[test]
    fn selection_origin_defaults_to_current_relay_and_normalizes() {
        let s = parse_selection(ID, "https://a.example").expect("default origin");
        assert_eq!(s.origin, "https://a.example");
        let s = parse_selection(&format!("{ID}=wss://b.example/"), "https://a.example")
            .expect("explicit origin");
        assert_eq!(s.origin, "https://b.example");
        assert!(parse_selection("nope", "https://a.example").is_err());
    }
}
