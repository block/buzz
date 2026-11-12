//! Signed local-relay seed/read-back for the user-deletion development harness.
use anyhow::{bail, ensure, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buzz_test_client::loadgen::identity::nip98_header;
use clap::{Parser, ValueEnum};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Write, path::PathBuf, time::Duration};
use url::Url;

#[derive(Clone, Copy, ValueEnum)]
enum Command {
    Seed,
    Verify,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    Before,
    Preview,
    AfterMedia,
    AfterContent,
}

#[derive(Parser)]
struct Options {
    #[arg(value_enum)]
    command: Command,
    #[arg(long)]
    relay_url: String,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long, value_enum, default_value = "before")]
    phase: Phase,
}

#[derive(Serialize, Deserialize)]
struct RecordedEvent {
    event: Event,
    removed_by_content: bool,
}

#[derive(Serialize, Deserialize)]
struct Upload {
    sha256: String,
    path: String,
    removed_by_media: bool,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    complete: bool,
    relay_url: String,
    channel: String,
    target: String,
    survivor: String,
    events: Vec<RecordedEvent>,
    uploads: Vec<Upload>,
}

struct Relay {
    client: Client,
    base: Url,
}

impl Relay {
    async fn post(&self, keys: &Keys, path: &str, value: &Value) -> Result<Value> {
        let url = self.base.join(path)?.to_string();
        let body = serde_json::to_vec(value)?;
        let response = self
            .client
            .post(&url)
            .header("Authorization", nip98_header(keys, "POST", &url, &body))
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        ensure!(status.is_success(), "{path}: HTTP {status}: {text}");
        Ok(serde_json::from_str(&text)?)
    }

    async fn publish(&self, keys: &Keys, event: &Event) -> Result<()> {
        let output = self
            .post(keys, "events", &serde_json::to_value(event)?)
            .await?;
        ensure!(output["accepted"] == true, "event rejected: {output}");
        Ok(())
    }

    fn blossom_auth(&self, keys: &Keys, hash: &str, action: &str) -> Result<String> {
        let authority = &self.base[url::Position::BeforeHost..url::Position::AfterPort];
        let expiration = (Timestamp::now().as_secs() + 55).to_string();
        let event = EventBuilder::new(Kind::Custom(24242), "Local deletion fixture")
            .tags([
                Tag::parse(["t", action])?,
                Tag::parse(["x", hash])?,
                Tag::parse(["expiration", &expiration])?,
                Tag::parse(["server", authority])?,
            ])
            .sign_with_keys(keys)?;
        Ok(format!("Nostr {}", URL_SAFE_NO_PAD.encode(event.as_json())))
    }

    async fn upload(&self, keys: &Keys, bytes: &[u8], removed: bool) -> Result<Upload> {
        let hash = hex::encode(Sha256::digest(bytes));
        let response = self
            .client
            .put(self.base.join("upload")?)
            .header("Authorization", self.blossom_auth(keys, &hash, "upload")?)
            .header("Content-Type", "image/png")
            .header("X-SHA-256", &hash)
            .body(bytes.to_vec())
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        ensure!(status.is_success(), "upload: HTTP {status}: {body}");
        let descriptor: Value = serde_json::from_str(&body)?;
        ensure!(
            descriptor["sha256"] == hash,
            "upload descriptor hash mismatch"
        );
        let url = Url::parse(descriptor["url"].as_str().context("upload URL missing")?)?;
        ensure!(
            url.path().contains(&hash),
            "upload descriptor path missing hash"
        );
        let upload = Upload {
            sha256: hash,
            path: url.path().into(),
            removed_by_media: removed,
        };
        self.download(keys, &upload).await?;
        Ok(upload)
    }

    async fn download(&self, keys: &Keys, upload: &Upload) -> Result<()> {
        // Always use the supplied loopback relay, even if its descriptor advertises
        // a different public base. This helper never follows redirects.
        let mut url = self.base.clone();
        url.set_path(&upload.path);
        let response = self
            .client
            .get(url)
            .header(
                "Authorization",
                self.blossom_auth(keys, &upload.sha256, "get")?,
            )
            .send()
            .await?;
        ensure!(
            response.status().is_success(),
            "download {}: HTTP {}",
            upload.sha256,
            response.status()
        );
        let bytes = response.bytes().await?;
        ensure!(
            hex::encode(Sha256::digest(bytes)) == upload.sha256,
            "downloaded bytes changed"
        );
        Ok(())
    }

    async fn verify_not_found(&self, keys: &Keys, upload: &Upload) -> Result<()> {
        let mut url = self.base.clone();
        url.set_path(&upload.path);
        let response = self
            .client
            .get(url)
            .header(
                "Authorization",
                self.blossom_auth(keys, &upload.sha256, "get")?,
            )
            .send()
            .await?;
        ensure!(
            response.status() == reqwest::StatusCode::NOT_FOUND,
            "removed media {} must return 404; got {} (denial/error is not absence)",
            upload.sha256,
            response.status()
        );
        Ok(())
    }
}

fn sign(keys: &Keys, kind: u16, content: &str, tags: Vec<Tag>) -> Result<Event> {
    Ok(EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags)
        .sign_with_keys(keys)?)
}

fn png() -> Result<Vec<u8>> {
    let entropy = uuid::Uuid::new_v4();
    let seed = entropy.as_bytes();
    let image = image::RgbImage::from_fn(16, 16, |x, y| {
        let offset = (x + y) as usize % seed.len();
        image::Rgb([
            seed[offset],
            seed[(offset + 1) % seed.len()],
            seed[(offset + 2) % seed.len()],
        ])
    });
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

async fn seed(
    relay: &Relay,
    target: &Keys,
    survivor: &Keys,
    manifest: &mut Manifest,
) -> Result<()> {
    let channel = &manifest.channel;
    let create = sign(
        survivor,
        9007,
        "",
        vec![
            Tag::parse(["h", channel])?,
            Tag::parse(["name", &format!("deletion-live-{channel}")])?,
            Tag::parse(["channel_type", "stream"])?,
            Tag::parse(["visibility", "open"])?,
        ],
    )?;
    relay.publish(survivor, &create).await?;
    let add = sign(
        survivor,
        9000,
        "",
        vec![
            Tag::parse(["h", channel])?,
            Tag::parse(["p", &target.public_key().to_hex()])?,
        ],
    )?;
    relay.publish(survivor, &add).await?;
    let exclusive = relay.upload(target, &png()?, true).await?;
    let shared_bytes = png()?;
    let shared = relay.upload(target, &shared_bytes, false).await?;
    let shared_again = relay.upload(survivor, &shared_bytes, false).await?;
    ensure!(
        shared.sha256 == shared_again.sha256,
        "reupload changed hash"
    );
    let unrelated = relay.upload(survivor, &png()?, false).await?;
    let mut messages = Vec::new();
    for (keys, label, upload, removed) in [
        (target, "target exclusive", &exclusive, true),
        (target, "target shared", &shared, true),
        (survivor, "survivor shared", &shared, false),
        (survivor, "survivor exclusive", &unrelated, false),
    ] {
        let content = format!("Live deletion fixture {label}: {}", upload.path);
        let event = sign(keys, 9, &content, vec![Tag::parse(["h", channel])?])?;
        relay.publish(keys, &event).await?;
        messages.push(event.clone());
        manifest.events.push(RecordedEvent {
            event,
            removed_by_content: removed,
        });
    }
    for (keys, message) in [(survivor, &messages[0]), (target, &messages[3])] {
        let event = sign(
            keys,
            7,
            "+",
            vec![
                Tag::parse(["h", channel])?,
                Tag::parse(["e", &message.id.to_hex()])?,
                Tag::parse(["p", &message.pubkey.to_hex()])?,
            ],
        )?;
        relay.publish(keys, &event).await?;
        manifest.events.push(RecordedEvent {
            event,
            removed_by_content: true,
        });
    }
    manifest.uploads = vec![exclusive, shared, unrelated];
    Ok(())
}

async fn verify(
    relay: &Relay,
    target: &Keys,
    survivor: &Keys,
    manifest: &Manifest,
    phase: Phase,
) -> Result<()> {
    let ids: Vec<_> = manifest
        .events
        .iter()
        .map(|item| item.event.id.to_hex())
        .collect();
    let response = relay
        .post(
            survivor,
            "query",
            &json!([{
                "ids": ids, "kinds": [7,9], "#h": [manifest.channel], "limit": 100
            }]),
        )
        .await?;
    let events: Vec<Event> = serde_json::from_value(response)?;
    for event in &events {
        event.verify()?;
    }
    let actual: BTreeSet<_> = events.iter().map(|event| event.id.to_hex()).collect();
    let expected: BTreeSet<_> = manifest
        .events
        .iter()
        .filter(|item| phase != Phase::AfterContent || !item.removed_by_content)
        .map(|item| item.event.id.to_hex())
        .collect();
    ensure!(
        actual == expected,
        "relay event read-back mismatch: expected {expected:?}, actual {actual:?}"
    );
    for upload in &manifest.uploads {
        if matches!(phase, Phase::AfterMedia | Phase::AfterContent) && upload.removed_by_media {
            relay.verify_not_found(target, upload).await?;
            continue; // HTTP absence; the harness separately checks exact S3 versions.
        }
        let keys = if upload.removed_by_media {
            target
        } else {
            survivor
        };
        relay.download(keys, upload).await?;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let options = Options::parse();
    let base = Url::parse(&options.relay_url)?;
    ensure!(
        matches!(base.scheme(), "http" | "https")
            && matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
            && base.username().is_empty()
            && base.password().is_none()
            && base.path() == "/"
            && base.query().is_none()
            && base.fragment().is_none(),
        "relay URL must be a loopback HTTP origin without credentials or overrides"
    );
    let target =
        Keys::parse(&std::env::var("TARGET_SECRET_KEY").context("TARGET_SECRET_KEY missing")?)?;
    let survivor =
        Keys::parse(&std::env::var("SURVIVOR_SECRET_KEY").context("SURVIVOR_SECRET_KEY missing")?)?;
    ensure!(
        target.public_key() != survivor.public_key(),
        "target and survivor must differ"
    );
    let relay = Relay {
        client: Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::none())
            .build()?,
        base,
    };
    let manifest = match options.command {
        Command::Seed => {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&options.manifest)
                .context("manifest must be a new file")?;
            let mut manifest = Manifest {
                complete: false,
                relay_url: relay.base.to_string(),
                channel: uuid::Uuid::new_v4().to_string(),
                target: target.public_key().to_hex(),
                survivor: survivor.public_key().to_hex(),
                events: Vec::new(),
                uploads: Vec::new(),
            };
            file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
            seed(&relay, &target, &survivor, &mut manifest).await?;
            verify(&relay, &target, &survivor, &manifest, Phase::Before).await?;
            manifest.complete = true;
            std::fs::write(&options.manifest, serde_json::to_vec_pretty(&manifest)?)?;
            manifest
        }
        Command::Verify => {
            let manifest: Manifest = serde_json::from_slice(&std::fs::read(&options.manifest)?)?;
            if !manifest.complete {
                bail!("live fixture seed is incomplete");
            }
            ensure!(
                manifest.relay_url == relay.base.as_str()
                    && manifest.target == target.public_key().to_hex()
                    && manifest.survivor == survivor.public_key().to_hex(),
                "manifest identity/relay mismatch"
            );
            verify(&relay, &target, &survivor, &manifest, options.phase).await?;
            manifest
        }
    };
    println!(
        "{}",
        json!({"verified":true,"channel":manifest.channel,
        "phase": match options.command { Command::Seed => Phase::Before, Command::Verify => options.phase },
        "target":manifest.target,"survivor":manifest.survivor,
        "signed_events":manifest.events.len(),"media_files":manifest.uploads.len()})
    );
    Ok(())
}
