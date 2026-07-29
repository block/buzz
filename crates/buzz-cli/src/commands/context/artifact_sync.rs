use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nostr::Event;
use serde::Serialize;

use super::profile::{ProfileEnvironment, ResolvedProfile};
use super::runtime::{event_builder, hostname, query_events, tag, ContextRuntime};
use crate::client::{normalize_artifact_sha256, BuzzClient};
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

pub async fn put(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    file: &Path,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let body = read_artifact(file)?;
    let receipt = runtime
        .local_artifact_client()?
        .put_artifact(bytes::Bytes::from(body))
        .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt)
            .map_err(|error| CliError::Other(format!("receipt serialization failed: {error}")))?
    );
    Ok(())
}

pub async fn get(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    sha256: &str,
    out: Option<&Path>,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let sha256 = normalize_artifact_sha256(sha256)?;
    let local = runtime.local_artifact_client()?;
    let bytes = match local.get_artifact(&sha256).await {
        Ok(bytes) => bytes,
        Err(local_error) => {
            runtime
                .cloud_artifact_client()
                .map_err(|_| local_error)?
                .get_artifact(&sha256)
                .await?
        }
    };
    let out = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(&sha256));
    std::fs::write(&out, &bytes)
        .map_err(|error| CliError::Other(format!("could not write {}: {error}", out.display())))?;
    println!(
        "saved artifact {sha256} to {} ({} bytes)",
        out.display(),
        bytes.len()
    );
    Ok(())
}

pub async fn head(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    sha256: &str,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let sha256 = normalize_artifact_sha256(sha256)?;
    if runtime
        .local_artifact_client()?
        .head_artifact(&sha256)
        .await?
    {
        println!("present {sha256}");
        Ok(())
    } else {
        println!("absent {sha256}");
        Err(CliError::NotFound(format!("artifact {sha256} is absent")))
    }
}

pub async fn announce(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    file: &Path,
    message: Option<&str>,
) -> Result<String, CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let body = read_artifact(file)?;
    let local_receipt = runtime
        .local_artifact_client()?
        .put_artifact(bytes::Bytes::from(body.clone()))
        .await?;
    let rendezvous_receipt = runtime
        .cloud_artifact_uploader_client()?
        .put_artifact(bytes::Bytes::from(body))
        .await?;
    if local_receipt.sha256 != rendezvous_receipt.sha256 {
        return Err(CliError::Other(format!(
            "artifact identity changed between local and rendezvous custody: {} != {}",
            local_receipt.sha256, rendezvous_receipt.sha256
        )));
    }

    let sha256 = local_receipt.sha256;
    let context = runtime.default_context()?;
    let (_, role) = runtime.local_event_client()?;
    let identity = runtime.identity_label(role);
    let machine = hostname();
    let default_message = format!(
        "artifact {} ({} bytes)",
        file.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unnamed"),
        local_receipt.size
    );
    let event_id = runtime
        .post_builder(event_builder(
            message.unwrap_or(&default_message),
            [
                tag(&["x", &sha256])?,
                tag(&["h", context])?,
                tag(&["agent", &identity])?,
                tag(&["machine", &machine])?,
            ],
            None,
        ))
        .await?;
    super::journal::sync(profile, environment, false)?;
    verify_rendezvous_manifest(profile, environment, &sha256, context).await?;
    println!("sha256: {sha256}");
    println!("manifest accepted: {event_id}");
    println!("rendezvous custody: verified");
    println!("context: {context}");
    Ok(sha256)
}

pub async fn verify_rendezvous_manifest(
    profile: &ResolvedProfile,
    environment: &ProfileEnvironment,
    sha256: &str,
    context: &str,
) -> Result<(), CliError> {
    let runtime = ContextRuntime::new(profile, environment)?;
    let sha256 = normalize_artifact_sha256(sha256)?;
    let reader = runtime.cloud_reader_client()?;
    let manifests = query_events(
        &reader,
        &[serde_json::json!({
            "kinds": [nostr::Kind::TextNote.as_u16()],
            "#x": [sha256],
            "#h": [context],
            "limit": 10,
        })],
    )
    .await?;
    if manifests.is_empty() {
        return Err(CliError::NotFound(format!(
            "rendezvous has no signed manifest for artifact {sha256} in context {context}"
        )));
    }
    runtime
        .cloud_artifact_client()?
        .get_artifact(&sha256)
        .await?;
    Ok(())
}

fn read_artifact(file: &Path) -> Result<Vec<u8>, CliError> {
    let metadata = std::fs::metadata(file)
        .map_err(|error| CliError::Usage(format!("cannot access {}: {error}", file.display())))?;
    if !metadata.is_file() {
        return Err(CliError::Usage(format!("{} is not a file", file.display())));
    }
    std::fs::read(file)
        .map_err(|error| CliError::Usage(format!("failed to read {}: {error}", file.display())))
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
