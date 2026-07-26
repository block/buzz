//! Rendezvous puller: drains an exported replication stream from a remote
//! portable relay node into a local one.
//!
//! The remote node (typically an always-on custodian such as the Cloudflare
//! adapter) authorizes this puller's key as a reader for the stream; the
//! local relay's peer trust binds the same stream to this puller's key as
//! transport provenance. The remote's cursor is persisted only after the
//! local relay returns checkpoint-safe receipts, so interrupted pulls resume
//! without loss and re-runs are idempotent by event ID.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Context};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use buzz_core::replication::{ReplicationBatch, ReplicationReceipt};
use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::hashes::Hash;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::types::Url;
use nostr::{EventBuilder, Keys, SecretKey, Tag};
use serde_json::json;
use uuid::Uuid;

const DEFAULT_BATCH_SIZE: usize = 100;

struct Config {
    from: String,
    to: String,
    source: String,
    key_file: PathBuf,
    cursor_file: PathBuf,
    batch_size: usize,
}

impl Config {
    fn from_args() -> anyhow::Result<Self> {
        let mut from = None;
        let mut to = None;
        let mut source = None;
        let mut key_file = None;
        let mut cursor_file = None;
        let mut batch_size = DEFAULT_BATCH_SIZE;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--from" => from = Some(next(&mut args, "--from")?),
                "--to" => to = Some(next(&mut args, "--to")?),
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
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }
        Ok(Self {
            from: from
                .context("--from <rendezvous base URL> is required")?
                .trim_end_matches('/')
                .to_string(),
            to: to
                .context("--to <local relay base URL> is required")?
                .trim_end_matches('/')
                .to_string(),
            source: source.context("--source <stream id> is required")?,
            key_file: key_file.context("--key <nsec hex file> is required")?,
            cursor_file: cursor_file.context("--cursor-file <path> is required")?,
            batch_size,
        })
    }
}

fn next(args: &mut impl Iterator<Item = String>, flag: &str) -> anyhow::Result<String> {
    args.next()
        .with_context(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "buzz-relay-pull — drain an exported stream from a rendezvous node

Usage:
  buzz-relay-pull --from URL --to URL --source ID --key PATH --cursor-file PATH [--batch N]

Options:
  --from URL          Remote node exporting the stream (POST /replication/read)
  --to URL            Local relay ingesting it (POST /replication)
  --source ID         Exported stream ID (reader-authorized at the remote,
                      peer-trusted for this key at the local relay)
  --key PATH          Node secret key used for both reads and local ingest
  --cursor-file PATH  Durable checkpoint in the remote's cursor space
  --batch N           Records per page (default: {DEFAULT_BATCH_SIZE})"
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

fn persist_cursor(path: &PathBuf, cursor: &str) -> anyhow::Result<()> {
    let mut file = std::fs::File::create(path)
        .with_context(|| format!("failed to open cursor file {}", path.display()))?;
    file.write_all(cursor.as_bytes())?;
    file.sync_data()?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_args()?;
    let secret = std::fs::read_to_string(&config.key_file)
        .with_context(|| format!("failed to read key file {}", config.key_file.display()))?;
    let keys = Keys::new(SecretKey::from_hex(secret.trim()).context("invalid secret key")?);
    let read_url = format!("{}/replication/read", config.from);
    let ingest_url = format!("{}/replication", config.to);
    let client = reqwest::Client::new();
    let mut cursor: Option<String> = match std::fs::read_to_string(&config.cursor_file) {
        Ok(saved) if !saved.trim().is_empty() => Some(saved.trim().to_string()),
        _ => None,
    };
    let mut pulled = 0usize;

    loop {
        let request_body = serde_json::to_vec(&json!({
            "source": config.source,
            "cursor": cursor,
            "limit": config.batch_size,
        }))?;
        let response = client
            .post(&read_url)
            .header(
                "authorization",
                nip98_header(&keys, &read_url, &request_body)?,
            )
            .header("content-type", "application/json")
            .body(request_body)
            .send()
            .await
            .context("rendezvous read failed")?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            bail!("rendezvous denied read: {status} {detail}");
        }
        let batch: ReplicationBatch = response
            .json()
            .await
            .context("rendezvous returned an invalid batch")?;

        if !batch.records.is_empty() {
            let ingest_body = serde_json::to_vec(&batch.records)?;
            let ingest = client
                .post(&ingest_url)
                .header(
                    "authorization",
                    nip98_header(&keys, &ingest_url, &ingest_body)?,
                )
                .header("content-type", "application/json")
                .body(ingest_body)
                .send()
                .await
                .context("local ingest failed")?;
            if !ingest.status().is_success() {
                let status = ingest.status();
                let detail = ingest.text().await.unwrap_or_default();
                bail!("local relay denied ingest: {status} {detail}");
            }
            let receipts: Vec<ReplicationReceipt> = ingest
                .json()
                .await
                .context("local relay returned invalid receipts")?;
            for receipt in &receipts {
                if !receipt.checkpoint_safe() {
                    bail!(
                        "pull halted at event {}: {:?}",
                        receipt.event_id,
                        receipt.outcome
                    );
                }
            }
            pulled += receipts.len();
        }

        persist_cursor(&config.cursor_file, batch.next_cursor.as_str())?;
        cursor = Some(batch.next_cursor.as_str().to_string());
        if batch.caught_up {
            println!(
                "caught up: {pulled} records pulled, cursor {}",
                batch.next_cursor.as_str()
            );
            return Ok(());
        }
        println!(
            "pulled {pulled} records so far, cursor {}",
            batch.next_cursor.as_str()
        );
    }
}
