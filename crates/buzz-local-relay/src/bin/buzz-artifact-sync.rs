//! Artifact-sync walker: fetch content-addressed blobs referenced by events
//! this node already holds, from a peer that serves them.
//!
//! Events are the manifest. The walker reads the local durable journal for
//! `x` tags (content-addressed references), skips any blob already present,
//! and fetches the rest from a peer's artifact store — verifying each by
//! content on arrival. There is no cursor and no reconciliation protocol:
//! possession is idempotent by hash, so a re-run is a no-op and an
//! interrupted run simply resumes by re-observing what is still missing.

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{bail, Context};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use nostr::hashes::sha256::Hash as Sha256Hash;
use nostr::hashes::Hash;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::types::Url;
use nostr::{Event, EventBuilder, Keys, SecretKey, Tag};
use uuid::Uuid;

struct Config {
    data: PathBuf,
    from: String,
    to: String,
    source_key_file: PathBuf,
    destination_key_file: PathBuf,
    dry_run: bool,
}

impl Config {
    fn from_args() -> anyhow::Result<Self> {
        Self::from_iter(std::env::args().skip(1))
    }

    fn from_iter(mut args: impl Iterator<Item = String>) -> anyhow::Result<Self> {
        let mut data = None;
        let mut from = None;
        let mut to = None;
        let mut collapsed_key_file = None;
        let mut source_key_file = None;
        let mut destination_key_file = None;
        let mut dry_run = false;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--data" => data = Some(PathBuf::from(next(&mut args, "--data")?)),
                "--from" => from = Some(next(&mut args, "--from")?),
                "--to" => to = Some(next(&mut args, "--to")?),
                "--key" => {
                    collapsed_key_file = Some(PathBuf::from(next(&mut args, "--key")?));
                }
                "--source-key" => {
                    source_key_file = Some(PathBuf::from(next(&mut args, "--source-key")?));
                }
                "--destination-key" => {
                    destination_key_file =
                        Some(PathBuf::from(next(&mut args, "--destination-key")?));
                }
                "--dry-run" => dry_run = true,
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                other => bail!("unknown argument: {other}"),
            }
        }
        let (source_key_file, destination_key_file) =
            match (collapsed_key_file, source_key_file, destination_key_file) {
                (Some(key), None, None) => (key.clone(), key),
                (None, Some(source), Some(destination)) => (source, destination),
                (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                    bail!("--key cannot be combined with --source-key or --destination-key")
                }
                (None, Some(_), None) | (None, None, Some(_)) => {
                    bail!("--source-key and --destination-key must be provided together")
                }
                (None, None, None) => {
                    bail!("provide --source-key and --destination-key (or legacy --key)")
                }
            };
        Ok(Self {
            data: data.context("--data <journal path> is required")?,
            from: from
                .context("--from <peer base URL> is required")?
                .trim_end_matches('/')
                .to_string(),
            to: to
                .context("--to <local base URL> is required")?
                .trim_end_matches('/')
                .to_string(),
            source_key_file,
            destination_key_file,
            dry_run,
        })
    }
}

fn next(args: &mut impl Iterator<Item = String>, flag: &str) -> anyhow::Result<String> {
    args.next()
        .with_context(|| format!("{flag} requires a value"))
}

fn print_help() {
    println!(
        "buzz-artifact-sync — fetch blobs referenced by events this node holds

Usage:
  buzz-artifact-sync --data PATH --from URL --to URL \
    --source-key PATH --destination-key PATH [--dry-run]
  buzz-artifact-sync --data PATH --from URL --to URL --key PATH [--dry-run]

Options:
  --data PATH   Local durable journal to walk for x-tag references
  --from URL    Peer whose artifact store serves the missing blobs
  --to URL      Local relay whose artifact store receives them
  --source-key PATH       Source-reader credential for peer GET
  --destination-key PATH  Destination-owner credential for local HEAD/POST
  --key PATH              Legacy explicit collapsed-role compatibility
  --dry-run               Report the missing set without fetching"
    );
}

fn header(keys: &Keys, method: HttpMethod, url: &str, body: &[u8]) -> anyhow::Result<String> {
    let data = HttpData::new(Url::parse(url)?, method).payload(Sha256Hash::hash(body));
    let nonce = Uuid::new_v4().to_string();
    let event = EventBuilder::http_auth(data)
        .tag(Tag::parse(["nonce", nonce.as_str()]).context("nonce tag parses")?)
        .sign_with_keys(keys)?;
    Ok(format!(
        "Nostr {}",
        BASE64.encode(serde_json::to_vec(&event)?)
    ))
}

/// Collects every 64-hex content reference carried in an `x` tag.
fn referenced_hashes(journal: &str) -> BTreeSet<String> {
    let mut hashes = BTreeSet::new();
    for line in journal.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(event) = serde_json::from_str::<Event>(line) else {
            continue;
        };
        for tag in event.tags.iter() {
            let slice = tag.as_slice();
            if slice.first().map(String::as_str) == Some("x") {
                if let Some(value) = slice.get(1) {
                    let value = value.to_ascii_lowercase();
                    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
                        hashes.insert(value);
                    }
                }
            }
        }
    }
    hashes
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_args()?;
    let source_keys = read_keys(&config.source_key_file, "source reader")?;
    let destination_keys = read_keys(&config.destination_key_file, "destination owner")?;
    let journal = std::fs::read_to_string(&config.data)
        .with_context(|| format!("failed to read journal {}", config.data.display()))?;
    let referenced = referenced_hashes(&journal);
    let client = reqwest::Client::new();
    let mut fetched = 0usize;
    let mut present = 0usize;

    for hash in &referenced {
        let local_url = format!("{}/artifacts/{hash}", config.to);
        let local = client
            .head(&local_url)
            .header(
                "authorization",
                header(&destination_keys, HttpMethod::GET, &local_url, b"")?,
            )
            .send()
            .await
            .context("local existence probe failed")?;
        if local.status().is_success() {
            present += 1;
            continue;
        }
        if config.dry_run {
            println!("missing {hash}");
            continue;
        }

        let remote_url = format!("{}/artifacts/{hash}", config.from);
        let remote = client
            .get(&remote_url)
            .header(
                "authorization",
                header(&source_keys, HttpMethod::GET, &remote_url, b"")?,
            )
            .send()
            .await
            .context("peer fetch failed")?;
        if !remote.status().is_success() {
            eprintln!("skip {hash}: peer returned {}", remote.status());
            continue;
        }
        let bytes = remote.bytes().await?.to_vec();
        let actual = Sha256Hash::hash(&bytes).to_string();
        if actual != *hash {
            bail!("content verification failed: expected {hash}, got {actual}");
        }

        let upload_url = format!("{}/artifacts", config.to);
        let stored = client
            .post(&upload_url)
            .header(
                "authorization",
                header(&destination_keys, HttpMethod::POST, &upload_url, &bytes)?,
            )
            .header("content-type", "application/octet-stream")
            .body(bytes)
            .send()
            .await
            .context("local store failed")?;
        if !stored.status().is_success() {
            bail!("local store rejected {hash}: {}", stored.status());
        }
        println!("fetched {hash}");
        fetched += 1;
    }

    println!(
        "artifact sync complete: {} referenced, {present} already present, {fetched} fetched",
        referenced.len()
    );
    Ok(())
}

fn read_keys(path: &PathBuf, role: &str) -> anyhow::Result<Keys> {
    let secret = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {role} key file {}", path.display()))?;
    Ok(Keys::new(
        SecretKey::from_hex(secret.trim()).with_context(|| format!("invalid {role} secret key"))?,
    ))
}

#[cfg(test)]
mod config_tests {
    use super::*;

    fn args<'a>(values: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        values.iter().map(|value| (*value).to_string())
    }

    #[test]
    fn split_credentials_are_kept_distinct() {
        let config = Config::from_iter(args(&[
            "--data",
            "journal",
            "--from",
            "https://source.example",
            "--to",
            "http://destination.example",
            "--source-key",
            "reader.key",
            "--destination-key",
            "owner.key",
        ]))
        .expect("config parses");
        assert_eq!(config.source_key_file, PathBuf::from("reader.key"));
        assert_eq!(config.destination_key_file, PathBuf::from("owner.key"));
    }

    #[test]
    fn legacy_key_is_an_explicit_collapsed_role() {
        let config = Config::from_iter(args(&[
            "--data",
            "journal",
            "--from",
            "https://source.example",
            "--to",
            "http://destination.example",
            "--key",
            "node.key",
        ]))
        .expect("config parses");
        assert_eq!(config.source_key_file, PathBuf::from("node.key"));
        assert_eq!(config.destination_key_file, PathBuf::from("node.key"));
    }

    #[test]
    fn partial_or_mixed_credentials_are_rejected() {
        for credentials in [
            vec!["--source-key", "reader.key"],
            vec!["--destination-key", "owner.key"],
            vec![
                "--key",
                "node.key",
                "--source-key",
                "reader.key",
                "--destination-key",
                "owner.key",
            ],
        ] {
            let mut base = vec![
                "--data",
                "journal",
                "--from",
                "https://source.example",
                "--to",
                "http://destination.example",
            ];
            base.extend(credentials);
            assert!(Config::from_iter(args(&base)).is_err());
        }
    }
}
