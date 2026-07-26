//! One-shot replication pusher: drains a local relay journal to a portable
//! relay destination over HTTP.
//!
//! Reads ordered pages from the local NDJSON journal through the portable
//! replication source port, authenticates each request with a payload-bound
//! NIP-98 proof signed by the node key, and advances a durable cursor file
//! only through checkpoint-safe receipts. Re-running after interruption
//! resumes from the persisted cursor; the destination is idempotent by
//! event ID either way.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_core::replication::ReplicationSourcePort;
use buzz_core::replication::{ReplicationCursor, ReplicationReceipt, ReplicationSourceId};
use buzz_local_relay::{EventStore, LocalReplicationSource, StorageMode};
use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::hashes::Hash;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::types::Url;
use nostr::Filter;
use nostr::{EventBuilder, Keys, SecretKey, Tag};
use uuid::Uuid;

const DEFAULT_BATCH_SIZE: usize = 100;

struct Config {
    data: PathBuf,
    destination: String,
    source: ReplicationSourceId,
    key_file: PathBuf,
    cursor_file: PathBuf,
    batch_size: usize,
    filter: Option<Vec<Filter>>,
}

impl Config {
    fn from_args() -> anyhow::Result<Self> {
        let mut data = None;
        let mut destination = None;
        let mut source = None;
        let mut key_file = None;
        let mut cursor_file = None;
        let mut batch_size = DEFAULT_BATCH_SIZE;
        let mut filter_json: Option<String> = None;
        let mut streams_file: Option<PathBuf> = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--data" => data = Some(PathBuf::from(next(&mut args, "--data")?)),
                "--to" => destination = Some(next(&mut args, "--to")?),
                "--source" => source = Some(next(&mut args, "--source")?),
                "--key" => key_file = Some(PathBuf::from(next(&mut args, "--key")?)),
                "--cursor-file" => {
                    cursor_file = Some(PathBuf::from(next(&mut args, "--cursor-file")?))
                }
                "--batch" => {
                    batch_size = next(&mut args, "--batch")?
                        .parse()
                        .context("--batch requires a positive integer")?
                }
                "--filter" => filter_json = Some(next(&mut args, "--filter")?),
                "--streams" => streams_file = Some(PathBuf::from(next(&mut args, "--streams")?)),
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }
        let data = data.context("--data <journal path> is required")?;
        let source_id = source.context("--source <stream id> is required")?;
        if filter_json.is_some() && streams_file.is_some() {
            bail!("--filter and --streams are mutually exclusive");
        }
        let filter = if let Some(json) = filter_json {
            Some(
                serde_json::from_str::<Vec<Filter>>(&json)
                    .context("--filter must be a JSON array of NIP-01 filters")?,
            )
        } else if let Some(path) = streams_file {
            let raw = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read streams file {}", path.display()))?;
            let streams: serde_json::Value =
                serde_json::from_str(&raw).context("streams file is not valid JSON")?;
            let entry = streams.get(&source_id).with_context(|| {
                format!(
                    "stream {source_id:?} not declared in {} (declared: {})",
                    path.display(),
                    streams
                        .as_object()
                        .map(|object| object.keys().cloned().collect::<Vec<_>>().join(", "))
                        .unwrap_or_default()
                )
            })?;
            // Selection is explicit: a whole-journal export must be declared
            // as {"mirror": true}; a bare null filter is rejected.
            if entry["mirror"] == serde_json::Value::Bool(true) {
                None
            } else {
                match &entry["filter"] {
                    serde_json::Value::Array(_) => Some(
                        serde_json::from_value::<Vec<Filter>>(entry["filter"].clone())
                            .context("stream filter must be a JSON array of NIP-01 filters")?,
                    ),
                    _ => bail!(
                        "stream {source_id:?} must declare {{\"filter\": [...]}} or {{\"mirror\": true}}"
                    ),
                }
            }
        } else {
            None
        };
        let cursor_file = cursor_file.unwrap_or_else(|| {
            let mut path = data.clone().into_os_string();
            path.push(".push-cursor");
            PathBuf::from(path)
        });
        Ok(Self {
            data,
            destination: destination
                .context("--to <destination base URL> is required")?
                .trim_end_matches('/')
                .to_string(),
            source: ReplicationSourceId::new(source_id),
            key_file: key_file.context("--key <nsec hex file> is required")?,
            cursor_file,
            batch_size,
            filter,
        })
    }
}

fn next(args: &mut impl Iterator<Item = String>, flag: &str) -> anyhow::Result<String> {
    args.next()
        .with_context(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "buzz-relay-push — drain a local relay journal to a portable relay destination

Usage:
  buzz-relay-push --data PATH --to URL --source ID --key PATH \\
                  [--cursor-file PATH] [--batch N] [--filter JSON | --streams PATH]

Options:
  --data PATH         Local append-only event journal (NDJSON)
  --to URL            Destination base URL exposing POST /replication
  --source ID         Destination-configured replication source stream ID
  --key PATH          File containing the node secret key (hex)
  --cursor-file PATH  Durable checkpoint (default: <data>.push-cursor)
  --batch N           Records per request (default: {DEFAULT_BATCH_SIZE})
  --filter JSON       Selective stream: NIP-01 filter array for this source ID
  --streams PATH      JSON file mapping source IDs to {{\"filter\": ...}} entries

A stream's filter is part of its identity: to change the filter, mint a new
source ID and start from a fresh cursor."
    );
}

fn nip98_header(keys: &Keys, url: &str, body: &[u8]) -> anyhow::Result<String> {
    let http_data =
        HttpData::new(Url::parse(url)?, HttpMethod::POST).payload(Sha256Hash::hash(body));
    let nonce = Uuid::new_v4().to_string();
    let event = EventBuilder::http_auth(http_data)
        .tag(Tag::parse(["nonce", nonce.as_str()]).context("nonce tag parses")?)
        .sign_with_keys(keys)?;
    Ok(format!(
        "Nostr {}",
        BASE64.encode(serde_json::to_vec(&event)?)
    ))
}

fn persist_cursor(path: &PathBuf, cursor: &ReplicationCursor) -> anyhow::Result<()> {
    let mut file = std::fs::File::create(path)
        .with_context(|| format!("failed to open cursor file {}", path.display()))?;
    file.write_all(cursor.as_str().as_bytes())?;
    file.sync_data()?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_args()?;
    let secret = std::fs::read_to_string(&config.key_file)
        .with_context(|| format!("failed to read key file {}", config.key_file.display()))?;
    let keys = Keys::new(SecretKey::from_hex(secret.trim()).context("invalid node secret key")?);
    let store = Arc::new(
        EventStore::open(StorageMode::Durable(config.data.clone()))
            .await
            .context("failed to open local journal")?,
    );
    let source = match config.filter.clone() {
        Some(filters) => LocalReplicationSource::with_filter(config.source.clone(), store, filters),
        None => LocalReplicationSource::new(config.source.clone(), store),
    };
    let mut cursor = match std::fs::read_to_string(&config.cursor_file) {
        Ok(saved) if !saved.trim().is_empty() => {
            Some(ReplicationCursor::new(saved.trim().to_string()))
        }
        _ => None,
    };
    let endpoint = format!("{}/replication", config.destination);
    let client = reqwest::Client::new();
    let mut pushed = 0usize;

    loop {
        let batch = source.read_batch(cursor.clone(), config.batch_size).await?;
        // Filtered streams may return empty pages mid-journal; only the
        // source's caught_up signal ends the run.
        if batch.records.is_empty() {
            persist_cursor(&config.cursor_file, &batch.next_cursor)?;
            cursor = Some(batch.next_cursor.clone());
            if batch.caught_up {
                println!(
                    "caught up: {pushed} records pushed, cursor {}",
                    batch.next_cursor.as_str()
                );
                return Ok(());
            }
            continue;
        }

        let body = serde_json::to_vec(&batch.records)?;
        let response = client
            .post(&endpoint)
            .header("authorization", nip98_header(&keys, &endpoint, &body)?)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .context("replication request failed")?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("destination denied replication: {status} {detail}");
        }
        let receipts: Vec<ReplicationReceipt> = response
            .json()
            .await
            .context("destination returned an invalid receipt batch")?;

        let mut advanced = cursor.clone();
        for receipt in &receipts {
            if !receipt.checkpoint_safe() {
                if let Some(safe) = advanced.as_ref() {
                    persist_cursor(&config.cursor_file, safe)?;
                }
                bail!(
                    "replication halted at event {}: {:?} (cursor checkpoint {})",
                    receipt.event_id,
                    receipt.outcome,
                    advanced
                        .as_ref()
                        .map(ReplicationCursor::as_str)
                        .unwrap_or("<start>"),
                );
            }
            advanced = Some(receipt.cursor.clone());
            pushed += 1;
        }
        advanced.clone().context("receipts advanced no cursor")?;
        // Every receipt was checkpoint-safe, so the scanned-through position
        // (which may extend past the last matched record) is durable.
        persist_cursor(&config.cursor_file, &batch.next_cursor)?;
        println!(
            "pushed {} records through cursor {}",
            receipts.len(),
            batch.next_cursor.as_str()
        );
        cursor = Some(batch.next_cursor.clone());
        if batch.caught_up {
            println!("caught up: {pushed} records pushed");
            return Ok(());
        }
    }
}
