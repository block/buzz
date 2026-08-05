//! Live OSS-only topology proof.
//!
//! The repository wrapper starts two stock relay processes with one formal
//! SQLx database, Redis, and MinIO. These ignored tests then drive real
//! WebSocket, HTTP, media, Git, and audio clients. Operator routes are not
//! registered by either relay; their explicit composition is covered by the
//! separate O5 operator tests.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
    time::Duration,
};

use anyhow::{bail, ensure, Context, Result};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buzz_auth::{
    AuthTransport, AuthorizationCapability, AuthorizationClock, AuthorizationClockError,
    AuthorizationTime, VerifiedEvidenceAdapter,
};
use buzz_core::CommunityId;
use buzz_relay::authorization_runtime::{
    finalization::AuthorizationMode,
    transport::{
        DomainTransportPolicy, ProtectedAuthorizationResolver, ProtectedOperationRequest,
        ProtectedResolution, ProtectedResolutionError, ProtectedTransportError,
        ProtectedTransportRuntime, VerifiedProviderEvidenceResolution,
        VerifiedProviderEvidenceResolutionError, VerifiedProviderEvidenceResolver,
    },
};
use buzz_ws_client::{build_auth_event, parse_relay_message, RelayMessage};
use futures_util::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, RelayUrl, Tag, Timestamp, ToBech32};
use reqwest::{header, Client};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::{header::HOST as WS_HOST, HeaderValue as WsHeaderValue},
        Message,
    },
    MaybeTlsStream, WebSocketStream,
};
use uuid::Uuid;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone)]
struct LiveConfig {
    database_url: String,
    redis_url: String,
    relay_identity: String,
    relay_a_ws: String,
    relay_b_ws: String,
    relay_a_http: String,
    relay_b_http: String,
    relay_a_metrics: String,
    relay_b_metrics: String,
    tenant_host: String,
    relay_a_log: PathBuf,
    relay_b_log: PathBuf,
    git_helper: PathBuf,
    restart_state: PathBuf,
}

impl LiveConfig {
    fn from_env() -> Result<Self> {
        let relay_identity = env_or("OSS_E2E_RELAY_IDENTITY", "ws://127.0.0.1:3301");
        Ok(Self {
            database_url: required_env("BUZZ_TEST_DATABASE_URL")?,
            redis_url: required_env("REDIS_URL")?,
            relay_a_ws: env_or("OSS_E2E_RELAY_A_WS", "ws://127.0.0.1:3301"),
            relay_b_ws: env_or("OSS_E2E_RELAY_B_WS", "ws://127.0.0.1:3302"),
            relay_a_http: env_or("OSS_E2E_RELAY_A_HTTP", "http://127.0.0.1:3301"),
            relay_b_http: env_or("OSS_E2E_RELAY_B_HTTP", "http://127.0.0.1:3302"),
            relay_a_metrics: env_or("OSS_E2E_RELAY_A_METRICS", "http://127.0.0.1:9301/metrics"),
            relay_b_metrics: env_or("OSS_E2E_RELAY_B_METRICS", "http://127.0.0.1:9302/metrics"),
            tenant_host: env_or("OSS_E2E_TENANT_HOST", "127.0.0.1:3301"),
            relay_a_log: PathBuf::from(required_env("OSS_E2E_RELAY_A_LOG")?),
            relay_b_log: PathBuf::from(required_env("OSS_E2E_RELAY_B_LOG")?),
            git_helper: PathBuf::from(required_env("GIT_CREDENTIAL_NOSTR_BIN")?),
            restart_state: PathBuf::from(required_env("OSS_E2E_RESTART_STATE")?),
            relay_identity,
        })
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} is required for the live OSS E2E gate"))
}

struct RelaySocket {
    inner: WsStream,
}

impl RelaySocket {
    async fn connect(
        bind_url: &str,
        tenant_host: &str,
        relay_identity: &str,
        keys: &Keys,
    ) -> Result<Self> {
        let mut request = bind_url
            .into_client_request()
            .context("construct relay WebSocket request")?;
        request.headers_mut().insert(
            WS_HOST,
            WsHeaderValue::from_str(tenant_host).context("construct tenant Host header")?,
        );
        let (inner, response) = connect_async(request)
            .await
            .with_context(|| format!("connect to live relay {bind_url}"))?;
        ensure!(
            response.status().as_u16() == 101,
            "live relay WebSocket upgrade returned {}",
            response.status()
        );
        let mut socket = Self { inner };
        let challenge = socket.wait_for_challenge().await?;
        let auth = build_auth_event(&challenge, relay_identity, keys, None)
            .context("build NIP-42 authentication event")?;
        let auth_id = auth.id.to_hex();
        socket.send_json(&json!(["AUTH", auth])).await?;
        let accepted = socket.wait_for_ok(&auth_id).await?;
        ensure!(
            accepted,
            "live relay rejected synthetic NIP-42 authentication"
        );
        Ok(socket)
    }

    async fn send_json(&mut self, value: &Value) -> Result<()> {
        self.inner
            .send(Message::Text(value.to_string().into()))
            .await
            .context("send live relay WebSocket JSON")
    }

    async fn send_event(&mut self, event: &Event) -> Result<()> {
        let event_id = event.id.to_hex();
        self.send_json(&json!(["EVENT", event])).await?;
        ensure!(
            self.wait_for_ok(&event_id).await?,
            "live relay rejected event {event_id}"
        );
        Ok(())
    }

    async fn subscribe_channel(
        &mut self,
        subscription_id: &str,
        channel_id: Uuid,
        kind: u16,
    ) -> Result<()> {
        self.send_json(&json!([
            "REQ",
            subscription_id,
            {"kinds": [kind], "#h": [channel_id.to_string()]}
        ]))
        .await?;
        self.wait_for_eose(subscription_id).await?;
        Ok(())
    }

    async fn wait_for_challenge(&mut self) -> Result<String> {
        loop {
            match self.next_message(Duration::from_secs(20)).await? {
                RelayMessage::Auth { challenge } => return Ok(challenge),
                RelayMessage::Notice { message } => bail!("relay notice before auth: {message}"),
                _ => {}
            }
        }
    }

    async fn wait_for_ok(&mut self, event_id: &str) -> Result<bool> {
        loop {
            if let RelayMessage::Ok(ok) = self.next_message(Duration::from_secs(30)).await? {
                if ok.event_id == event_id {
                    if !ok.accepted {
                        bail!("relay rejected {event_id}: {}", ok.message);
                    }
                    return Ok(true);
                }
            }
        }
    }

    async fn wait_for_eose(&mut self, subscription_id: &str) -> Result<()> {
        loop {
            match self.next_message(Duration::from_secs(20)).await? {
                RelayMessage::Eose {
                    subscription_id: observed,
                } if observed == subscription_id => return Ok(()),
                RelayMessage::Closed {
                    subscription_id: observed,
                    message,
                } if observed == subscription_id => {
                    bail!("subscription {subscription_id} closed: {message}")
                }
                _ => {}
            }
        }
    }

    async fn wait_for_event(&mut self, subscription_id: &str, event_id: &str) -> Result<()> {
        loop {
            match self.next_message(Duration::from_secs(30)).await? {
                RelayMessage::Event {
                    subscription_id: observed,
                    event,
                } if observed == subscription_id && event.id.to_hex() == event_id => return Ok(()),
                RelayMessage::Closed {
                    subscription_id: observed,
                    message,
                } if observed == subscription_id => {
                    bail!("subscription {subscription_id} closed: {message}")
                }
                _ => {}
            }
        }
    }

    async fn next_message(&mut self, wait: Duration) -> Result<RelayMessage> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .context("timed out waiting for live relay message")?;
            let message = tokio::time::timeout(remaining, self.inner.next())
                .await
                .context("timed out waiting for live relay message")?
                .context("live relay closed WebSocket")?
                .context("read live relay WebSocket message")?;
            match message {
                Message::Text(text) => {
                    return parse_relay_message(&text).context("parse live relay message")
                }
                Message::Ping(bytes) => self
                    .inner
                    .send(Message::Pong(bytes))
                    .await
                    .context("send live relay pong")?,
                Message::Close(frame) => bail!("live relay closed WebSocket: {frame:?}"),
                _ => {}
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct RestartState {
    channel_id: Uuid,
    event_id: String,
}

struct LiveScenario {
    owner: Keys,
    channel_id: Uuid,
}

struct UnreachableAuthorizationResolver;

#[async_trait]
impl ProtectedAuthorizationResolver for UnreachableAuthorizationResolver {
    async fn resolve(
        &self,
        _request: &ProtectedOperationRequest,
    ) -> std::result::Result<ProtectedResolution, ProtectedResolutionError> {
        panic!("ambiguous provider evidence must deny before authority resolution")
    }
}

struct AmbiguousProviderEvidenceResolver;

impl VerifiedProviderEvidenceResolver for AmbiguousProviderEvidenceResolver {
    fn resolve(
        &self,
        _request: &ProtectedOperationRequest,
    ) -> std::result::Result<
        VerifiedProviderEvidenceResolution,
        VerifiedProviderEvidenceResolutionError,
    > {
        Ok(VerifiedProviderEvidenceResolution::Ambiguous)
    }
}

struct AbsentProviderEvidenceResolver;

impl VerifiedProviderEvidenceResolver for AbsentProviderEvidenceResolver {
    fn resolve(
        &self,
        _request: &ProtectedOperationRequest,
    ) -> std::result::Result<
        VerifiedProviderEvidenceResolution,
        VerifiedProviderEvidenceResolutionError,
    > {
        Ok(VerifiedProviderEvidenceResolution::Absent)
    }
}

struct FixedAuthorizationClock;

impl AuthorizationClock for FixedAuthorizationClock {
    fn now(&self) -> std::result::Result<AuthorizationTime, AuthorizationClockError> {
        Ok(AuthorizationTime::from_unix_seconds(1))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the repository-managed two-relay OSS topology"]
async fn live_two_relay_clients_and_migrations() {
    let config = LiveConfig::from_env().expect("load live OSS E2E configuration");
    verify_exact_migration_chain(&config)
        .await
        .expect("M01 exact SQLx migration chain");
    absent_provider_evidence_denies_runtime(&config)
        .await
        .expect("D01 absent typed provider evidence denies before authority resolution");
    ambiguous_provider_evidence_denies_runtime(&config)
        .await
        .expect("D02 ambiguous typed provider evidence denies in the relay runtime");
    let scenario = websocket_http_fanout(&config)
        .await
        .expect("A01 real WebSocket/HTTP cross-relay fan-out");
    media_roundtrip(&config, &scenario.owner)
        .await
        .expect("M02 real media client through shared object storage");
    git_roundtrip(&config, &scenario)
        .await
        .expect("G01 real Git client through relay transport");
    audio_roundtrip(&config, &scenario)
        .await
        .expect("AU01 real audio clients exchange a v2 frame");
    runtime_canaries_are_absent(&config)
        .await
        .expect("P01 runtime logs, errors, and metrics redact planted canaries");
}

fn verified_direct_request(
    config: &LiveConfig,
    challenge: &'static str,
) -> Result<ProtectedOperationRequest> {
    let authorization_domain = CommunityId::from_uuid(Uuid::new_v4());
    let keys = Keys::generate();
    let relay_url = RelayUrl::parse(&config.relay_identity).context("parse relay identity")?;
    let auth_event = EventBuilder::auth(challenge, relay_url)
        .sign_with_keys(&keys)
        .context("sign synthetic direct-origin NIP-42 proof")?;
    let proof = VerifiedEvidenceAdapter::new()
        .verify_nip42(
            authorization_domain,
            AuthTransport::RelayWebSocket,
            &auth_event,
            challenge,
            &config.relay_identity,
            None,
        )
        .context("verify synthetic direct-origin NIP-42 proof")?;
    ProtectedOperationRequest::new(
        Arc::new(proof),
        None,
        AuthorizationCapability::CommunityRead,
        Uuid::new_v4(),
        "ws_req",
    )
    .context("construct typed direct-origin protected request")
}

async fn absent_provider_evidence_denies_runtime(config: &LiveConfig) -> Result<()> {
    let request = verified_direct_request(config, "oss-e2e-d01-absent-provider-evidence")?;
    let authorization_domain = request.authorization_domain();
    let runtime = ProtectedTransportRuntime::new(
        [DomainTransportPolicy::from_server_configuration(
            authorization_domain,
            AuthorizationMode::Enforce,
        )],
        Arc::new(UnreachableAuthorizationResolver),
        Arc::new(FixedAuthorizationClock),
    )
    .context("construct D01 relay authorization runtime")?
    .with_provider_evidence_resolver(Arc::new(AbsentProviderEvidenceResolver));

    match runtime.authorize(&request).await {
        Err(ProtectedTransportError::Resolution(error)) => ensure!(
            error.code() == "provider_evidence_missing",
            "D01 absent evidence returned the wrong denial class"
        ),
        Err(error) => bail!("D01 absent evidence returned the wrong denial: {error}"),
        Ok(_) => bail!("D01 absent evidence reached an authority grant"),
    }
    Ok(())
}

async fn ambiguous_provider_evidence_denies_runtime(config: &LiveConfig) -> Result<()> {
    let request = verified_direct_request(config, "oss-e2e-d02-ambiguous-provider-evidence")?;
    let authorization_domain = request.authorization_domain();
    let runtime = ProtectedTransportRuntime::new(
        [DomainTransportPolicy::from_server_configuration(
            authorization_domain,
            AuthorizationMode::Enforce,
        )],
        Arc::new(UnreachableAuthorizationResolver),
        Arc::new(FixedAuthorizationClock),
    )
    .context("construct D02 relay authorization runtime")?
    .with_provider_evidence_resolver(Arc::new(AmbiguousProviderEvidenceResolver));

    ensure!(
        matches!(
            runtime.authorize(&request).await,
            Err(ProtectedTransportError::AmbiguousProviderEvidence)
        ),
        "D02 ambiguous typed provider evidence did not fail closed"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires relay B to have been restarted by the repository wrapper"]
async fn restarted_relay_restores_persisted_event() {
    let config = LiveConfig::from_env().expect("load live OSS E2E configuration");
    let state: RestartState = serde_json::from_slice(
        &std::fs::read(&config.restart_state).expect("read pre-restart state"),
    )
    .expect("parse pre-restart state");
    let keys = Keys::generate();
    let mut relay_b = RelaySocket::connect(
        &config.relay_b_ws,
        &config.tenant_host,
        &config.relay_identity,
        &keys,
    )
    .await
    .expect("connect to restarted relay B");
    let subscription_id = "oss-restart-persistence";
    relay_b
        .send_json(&json!([
            "REQ",
            subscription_id,
            {"kinds": [buzz_core::kind::KIND_STREAM_MESSAGE], "#h": [state.channel_id.to_string()]}
        ]))
        .await
        .expect("query restarted relay");
    relay_b
        .wait_for_event(subscription_id, &state.event_id)
        .await
        .expect("R01 restarted relay returns the persisted event");
}

async fn verify_exact_migration_chain(config: &LiveConfig) -> Result<()> {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&config.database_url)
        .await
        .context("connect to live OSS PostgreSQL")?;
    let embedded = MIGRATOR.iter().collect::<Vec<_>>();
    ensure!(
        embedded.len() == 50,
        "embedded migrator must contain 50 entries"
    );
    ensure!(
        embedded
            .iter()
            .enumerate()
            .all(|(index, migration)| migration.version == (index + 1) as i64),
        "embedded migrations must be gap-free from 0001 through 0050"
    );
    let applied = sqlx::query_as::<_, (i64, Vec<u8>, bool)>(
        "SELECT version, checksum, success FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(&pool)
    .await
    .context("read applied SQLx migrations")?;
    ensure!(
        applied.len() == embedded.len(),
        "live database applied {} migrations, expected {}",
        applied.len(),
        embedded.len()
    );
    for ((version, checksum, success), expected) in applied.iter().zip(embedded) {
        ensure!(*success, "migration {version:04} is not successful");
        ensure!(
            *version == expected.version,
            "migration version mismatch: {version} != {}",
            expected.version
        );
        ensure!(
            checksum.as_slice() == expected.checksum.as_ref(),
            "migration {version:04} checksum differs from the embedded SQLx chain"
        );
    }
    pool.close().await;
    Ok(())
}

async fn websocket_http_fanout(config: &LiveConfig) -> Result<LiveScenario> {
    let owner = Keys::generate();
    let channel_id = Uuid::new_v4();
    let channel = EventBuilder::new(Kind::Custom(9007), "")
        .tags([
            Tag::parse(["h", &channel_id.to_string()]).context("channel h tag")?,
            Tag::parse(["name", "OSS E2E fan-out"]).context("channel name tag")?,
            Tag::parse(["channel_type", "stream"]).context("channel type tag")?,
            Tag::parse(["visibility", "open"]).context("channel visibility tag")?,
        ])
        .sign_with_keys(&owner)
        .context("sign synthetic channel event")?;

    let mut relay_a = RelaySocket::connect(
        &config.relay_a_ws,
        &config.tenant_host,
        &config.relay_identity,
        &owner,
    )
    .await?;
    relay_a.send_event(&channel).await?;

    let mut relay_b = RelaySocket::connect(
        &config.relay_b_ws,
        &config.tenant_host,
        &config.relay_identity,
        &owner,
    )
    .await?;
    let subscription_id = "oss-live-fanout";
    let stream_kind = buzz_core::kind::KIND_STREAM_MESSAGE as u16;
    relay_b
        .subscribe_channel(subscription_id, channel_id, stream_kind)
        .await?;
    let redis_channel = wait_for_redis_subscription(&config.redis_url, channel_id).await?;
    let redis_client = redis::Client::open(config.redis_url.as_str())
        .context("open live OSS Redis diagnostic client")?;
    let mut redis_probe = redis_client
        .get_async_pubsub()
        .await
        .context("connect live OSS Redis diagnostic subscriber")?;
    redis_probe
        .subscribe(&redis_channel)
        .await
        .context("subscribe live OSS Redis diagnostic subscriber")?;
    let event = EventBuilder::new(Kind::Custom(stream_kind), "synthetic cross-relay fan-out")
        .tags([Tag::parse(["h", &channel_id.to_string()]).context("message h tag")?])
        .sign_with_keys(&owner)
        .context("sign synthetic fan-out event")?;
    let event_id = event.id.to_hex();
    relay_a.send_event(&event).await?;
    let mut redis_messages = redis_probe.on_message();
    let redis_message = tokio::time::timeout(Duration::from_secs(5), redis_messages.next())
        .await
        .context("Redis did not publish the synthetic cross-relay event")?
        .context("Redis diagnostic subscription ended before publication")?;
    let redis_payload: String = redis_message
        .get_payload()
        .context("decode synthetic Redis publication")?;
    let redis_event =
        Event::from_json(&redis_payload).context("parse synthetic event from Redis publication")?;
    ensure!(
        redis_event.id.to_hex() == event_id,
        "Redis diagnostic subscriber observed the wrong event"
    );
    if let Err(error) = relay_b.wait_for_event(subscription_id, &event_id).await {
        let metrics = diagnostic_metric_lines(&config.relay_b_metrics).await;
        bail!(
            "relay B missed event {event_id} after Redis published it on {redis_channel}: \
             {error:#}; relay-B metrics: {metrics}"
        );
    }

    let restart_state = RestartState {
        channel_id,
        event_id: event_id.clone(),
    };
    std::fs::write(
        &config.restart_state,
        serde_json::to_vec_pretty(&restart_state).context("encode restart state")?,
    )
    .context("write bounded synthetic restart state")?;
    Ok(LiveScenario { owner, channel_id })
}

async fn wait_for_redis_subscription(redis_url: &str, channel_id: Uuid) -> Result<String> {
    let client = redis::Client::open(redis_url).context("open live OSS Redis client")?;
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .context("connect live OSS Redis client")?;
    let pattern = format!("buzz:*:channel:{channel_id}");
    let expected_suffix = format!(":channel:{channel_id}");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    loop {
        let channels: Vec<String> = redis::cmd("PUBSUB")
            .arg("CHANNELS")
            .arg(&pattern)
            .query_async(&mut connection)
            .await
            .context("query live Redis subscriptions")?;
        if let Some(channel) = channels
            .into_iter()
            .find(|channel| channel.ends_with(&expected_suffix))
        {
            let subscribers: Vec<(String, u64)> = redis::cmd("PUBSUB")
                .arg("NUMSUB")
                .arg(&channel)
                .query_async(&mut connection)
                .await
                .context("query live Redis subscriber count")?;
            ensure!(
                subscribers
                    .iter()
                    .any(|(observed, count)| observed == &channel && *count >= 1),
                "Redis reported the scoped channel without a live subscriber"
            );
            return Ok(channel);
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("relay B did not acknowledge scoped Redis subscription {pattern}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn diagnostic_metric_lines(metrics_url: &str) -> String {
    const NAMES: [&str; 3] = [
        "buzz_multinode_fanout_total",
        "buzz_multinode_fanout_lag_total",
        "buzz_subscriptions_active",
    ];
    match Client::new().get(metrics_url).send().await {
        Ok(response) => match response.text().await {
            Ok(body) => {
                let selected: Vec<_> = body
                    .lines()
                    .filter(|line| !line.starts_with('#'))
                    .filter(|line| NAMES.iter().any(|name| line.starts_with(name)))
                    .collect();
                if selected.is_empty() {
                    "requested metrics absent".to_owned()
                } else {
                    selected.join(" | ")
                }
            }
            Err(error) => format!("metrics body unavailable: {error}"),
        },
        Err(error) => format!("metrics endpoint unavailable: {error}"),
    }
}

async fn post_event(config: &LiveConfig, event: &Event) -> Result<()> {
    let response = Client::new()
        .post(format!("{}/events", config.relay_a_http))
        .header(header::HOST, &config.tenant_host)
        .header("x-pubkey", event.pubkey.to_hex())
        .header(header::CONTENT_TYPE, "application/json")
        .json(event)
        .send()
        .await
        .context("POST event through the live HTTP bridge")?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    ensure!(
        status.is_success(),
        "HTTP event bridge returned {status}: {body}"
    );
    Ok(())
}

async fn media_roundtrip(config: &LiveConfig, keys: &Keys) -> Result<()> {
    let bytes = STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
        .context("decode synthetic one-pixel PNG")?;
    let digest = hex::encode(Sha256::digest(&bytes));
    let expiration = (Timestamp::now().as_secs() + 300).to_string();
    let auth = EventBuilder::new(Kind::Custom(24242), "synthetic OSS upload")
        .tags([
            Tag::parse(["t", "upload"]).context("media action tag")?,
            Tag::parse(["x", &digest]).context("media digest tag")?,
            Tag::parse(["expiration", &expiration]).context("media expiration tag")?,
        ])
        .sign_with_keys(keys)
        .context("sign synthetic media authorization")?;
    let authorization = format!(
        "Nostr {}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(auth.as_json().as_bytes())
    );
    let client = Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .context("build media client")?;
    let upload = client
        .put(format!("{}/upload", config.relay_a_http))
        .header(header::HOST, &config.tenant_host)
        .header(header::AUTHORIZATION, authorization)
        .header(header::CONTENT_TYPE, "image/png")
        .header("x-sha-256", &digest)
        .body(bytes.clone())
        .send()
        .await
        .context("upload synthetic media through relay A")?;
    let upload_status = upload.status();
    let upload_body = upload.text().await.unwrap_or_default();
    ensure!(
        upload_status.is_success(),
        "media upload returned {upload_status}: {upload_body}"
    );
    let descriptor: Value = serde_json::from_str(&upload_body).context("parse media descriptor")?;
    ensure!(
        descriptor["sha256"] == digest,
        "media descriptor digest mismatch"
    );

    let download = client
        .get(format!("{}/media/{digest}.png", config.relay_b_http))
        .header(header::HOST, &config.tenant_host)
        .send()
        .await
        .context("download shared media through relay B")?;
    let download_status = download.status();
    let downloaded = download.bytes().await.context("read downloaded media")?;
    ensure!(
        download_status.is_success(),
        "relay B media download returned {download_status}"
    );
    ensure!(
        downloaded.as_ref() == bytes,
        "downloaded media bytes differ"
    );
    Ok(())
}

async fn git_roundtrip(config: &LiveConfig, scenario: &LiveScenario) -> Result<()> {
    ensure!(
        config.git_helper.is_file(),
        "git credential helper is missing at {}",
        config.git_helper.display()
    );
    let repository = format!("oss-e2e-{}", Uuid::new_v4().simple());
    let announcement = EventBuilder::new(Kind::Custom(30617), "")
        .tags([
            Tag::parse(["d", &repository]).context("repository d tag")?,
            Tag::parse(["name", "OSS E2E repository"]).context("repository name tag")?,
            Tag::parse(["buzz-channel", &scenario.channel_id.to_string()])
                .context("repository channel tag")?,
        ])
        .sign_with_keys(&scenario.owner)
        .context("sign synthetic repository announcement")?;
    post_event(config, &announcement).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let temporary = tempfile::tempdir().context("create synthetic Git workspace")?;
    let owner_hex = scenario.owner.public_key().to_hex();
    let owner_nsec = scenario
        .owner
        .secret_key()
        .to_bech32()
        .context("encode synthetic Git key")?;
    let remote = format!("{}/git/{owner_hex}/{repository}", config.relay_a_http);
    run_git(
        config,
        temporary.path(),
        &owner_nsec,
        &["clone", "--quiet", &remote, "writer"],
    )?;
    let writer = temporary.path().join("writer");
    std::fs::write(writer.join("README.md"), "synthetic OSS E2E\n")
        .context("write synthetic Git fixture")?;
    run_git(config, &writer, &owner_nsec, &["add", "README.md"])?;
    run_git(
        config,
        &writer,
        &owner_nsec,
        &["commit", "--quiet", "-m", "synthetic fixture"],
    )?;
    run_git(config, &writer, &owner_nsec, &["branch", "-M", "main"])?;
    run_git(
        config,
        &writer,
        &owner_nsec,
        &["push", "--quiet", "origin", "main"],
    )?;
    run_git(
        config,
        temporary.path(),
        &owner_nsec,
        &["clone", "--quiet", &remote, "reader"],
    )?;
    let observed = std::fs::read_to_string(temporary.path().join("reader/README.md"))
        .context("read cloned synthetic Git fixture")?;
    ensure!(
        observed == "synthetic OSS E2E\n",
        "Git clone content mismatch"
    );
    Ok(())
}

fn run_git(config: &LiveConfig, cwd: &Path, nsec: &str, args: &[&str]) -> Result<Output> {
    let output = Command::new("git")
        .args([
            "-c",
            "credential.useHttpPath=true",
            "-c",
            &format!("credential.helper={}", config.git_helper.display()),
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
            "-c",
            "user.name=OSS E2E",
            "-c",
            "user.email=oss-e2e@example.invalid",
        ])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_CONFIG_COUNT")
        .env("NOSTR_PRIVATE_KEY", nsec)
        .output()
        .with_context(|| format!("run synthetic git {args:?}"))?;
    ensure!(
        output.status.success(),
        "git {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

struct AudioSocket {
    inner: WsStream,
}

impl AudioSocket {
    async fn connect(config: &LiveConfig, channel_id: Uuid, keys: &Keys) -> Result<Self> {
        let url = format!("{}/huddle/{channel_id}/audio", config.relay_a_ws);
        let mut request = url
            .into_client_request()
            .context("construct audio request")?;
        request.headers_mut().insert(
            WS_HOST,
            WsHeaderValue::from_str(&config.tenant_host).context("construct audio Host header")?,
        );
        let (inner, _) = connect_async(request)
            .await
            .context("connect real audio client")?;
        let mut socket = Self { inner };
        let challenge = loop {
            let message = socket.next(Duration::from_secs(10)).await?;
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(&text).context("parse audio challenge")?;
                if value["type"] == "challenge" {
                    break value["challenge"]
                        .as_str()
                        .context("audio challenge string")?
                        .to_owned();
                }
            }
        };
        let auth = build_auth_event(&challenge, &config.relay_identity, keys, None)
            .context("build audio NIP-42 event")?;
        socket
            .inner
            .send(Message::Text(
                json!({
                    "type": "auth",
                    "event": auth,
                    "parent_channel_id": null,
                    "protocol_version": 2
                })
                .to_string()
                .into(),
            ))
            .await
            .context("send audio authentication")?;
        loop {
            if let Message::Text(text) = socket.next(Duration::from_secs(10)).await? {
                let value: Value = serde_json::from_str(&text).context("parse audio join")?;
                match value["type"].as_str() {
                    Some("joined") => return Ok(socket),
                    Some("error") => bail!("audio join rejected: {}", value["message"]),
                    _ => {}
                }
            }
        }
    }

    async fn next(&mut self, wait: Duration) -> Result<Message> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .context("timed out while servicing audio relay control frames")?;
            let message = tokio::time::timeout(remaining, self.inner.next())
                .await
                .context("timed out waiting for audio relay")?
                .context("audio relay closed connection")?
                .context("read audio relay frame")?;
            match message {
                Message::Ping(bytes) => self
                    .inner
                    .send(Message::Pong(bytes))
                    .await
                    .context("send audio pong")?,
                other => return Ok(other),
            }
        }
    }
}

async fn audio_roundtrip(config: &LiveConfig, scenario: &LiveScenario) -> Result<()> {
    let peer = Keys::generate();
    let mut sender = AudioSocket::connect(config, scenario.channel_id, &scenario.owner).await?;
    let mut receiver = AudioSocket::connect(config, scenario.channel_id, &peer).await?;
    let frame = vec![0x00, 0x01, 0x00, 0x00, 0x03, 0xC0, 0xF0, 0x00, 0xF8, 0xFF];
    sender
        .inner
        .send(Message::Binary(frame.clone().into()))
        .await
        .context("send synthetic v2 audio frame")?;
    loop {
        if let Message::Binary(observed) = receiver.next(Duration::from_secs(10)).await? {
            ensure!(
                observed.len() == frame.len() + 1,
                "audio relay must prepend exactly one peer-index byte"
            );
            ensure!(
                &observed[1..] == frame.as_slice(),
                "audio relay altered the synthetic frame"
            );
            break;
        }
    }
    sender
        .inner
        .close(None)
        .await
        .context("close audio sender")?;
    receiver
        .inner
        .close(None)
        .await
        .context("close audio receiver")?;
    Ok(())
}

async fn runtime_canaries_are_absent(config: &LiveConfig) -> Result<()> {
    let canaries = [
        "oss-e2e-bearer-canary-7f36",
        "oss-e2e-jwt-canary-58aa",
        "oss-e2e-private-claim-canary-91cd",
    ];
    let response = Client::new()
        .put(format!("{}/upload", config.relay_a_http))
        .header(header::HOST, &config.tenant_host)
        .header(header::AUTHORIZATION, format!("Bearer {}", canaries[0]))
        .header("x-forwarded-identity-token", canaries[1])
        .header("x-synthetic-private-claim", canaries[2])
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body("synthetic unauthorized body")
        .send()
        .await
        .context("plant runtime redaction canaries")?;
    let status = response.status();
    let error_body = response.text().await.unwrap_or_default();
    ensure!(
        status.is_client_error(),
        "invalid media authorization unexpectedly returned {status}"
    );

    let metrics_a = Client::new()
        .get(&config.relay_a_metrics)
        .send()
        .await
        .context("read relay A metrics")?
        .text()
        .await
        .context("read relay A metrics body")?;
    let metrics_b = Client::new()
        .get(&config.relay_b_metrics)
        .send()
        .await
        .context("read relay B metrics")?
        .text()
        .await
        .context("read relay B metrics body")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let logs_a = std::fs::read_to_string(&config.relay_a_log)
        .with_context(|| format!("read {}", config.relay_a_log.display()))?;
    let logs_b = std::fs::read_to_string(&config.relay_b_log)
        .with_context(|| format!("read {}", config.relay_b_log.display()))?;
    let surfaces = [error_body, metrics_a, metrics_b, logs_a, logs_b];
    for canary in canaries {
        ensure!(
            surfaces.iter().all(|surface| !surface.contains(canary)),
            "planted private canary crossed a runtime log, error, or metric boundary"
        );
    }
    Ok(())
}
