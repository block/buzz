use std::collections::BTreeSet;

use nostr::Event;
use serde::Serialize;

use super::profile::{ProfileEnvironment, ResolvedProfile};
use crate::client::BuzzClient;
use crate::error::CliError;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ArtifactSyncReport {
    pub profile: String,
    pub source: String,
    pub destination: String,
    pub source_reader_pubkey: String,
    pub destination_owner_pubkey: String,
    pub dry_run: bool,
    pub referenced: usize,
    pub present: usize,
    pub fetched: usize,
    pub missing: Vec<String>,
}

pub async fn sync(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    dry_run: bool,
) -> Result<ArtifactSyncReport, CliError> {
    profile.require_ready()?;
    let source = profile.file.relays.rendezvous.as_deref().ok_or_else(|| {
        CliError::Usage(format!(
            "profile {} does not configure relays.rendezvous",
            profile.name
        ))
    })?;
    let source_client = profile.client_for("artifact_source_reader", source, environment)?;
    let destination_client = profile.client_for(
        "artifact_destination_owner",
        &profile.file.relays.local,
        environment,
    )?;
    let journal = std::fs::read_to_string(&profile.journal).map_err(|error| {
        CliError::Other(format!(
            "could not read profile journal {}: {error}",
            profile.journal.display()
        ))
    })?;
    let referenced = referenced_hashes(&journal);
    sync_clients(
        &profile.name,
        source,
        &profile.file.relays.local,
        &source_client,
        &destination_client,
        referenced,
        dry_run,
    )
    .await
}

async fn sync_clients(
    profile: &str,
    source: &str,
    destination: &str,
    source_client: &BuzzClient,
    destination_client: &BuzzClient,
    referenced: BTreeSet<String>,
    dry_run: bool,
) -> Result<ArtifactSyncReport, CliError> {
    let mut present = 0usize;
    let mut fetched = 0usize;
    let mut missing = Vec::new();
    for hash in &referenced {
        if destination_client.head_artifact(hash).await? {
            present += 1;
            continue;
        }
        missing.push(hash.clone());
        if dry_run {
            continue;
        }
        let bytes = source_client.get_artifact(hash).await?;
        let receipt = destination_client.put_artifact(bytes).await?;
        if receipt.sha256 != *hash {
            return Err(CliError::Other(format!(
                "destination receipt changed artifact identity: expected {hash}, got {}",
                receipt.sha256
            )));
        }
        fetched += 1;
    }
    Ok(ArtifactSyncReport {
        profile: profile.into(),
        source: source.into(),
        destination: destination.into(),
        source_reader_pubkey: source_client.keys().public_key().to_hex(),
        destination_owner_pubkey: destination_client.keys().public_key().to_hex(),
        dry_run,
        referenced: referenced.len(),
        present,
        fetched,
        missing,
    })
}

fn referenced_hashes(journal: &str) -> BTreeSet<String> {
    let mut hashes = BTreeSet::new();
    for line in journal.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<Event>(line) else {
            continue;
        };
        for tag in event.tags.iter() {
            let slice = tag.as_slice();
            if slice.first().map(String::as_str) != Some("x") {
                continue;
            }
            let Some(value) = slice.get(1) else {
                continue;
            };
            if value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                hashes.insert(value.clone());
            }
        }
    }
    hashes
}

pub fn print_report(report: &ArtifactSyncReport, json: bool) -> Result<(), CliError> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).map_err(|error| {
                CliError::Other(format!("artifact sync serialization failed: {error}"))
            })?
        );
        return Ok(());
    }
    for hash in &report.missing {
        if report.dry_run {
            println!("missing {hash}");
        }
    }
    println!(
        "artifact sync complete: {} referenced, {} already present, {} fetched",
        report.referenced, report.present, report.fetched
    );
    println!("  source reader       {}", report.source_reader_pubkey);
    println!("  destination owner   {}", report.destination_owner_pubkey);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
    use sha2::{Digest, Sha256};

    use super::*;

    #[derive(Clone)]
    struct Store {
        expected_pubkey: String,
        blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
        uploads: Arc<Mutex<usize>>,
    }

    async fn get_blob(
        State(store): State<Store>,
        Path(hash): Path<String>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        if signer(&headers).as_deref() != Some(store.expected_pubkey.as_str()) {
            return (StatusCode::FORBIDDEN, Vec::new());
        }
        match store.blobs.lock().expect("store lock").get(&hash).cloned() {
            Some(bytes) => (StatusCode::OK, bytes),
            None => (StatusCode::NOT_FOUND, Vec::new()),
        }
    }

    async fn head_blob(
        State(store): State<Store>,
        Path(hash): Path<String>,
        headers: HeaderMap,
    ) -> StatusCode {
        if signer(&headers).as_deref() != Some(store.expected_pubkey.as_str()) {
            return StatusCode::FORBIDDEN;
        }
        if store.blobs.lock().expect("store lock").contains_key(&hash) {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        }
    }

    async fn put_blob(
        State(store): State<Store>,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl IntoResponse {
        if signer(&headers).as_deref() != Some(store.expected_pubkey.as_str()) {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({"error":"wrong signer"})),
            );
        }
        let hash = hex::encode(Sha256::digest(&body));
        store
            .blobs
            .lock()
            .expect("store lock")
            .insert(hash.clone(), body.to_vec());
        *store.uploads.lock().expect("upload lock") += 1;
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "sha256": hash,
                "size": body.len(),
                "url": format!("/artifacts/{hash}")
            })),
        )
    }

    fn signer(headers: &HeaderMap) -> Option<String> {
        let encoded = headers
            .get("authorization")?
            .to_str()
            .ok()?
            .strip_prefix("Nostr ")?;
        let decoded = STANDARD.decode(encoded).ok()?;
        let event = Event::from_json(std::str::from_utf8(&decoded).ok()?).ok()?;
        event.verify().ok()?;
        Some(event.pubkey.to_hex())
    }

    async fn serve(store: Store) -> String {
        let app = Router::new()
            .route("/artifacts/{hash}", get(get_blob).head(head_blob))
            .route("/artifacts", post(put_blob))
            .with_state(store);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server");
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn split_identity_sync_converges_on_the_second_pass() {
        let source_keys = Keys::generate();
        let destination_keys = Keys::generate();
        assert_ne!(source_keys.public_key(), destination_keys.public_key());
        let bytes = b"split identity artifact".to_vec();
        let hash = hex::encode(Sha256::digest(&bytes));
        let source_store = Store {
            expected_pubkey: source_keys.public_key().to_hex(),
            blobs: Arc::new(Mutex::new(HashMap::from([(hash.clone(), bytes)]))),
            uploads: Arc::new(Mutex::new(0)),
        };
        let destination_store = Store {
            expected_pubkey: destination_keys.public_key().to_hex(),
            blobs: Arc::new(Mutex::new(HashMap::new())),
            uploads: Arc::new(Mutex::new(0)),
        };
        let source_url = serve(source_store).await;
        let destination_url = serve(destination_store.clone()).await;
        let source_client =
            BuzzClient::new(source_url.clone(), source_keys, None, None).expect("source client");
        let destination_client =
            BuzzClient::new(destination_url.clone(), destination_keys, None, None)
                .expect("destination client");

        let first = sync_clients(
            "enterprise",
            &source_url,
            &destination_url,
            &source_client,
            &destination_client,
            BTreeSet::from([hash.clone()]),
            false,
        )
        .await
        .expect("first pass");
        assert_eq!(first.fetched, 1);
        assert_eq!(first.present, 0);

        let second = sync_clients(
            "enterprise",
            &source_url,
            &destination_url,
            &source_client,
            &destination_client,
            BTreeSet::from([hash]),
            false,
        )
        .await
        .expect("second pass");
        assert_eq!(second.fetched, 0);
        assert_eq!(second.present, 1);
        assert_eq!(*destination_store.uploads.lock().expect("uploads"), 1);
    }

    #[test]
    fn journal_manifest_accepts_only_canonical_lowercase_hashes() {
        let keys = Keys::generate();
        let good = "a".repeat(64);
        let upper = "B".repeat(64);
        let event = EventBuilder::new(Kind::TextNote, "manifest")
            .tags([
                Tag::parse(["x", good.as_str()]).expect("tag"),
                Tag::parse(["x", upper.as_str()]).expect("tag"),
                Tag::parse(["x", "short"]).expect("tag"),
            ])
            .sign_with_keys(&keys)
            .expect("sign");
        let journal = format!("{}\n", event.as_json());
        assert_eq!(referenced_hashes(&journal), BTreeSet::from([good]));
    }
}
