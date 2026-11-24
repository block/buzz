//! Relay acceptance probe behind the `buzz-acceptance` binary.
//!
//! One run checks the relay's core jobs once, against one relay, and reports
//! each job (a "check") with its result, the stage where it failed, and how
//! long each stage took. It is meant to run on a schedule or as a deployment
//! step, and to exit non-zero when any check fails.
//!
//! Every run creates its own private channel, writes only there, and deletes
//! it at the end. A check that needs an earlier check's result is reported as
//! skipped when that check failed, so each failure is counted once, at its
//! cause, and every run reports the same set of checks.

#[cfg(test)]
mod fake_relay;
pub mod metrics;

use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use buzz_ws_client::{HeaderMap, HeaderValue, NostrWsConnection, RelayMessage, WsClientError};
use nostr::{
    Alphabet, Event, EventBuilder, EventId, Filter, Keys, Kind, SingleLetterTag, Tag, Timestamp,
};
use serde::Serialize;
use serde_json::json;

/// Request header carrying a NIP-FI assertion (`docs/nips/NIP-FI.md`).
const FEDERATED_IDENTITY_HEADER: &str = "Nostr-Federated-Identity";

/// Kind of the probe's channel messages (NIP-29 chat message).
const KIND_MESSAGE: u16 = 9;
/// NIP-29 group create and delete kinds.
const KIND_CREATE_GROUP: u16 = 9007;
const KIND_DELETE_GROUP: u16 = 9008;
/// Seconds of inactivity after which the relay archives a probe channel. A
/// backstop for runs that end before `cleanup` can delete their channel.
const CHANNEL_TTL_SECS: &str = "3600";
/// Largest assertion file the probe reads.
const MAX_ASSERTION_BYTES: u64 = 16 * 1024;

/// Every check, in the order a run reports them.
pub const CHECKS: [&str; 8] = [
    "relay_info",
    "connect_auth",
    "channel_create",
    "publish",
    "fanout",
    "history",
    "access_control",
    "cleanup",
];

/// What one run needs.
pub struct Config {
    /// The relay's WebSocket URL (`wss://…` or `ws://…`).
    pub relay_url: String,
    /// A plaintext `host:port` to dial instead of the URL's host, such as the
    /// relay's in-cluster service. `relay_url` still names the relay: it
    /// supplies the `Host` header, the NIP-42 relay tag and the community.
    pub connect_addr: Option<String>,
    /// The identity the probe authenticates and signs as.
    pub keys: Keys,
    /// A file holding a NIP-FI assertion for `keys`. It is read again before
    /// every connection, so another process can keep it fresh.
    pub assertion_file: Option<PathBuf>,
    /// The longest any one stage may take.
    pub stage_timeout: Duration,
}

/// The outcome of one run.
#[derive(Debug, Serialize)]
pub struct Report {
    /// The relay the run checked.
    pub relay_url: String,
    /// The public key (hex) the run authenticated and signed as.
    pub pubkey: String,
    /// The relay's NIP-11 `version`, when it reported one.
    pub relay_version: Option<String>,
    /// True when no check failed.
    pub passed: bool,
    /// How long the whole run took.
    pub seconds: f64,
    /// One entry per name in [`CHECKS`], in that order.
    pub checks: Vec<CheckReport>,
}

/// The outcome of one check.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    /// The check's name, one of [`CHECKS`].
    pub check: &'static str,
    /// Whether it passed, failed or was skipped.
    pub result: Outcome,
    /// The stage that failed, for a failure.
    pub failed_stage: Option<&'static str>,
    /// Why it failed or was skipped.
    pub error: Option<String>,
    /// How long the check took, failed stage included.
    pub seconds: f64,
    /// Each stage the check completed, with its duration.
    pub stages: Vec<StageTiming>,
}

/// One completed stage and how long it took.
#[derive(Debug, Serialize)]
pub struct StageTiming {
    /// The stage's name.
    pub stage: &'static str,
    /// How long it took.
    pub seconds: f64,
}

/// The result of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// Every stage passed.
    Success,
    /// A stage failed or timed out.
    Failure,
    /// Not run, because a check it depends on failed.
    Skipped,
}

impl Outcome {
    /// The lowercase name used in reports and metric tags.
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Success => "success",
            Outcome::Failure => "failure",
            Outcome::Skipped => "skipped",
        }
    }
}

/// A stage that failed: its name and why.
struct StageError {
    stage: &'static str,
    error: String,
}

/// One check in progress: times its stages and bounds each by the timeout.
struct Check {
    name: &'static str,
    started: Instant,
    timeout: Duration,
    stages: Vec<StageTiming>,
}

impl Check {
    fn new(name: &'static str, timeout: Duration) -> Self {
        Self {
            name,
            started: Instant::now(),
            timeout,
            stages: Vec::new(),
        }
    }

    /// Runs `work` as `stage`, recording its duration when it succeeds.
    async fn stage<T>(
        &mut self,
        stage: &'static str,
        work: impl Future<Output = anyhow::Result<T>>,
    ) -> Result<T, StageError> {
        let started = Instant::now();
        let result = match tokio::time::timeout(self.timeout, work).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(format!("{e:#}")),
            Err(_) => Err(format!("timed out after {:?}", self.timeout)),
        };
        match result {
            Ok(value) => {
                self.stages.push(StageTiming {
                    stage,
                    seconds: started.elapsed().as_secs_f64(),
                });
                Ok(value)
            }
            Err(error) => Err(StageError { stage, error }),
        }
    }

    fn finish(self, result: Result<(), StageError>) -> CheckReport {
        let (outcome, failed_stage, error) = match result {
            Ok(()) => (Outcome::Success, None, None),
            Err(e) => (Outcome::Failure, Some(e.stage), Some(e.error)),
        };
        CheckReport {
            check: self.name,
            result: outcome,
            failed_stage,
            error,
            seconds: self.started.elapsed().as_secs_f64(),
            stages: self.stages,
        }
    }
}

fn skipped(check: &'static str, because: &str) -> CheckReport {
    CheckReport {
        check,
        result: Outcome::Skipped,
        failed_stage: None,
        error: Some(format!("skipped: {because} failed")),
        seconds: 0.0,
        stages: Vec::new(),
    }
}

/// Runs every check in [`CHECKS`] once against `cfg.relay_url`.
pub async fn run(cfg: &Config) -> Report {
    let started = Instant::now();
    let mut checks = Vec::with_capacity(CHECKS.len());

    let (report, relay_version) = relay_info(cfg).await;
    checks.push(report);

    let (report, conn) = connect_auth(cfg).await;
    checks.push(report);
    match conn {
        Some(mut conn) => {
            let created = run_channel_checks(cfg, &mut conn, &mut checks).await;
            // Closing is best effort: every check has already been recorded.
            let _ = conn.disconnect().await;
            checks.push(match created {
                Some(channel) => cleanup(cfg, &channel).await,
                None => skipped("cleanup", "channel_create"),
            });
        }
        None => {
            for &check in &CHECKS[2..] {
                checks.push(skipped(check, "connect_auth"));
            }
        }
    }

    Report {
        relay_url: cfg.relay_url.clone(),
        pubkey: cfg.keys.public_key().to_hex(),
        relay_version,
        passed: checks.iter().all(|c| c.result != Outcome::Failure),
        seconds: started.elapsed().as_secs_f64(),
        checks,
    }
}

/// The checks that need the authenticated connection `conn`, `cleanup`
/// excepted. Returns the channel `cleanup` must delete: one the relay
/// accepted, or one whose create may have committed without an answer.
async fn run_channel_checks(
    cfg: &Config,
    conn: &mut NostrWsConnection,
    checks: &mut Vec<CheckReport>,
) -> Option<String> {
    let (report, created) = channel_create(cfg, conn).await;
    let passed = report.result == Outcome::Success;
    checks.push(report);
    if !passed {
        for &check in &CHECKS[3..CHECKS.len() - 1] {
            checks.push(skipped(check, "channel_create"));
        }
        return created;
    }
    let channel = created?;

    let (report, published) = publish(cfg, conn, &channel).await;
    checks.push(report);
    checks.push(fanout(cfg, conn, &channel).await);
    checks.push(match published {
        Some(id) => history(cfg, conn, &channel, id).await,
        None => skipped("history", "publish"),
    });
    checks.push(access_control(cfg, conn, &channel).await);
    Some(channel)
}

async fn relay_info(cfg: &Config) -> (CheckReport, Option<String>) {
    let mut check = Check::new("relay_info", cfg.stage_timeout);
    let result = check
        .stage("nip11", async {
            let response = nip11_request(cfg)?
                .header("Accept", "application/nostr+json")
                .send()
                .await?;
            if !response.status().is_success() {
                bail!("NIP-11 request answered {}", response.status());
            }
            let info: serde_json::Value = response.json().await?;
            if !info["supported_nips"].is_array() {
                bail!("not a NIP-11 document: no supported_nips");
            }
            Ok(info["version"].as_str().map(str::to_owned))
        })
        .await;
    match result {
        Ok(version) => (check.finish(Ok(())), version),
        Err(e) => (check.finish(Err(e)), None),
    }
}

async fn connect_auth(cfg: &Config) -> (CheckReport, Option<NostrWsConnection>) {
    let mut check = Check::new("connect_auth", cfg.stage_timeout);
    let result = async {
        let mut conn = check.stage("ws_connect", open(cfg)).await?;
        check
            .stage("nip42_auth", async {
                Ok(conn.authenticate(&cfg.keys, None).await?)
            })
            .await?;
        Ok(conn)
    }
    .await;
    match result {
        Ok(conn) => (check.finish(Ok(())), Some(conn)),
        Err(e) => (check.finish(Err(e)), None),
    }
}

/// Creates the run's private channel, with a TTL as a backstop. Returns the
/// channel unless the relay refused it: after a timeout or a lost connection
/// the create may still have committed, so `cleanup` must try to delete it.
async fn channel_create(
    cfg: &Config,
    conn: &mut NostrWsConnection,
) -> (CheckReport, Option<String>) {
    let mut check = Check::new("channel_create", cfg.stage_timeout);
    let channel = uuid::Uuid::new_v4().to_string();
    let mut refused = false;
    let result = check
        .stage("create", async {
            let event = EventBuilder::new(Kind::Custom(KIND_CREATE_GROUP), "")
                .tags([
                    tag(["h", &channel])?,
                    tag(["name", &format!("acceptance-{channel}")])?,
                    tag(["channel_type", "stream"])?,
                    tag(["visibility", "private"])?,
                    tag(["ttl", CHANNEL_TTL_SECS])?,
                ])
                .sign_with_keys(&cfg.keys)?;
            let ok = conn.send_event(event).await?;
            if !ok.accepted {
                refused = true;
                bail!("relay rejected the event: {}", ok.message);
            }
            Ok(())
        })
        .await;
    let report = check.finish(result);
    (report, (!refused).then_some(channel))
}

async fn publish(
    cfg: &Config,
    conn: &mut NostrWsConnection,
    channel: &str,
) -> (CheckReport, Option<EventId>) {
    let mut check = Check::new("publish", cfg.stage_timeout);
    let result = check
        .stage("ok", async {
            let event = message(&cfg.keys, channel, "acceptance publish")?;
            let id = event.id;
            accepted(conn, event).await?;
            Ok(id)
        })
        .await;
    match result {
        Ok(id) => (check.finish(Ok(())), Some(id)),
        Err(e) => (check.finish(Err(e)), None),
    }
}

/// A second connection subscribes to the channel; the first publishes; the
/// second must receive it intact. `receive` is timed from the publish to the moment
/// the event arrived on the second socket.
async fn fanout(cfg: &Config, conn: &mut NostrWsConnection, channel: &str) -> CheckReport {
    let mut check = Check::new("fanout", cfg.stage_timeout);
    let result = async {
        let mut listener = check
            .stage("connect_second", async {
                let mut conn = open(cfg).await?;
                conn.authenticate(&cfg.keys, None).await?;
                Ok(conn)
            })
            .await?;
        let result = fanout_on(&mut check, cfg, conn, &mut listener, channel).await;
        let _ = listener.disconnect().await;
        result
    }
    .await;
    check.finish(result)
}

async fn fanout_on(
    check: &mut Check,
    cfg: &Config,
    conn: &mut NostrWsConnection,
    listener: &mut NostrWsConnection,
    channel: &str,
) -> Result<(), StageError> {
    let timeout = cfg.stage_timeout;
    check
        .stage("subscribe", async {
            let live = channel_filter(channel).since(Timestamp::now()).limit(0);
            listener.send_raw(&json!(["REQ", "fanout", live])).await?;
            wait_for_eose(listener, "fanout", timeout).await
        })
        .await?;
    let event = message(&cfg.keys, channel, "acceptance fanout").map_err(|e| StageError {
        stage: "publish",
        error: format!("{e:#}"),
    })?;
    let id = event.id;
    // Read the listener while the publish waits for its OK, so `receive`
    // measures delivery, not the publisher's acknowledgement.
    let sent_at = Instant::now();
    let arrived = async {
        loop {
            match listener.next_event_received(timeout).await? {
                (
                    at,
                    RelayMessage::Event {
                        subscription_id,
                        event,
                    },
                ) if subscription_id == "fanout" && event.id == id => {
                    intact(&event)?;
                    return Ok(at);
                }
                (
                    _,
                    RelayMessage::Closed {
                        subscription_id,
                        message,
                    },
                ) if subscription_id == "fanout" => {
                    bail!("subscription closed: {message}")
                }
                _ => {}
            }
        }
    };
    let (published, arrived) = tokio::join!(
        check.stage("publish", accepted(conn, event)),
        tokio::time::timeout(timeout, arrived)
    );
    published?;
    match arrived {
        Ok(Ok(at)) => {
            check.stages.push(StageTiming {
                stage: "receive",
                seconds: at.saturating_duration_since(sent_at).as_secs_f64(),
            });
            Ok(())
        }
        Ok(Err(e)) => Err(StageError {
            stage: "receive",
            error: format!("{e:#}"),
        }),
        Err(_) => Err(StageError {
            stage: "receive",
            error: format!("timed out after {timeout:?}"),
        }),
    }
}

/// A fresh REQ for the channel must return the event `publish` stored,
/// intact.
async fn history(
    cfg: &Config,
    conn: &mut NostrWsConnection,
    channel: &str,
    id: EventId,
) -> CheckReport {
    let mut check = Check::new("history", cfg.stage_timeout);
    let timeout = cfg.stage_timeout;
    let result = check
        .stage("req_eose", async {
            let filter = channel_filter(channel).id(id);
            conn.send_raw(&json!(["REQ", "history", filter])).await?;
            let mut found = false;
            loop {
                match conn.next_event(timeout).await? {
                    RelayMessage::Event {
                        subscription_id,
                        event,
                    } if subscription_id == "history" && event.id == id => {
                        intact(&event)?;
                        found = true;
                    }
                    RelayMessage::Eose { subscription_id } if subscription_id == "history" => break,
                    RelayMessage::Closed {
                        subscription_id,
                        message,
                    } if subscription_id == "history" => {
                        bail!("subscription closed: {message}")
                    }
                    _ => {}
                }
            }
            conn.send_raw(&json!(["CLOSE", "history"])).await?;
            if !found {
                bail!("stored event {id} missing from history");
            }
            Ok(())
        })
        .await;
    check.finish(result)
}

/// Denial reason (NIP-01 prefix) for a write on a connection without NIP-42 AUTH.
const AUTH_REQUIRED: &str = "auth-required:";
/// Denial reason for a write signed by a key other than the authenticated one.
const PUBKEY_MISMATCH: &str = "invalid: event pubkey does not match authenticated identity";

/// The relay must refuse, with the matching reason, a write on a connection
/// that skipped NIP-42 AUTH, and a write signed by a key other than the one
/// the connection authenticated as. The unauthenticated connection still
/// sends the NIP-FI assertion, so only AUTH is missing. Any other answer,
/// including a closed socket or a different denial, fails the stage.
async fn access_control(cfg: &Config, conn: &mut NostrWsConnection, channel: &str) -> CheckReport {
    let mut check = Check::new("access_control", cfg.stage_timeout);
    let result = async {
        check
            .stage("unauthenticated_rejected", async {
                let event = message(&cfg.keys, channel, "acceptance unauthenticated")?;
                let mut anon = open(cfg).await?;
                let outcome = anon.send_event(event).await;
                let _ = anon.disconnect().await;
                denied(outcome, AUTH_REQUIRED)
            })
            .await?;
        check
            .stage("pubkey_mismatch_rejected", async {
                let event = message(&Keys::generate(), channel, "acceptance mismatch")?;
                denied(conn.send_event(event).await, PUBKEY_MISMATCH)
            })
            .await
    }
    .await;
    check.finish(result)
}

/// Deletes `channel` over a fresh connection, so a socket an earlier check
/// lost cannot strand it. A failure names the channel.
async fn cleanup(cfg: &Config, channel: &str) -> CheckReport {
    let mut check = Check::new("cleanup", cfg.stage_timeout);
    let result = async {
        let mut conn = check
            .stage("connect", async {
                let mut conn = open(cfg).await?;
                conn.authenticate(&cfg.keys, None).await?;
                Ok(conn)
            })
            .await?;
        let result = check
            .stage("delete_channel", async {
                let event = EventBuilder::new(Kind::Custom(KIND_DELETE_GROUP), "")
                    .tags([tag(["h", channel])?])
                    .sign_with_keys(&cfg.keys)?;
                accepted(&mut conn, event).await
            })
            .await;
        let _ = conn.disconnect().await;
        result
    }
    .await
    .map_err(|e| StageError {
        stage: e.stage,
        error: format!("channel {channel}: {}", e.error),
    });
    check.finish(result)
}

/// Sends `event` and fails unless the relay accepts it.
async fn accepted(conn: &mut NostrWsConnection, event: Event) -> anyhow::Result<()> {
    let ok = conn.send_event(event).await?;
    if !ok.accepted {
        bail!("relay rejected the event: {}", ok.message);
    }
    Ok(())
}

/// Passes only when the relay answered `OK false` with a reason starting
/// with `expected`. No answer, a closed socket or another reason fails.
fn denied(
    outcome: Result<buzz_ws_client::OkResponse, WsClientError>,
    expected: &str,
) -> anyhow::Result<()> {
    let ok = outcome.context("no OK for the write that should be refused")?;
    if ok.accepted {
        bail!("relay accepted the event");
    }
    if !ok.message.starts_with(expected) {
        bail!("relay refused for another reason: {}", ok.message);
    }
    Ok(())
}

async fn wait_for_eose(
    conn: &mut NostrWsConnection,
    sub_id: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    loop {
        match conn.next_event(timeout).await? {
            RelayMessage::Eose { subscription_id } if subscription_id == sub_id => return Ok(()),
            RelayMessage::Closed {
                subscription_id,
                message,
            } if subscription_id == sub_id => {
                bail!("subscription closed: {message}")
            }
            _ => {}
        }
    }
}

/// The WebSocket upgrade headers: the NIP-FI assertion, when configured.
fn upgrade_headers(cfg: &Config) -> anyhow::Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    if let Some(path) = &cfg.assertion_file {
        let assertion = read_secret_file(path, "assertion file", MAX_ASSERTION_BYTES)?;
        let mut value = HeaderValue::from_str(&format!("Bearer {assertion}"))
            .map_err(|_| anyhow!("assertion file {} is not a header value", path.display()))?;
        value.set_sensitive(true);
        headers.insert(FEDERATED_IDENTITY_HEADER, value);
    }
    Ok(headers)
}

/// Reads a small secret another process writes, such as a key or an
/// assertion, and returns it without surrounding whitespace.
///
/// The file must be a regular, non-empty file of at most `max_bytes`.
/// A FIFO or directory fails at once rather than blocking the run.
pub fn read_secret_file(path: &Path, what: &str, max_bytes: u64) -> anyhow::Result<String> {
    let read = || -> std::io::Result<Option<String>> {
        // Check before opening: opening a FIFO blocks until a writer
        // appears, and no async timeout can interrupt that.
        if !std::fs::metadata(path)?.is_file() {
            return Ok(None);
        }
        let file = std::fs::File::open(path)?;
        if !file.metadata()?.is_file() {
            return Ok(None);
        }
        // Cap the read on the open file: it may grow after the check.
        let mut text = String::new();
        file.take(max_bytes + 1).read_to_string(&mut text)?;
        Ok((text.len() as u64 <= max_bytes).then_some(text))
    };
    let text = read()
        .with_context(|| format!("read {what} {}", path.display()))?
        .ok_or_else(|| {
            anyhow!(
                "{what} {} must be a regular file of at most {max_bytes} bytes",
                path.display()
            )
        })?;
    let text = text.trim();
    if text.is_empty() {
        bail!("{what} {} is empty", path.display());
    }
    Ok(text.to_owned())
}

/// Opens a connection to the relay without authenticating, at
/// `cfg.connect_addr` when one is set.
async fn open(cfg: &Config) -> anyhow::Result<NostrWsConnection> {
    let headers = upgrade_headers(cfg)?;
    Ok(match &cfg.connect_addr {
        Some(addr) => NostrWsConnection::connect_to_addr(&cfg.relay_url, addr, headers).await?,
        None => NostrWsConnection::connect_with_headers(&cfg.relay_url, headers).await?,
    })
}

/// The NIP-11 request for the relay. With `cfg.connect_addr` it goes to that
/// address over plain HTTP, with the relay URL's host as the `Host` header,
/// so the relay serves the same community's document. Like the WebSocket
/// connections, it then uses no proxy and follows no redirect: either could
/// reach another deployment and report its version.
fn nip11_request(cfg: &Config) -> anyhow::Result<reqwest::RequestBuilder> {
    let url = http_url(&cfg.relay_url)?;
    let Some(addr) = &cfg.connect_addr else {
        return Ok(reqwest::Client::new().get(url));
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let host = &url[url::Position::BeforeHost..url::Position::AfterPort];
    let target = &url[url::Position::BeforePath..url::Position::AfterQuery];
    Ok(client
        .get(format!("http://{addr}{target}"))
        .header(reqwest::header::HOST, host))
}

/// The relay's HTTP base for `ws_url`: `wss` becomes `https`, `ws` `http`.
fn http_url(ws_url: &str) -> anyhow::Result<url::Url> {
    let mut url = url::Url::parse(ws_url).with_context(|| format!("parse {ws_url}"))?;
    let scheme = match url.scheme() {
        "wss" => "https",
        "ws" => "http",
        other => bail!("relay URL scheme must be ws or wss, not {other}"),
    };
    url.set_scheme(scheme)
        .map_err(|()| anyhow!("cannot use {scheme} for {ws_url}"))?;
    Ok(url)
}

/// Fails unless `event`'s id and signature match its content. A match on
/// the id field alone would pass an event the relay corrupted.
fn intact(event: &Event) -> anyhow::Result<()> {
    event
        .verify()
        .with_context(|| format!("event {} failed verification", event.id))
}

fn channel_filter(channel: &str) -> Filter {
    Filter::new()
        .kind(Kind::Custom(KIND_MESSAGE))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::H), channel)
}

fn message(keys: &Keys, channel: &str, content: &str) -> anyhow::Result<Event> {
    Ok(EventBuilder::new(Kind::Custom(KIND_MESSAGE), content)
        .tags([tag(["h", channel])?])
        .sign_with_keys(keys)?)
}

fn tag<const N: usize>(parts: [&str; N]) -> anyhow::Result<Tag> {
    Ok(Tag::parse(parts)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(relay_url: String) -> Config {
        Config {
            relay_url,
            connect_addr: None,
            keys: Keys::generate(),
            assertion_file: None,
            stage_timeout: Duration::from_secs(2),
        }
    }

    /// With nothing listening, the run fails at the cause (`relay_info`
    /// and `connect_auth`), skips the checks that need a connection, and
    /// still reports every check in order.
    #[tokio::test]
    async fn unreachable_relay_fails_at_the_cause_and_skips_dependents() {
        let port = {
            let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            socket.local_addr().unwrap().port()
        };
        let report = run(&config(format!("ws://127.0.0.1:{port}"))).await;

        assert!(!report.passed);
        let names: Vec<&str> = report.checks.iter().map(|c| c.check).collect();
        assert_eq!(names, CHECKS);
        let results: Vec<(&str, Outcome, Option<&str>)> = report
            .checks
            .iter()
            .map(|c| (c.check, c.result, c.failed_stage))
            .collect();
        assert_eq!(results[0], ("relay_info", Outcome::Failure, Some("nip11")));
        assert_eq!(
            results[1],
            ("connect_auth", Outcome::Failure, Some("ws_connect"))
        );
        for (name, outcome, stage) in &results[2..] {
            assert_eq!((*outcome, *stage), (Outcome::Skipped, None), "{name}");
        }
    }

    #[tokio::test]
    async fn a_stage_that_overruns_the_timeout_fails_with_its_name() {
        let mut check = Check::new("t", Duration::from_millis(20));
        check.stage("fast", async { Ok(()) }).await.ok().unwrap();
        let err = check
            .stage("slow", async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            })
            .await
            .err()
            .unwrap();
        let report = check.finish(Err(err));
        assert_eq!(report.result, Outcome::Failure);
        assert_eq!(report.failed_stage, Some("slow"));
        assert!(report.error.unwrap().contains("timed out"));
        let stages: Vec<&str> = report.stages.iter().map(|s| s.stage).collect();
        assert_eq!(stages, ["fast"]);
    }

    #[test]
    fn upgrade_headers_read_the_assertion_file_each_time() {
        let dir = std::env::temp_dir().join(format!("acceptance-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("assertion");
        let mut cfg = config("wss://relay.example".into());
        assert!(upgrade_headers(&cfg).unwrap().is_empty());

        cfg.assertion_file = Some(path.clone());
        assert!(upgrade_headers(&cfg).is_err(), "missing file must fail");
        std::fs::write(&path, "  \n").unwrap();
        assert!(upgrade_headers(&cfg).is_err(), "empty file must fail");

        std::fs::write(&path, "first\n").unwrap();
        assert_eq!(
            upgrade_headers(&cfg).unwrap()[FEDERATED_IDENTITY_HEADER],
            "Bearer first"
        );
        std::fs::write(&path, "second").unwrap();
        assert_eq!(
            upgrade_headers(&cfg).unwrap()[FEDERATED_IDENTITY_HEADER],
            "Bearer second"
        );

        std::fs::write(&path, "x".repeat(MAX_ASSERTION_BYTES as usize + 1)).unwrap();
        assert!(upgrade_headers(&cfg).is_err(), "oversized file must fail");
        cfg.assertion_file = Some(dir.clone());
        assert!(upgrade_headers(&cfg).is_err(), "a directory must fail");

        // A FIFO with no writer must fail at once, not block the open.
        #[cfg(unix)]
        {
            let fifo = dir.join("fifo");
            let made = std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap();
            assert!(made.success());
            let mut cfg = config("wss://relay.example".into());
            cfg.assertion_file = Some(fifo);
            let (done, result) = std::sync::mpsc::channel();
            std::thread::spawn(move || done.send(upgrade_headers(&cfg).is_err()).unwrap());
            assert!(
                result.recv_timeout(Duration::from_secs(5)).unwrap(),
                "a FIFO must fail"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// With a connect address the probe dials it, but every upgrade and
    /// AUTH still names the relay URL, so the relay serves that community.
    #[tokio::test]
    async fn connect_addr_dials_the_address_and_keeps_the_relay_url() {
        let dir = std::env::temp_dir().join(format!("acceptance-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let assertion = dir.join("assertion");
        std::fs::write(&assertion, "header.payload.signature").unwrap();
        let relay = fake_relay::start(fake_relay::Script::default()).await;
        let mut cfg = config("wss://relay.invalid".into());
        cfg.connect_addr = Some(relay.url.trim_start_matches("ws://").to_owned());
        cfg.assertion_file = Some(assertion);
        let report = run(&cfg).await;
        std::fs::remove_dir_all(dir).unwrap();

        assert_all_pass_but_relay_info(&report);
        let upgrades = relay.upgrades();
        assert!(!upgrades.is_empty());
        for headers in &upgrades {
            assert_eq!(headers["host"], "relay.invalid", "{headers:?}");
            assert_eq!(
                headers["nostr-federated-identity"], "Bearer header.payload.signature",
                "{headers:?}"
            );
        }
        let relays = relay.auth_relays();
        assert!(!relays.is_empty());
        assert!(
            relays.iter().all(|r| r == "wss://relay.invalid"),
            "{relays:?}"
        );
    }

    /// Serves one HTTP request with `head` (status line and headers) and
    /// `body`. Returns the address and the request it received, lowercased.
    async fn one_http_response(
        head: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                tcp.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let response = format!(
                "{head}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            tcp.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap().to_ascii_lowercase()
        });
        (addr, server)
    }

    /// With a connect address the NIP-11 fetch goes to it over plain HTTP,
    /// with the relay URL's path and query, and its host and port as the
    /// `Host` header.
    #[tokio::test]
    async fn nip11_goes_to_the_connect_addr_with_the_relay_host() {
        let (addr, server) = one_http_response(
            "HTTP/1.1 200 OK\r\ncontent-type: application/nostr+json",
            r#"{"supported_nips":[1,11],"version":"1.2.3"}"#,
        )
        .await;
        let mut cfg = config("wss://relay.invalid:8443/relay?route=probe".into());
        cfg.connect_addr = Some(addr);
        let (report, version) = relay_info(&cfg).await;

        assert_eq!(report.result, Outcome::Success, "{report:?}");
        assert_eq!(version.as_deref(), Some("1.2.3"));
        let request = server.await.unwrap();
        assert!(
            request.starts_with("get /relay?route=probe http/1.1\r\n"),
            "{request}"
        );
        assert!(
            request.contains("\r\nhost: relay.invalid:8443\r\n"),
            "{request}"
        );
    }

    /// With a connect address the NIP-11 fetch follows no redirect: one to
    /// another host fails the check and reports no version.
    #[tokio::test]
    async fn nip11_at_the_connect_addr_does_not_follow_a_redirect() {
        let (addr, server) = one_http_response(
            "HTTP/1.1 301 Moved Permanently\r\nlocation: http://127.0.0.1:9/",
            "",
        )
        .await;
        let mut cfg = config("wss://relay.invalid".into());
        cfg.connect_addr = Some(addr);
        let (report, version) = relay_info(&cfg).await;

        assert_eq!(report.result, Outcome::Failure, "{report:?}");
        assert_eq!(report.failed_stage, Some("nip11"));
        assert!(
            report.error.as_deref().unwrap().contains("301"),
            "{report:?}"
        );
        assert_eq!(version, None);
        server.await.unwrap();
    }

    #[test]
    fn http_url_maps_websocket_schemes() {
        assert_eq!(
            http_url("wss://r.example/").unwrap().as_str(),
            "https://r.example/"
        );
        assert_eq!(
            http_url("ws://127.0.0.1:3000").unwrap().as_str(),
            "http://127.0.0.1:3000/"
        );
        assert!(http_url("https://r.example").is_err());
    }

    #[test]
    fn denied_passes_only_on_the_expected_refusal() {
        let ok = |accepted, message: &str| {
            Ok(buzz_ws_client::OkResponse {
                event_id: "e".into(),
                accepted,
                message: message.into(),
            })
        };
        assert!(denied(ok(false, "auth-required: not authenticated"), AUTH_REQUIRED).is_ok());
        assert!(denied(ok(false, PUBKEY_MISMATCH), PUBKEY_MISMATCH).is_ok());
        assert!(denied(ok(false, "error: database unavailable"), AUTH_REQUIRED).is_err());
        assert!(denied(ok(false, "rate-limited: slow down"), PUBKEY_MISMATCH).is_err());
        assert!(denied(ok(true, ""), AUTH_REQUIRED).is_err());
        assert!(denied(Err(WsClientError::ConnectionClosed), AUTH_REQUIRED).is_err());
        assert!(denied(Err(WsClientError::Timeout), AUTH_REQUIRED).is_err());
    }

    fn check<'a>(report: &'a Report, name: &str) -> &'a CheckReport {
        report.checks.iter().find(|c| c.check == name).unwrap()
    }

    fn stage_seconds(check: &CheckReport, stage: &str) -> f64 {
        check
            .stages
            .iter()
            .find(|s| s.stage == stage)
            .unwrap_or_else(|| panic!("no {stage} stage in {check:?}"))
            .seconds
    }

    /// The `h` tag of every event of `kind` the relay saw.
    fn channels(relay: &fake_relay::FakeRelay, kind: u16) -> Vec<String> {
        relay
            .events()
            .iter()
            .filter(|e| e.kind.as_u16() == kind)
            .filter_map(|e| {
                e.tags
                    .iter()
                    .find(|t| t.kind().to_string() == "h")
                    .and_then(|t| t.content().map(str::to_owned))
            })
            .collect()
    }

    /// The scripted relay serves no NIP-11, so only `relay_info` fails.
    fn assert_all_pass_but_relay_info(report: &Report) {
        for c in &report.checks[1..] {
            assert_eq!(c.result, Outcome::Success, "{c:?}");
        }
    }

    /// The publisher's OK arrives 600 ms after the event reaches the
    /// listener; `receive` must report delivery, not the OK wait.
    #[tokio::test]
    async fn fanout_receive_is_timed_from_delivery_not_the_publishers_ack() {
        let relay = fake_relay::start(fake_relay::Script {
            ack_delay: [(KIND_MESSAGE, Duration::from_millis(600))].into(),
            ..Default::default()
        })
        .await;
        let report = run(&config(relay.url.clone())).await;

        assert_all_pass_but_relay_info(&report);
        let fanout = check(&report, "fanout");
        assert!(stage_seconds(fanout, "publish") >= 0.6, "{fanout:?}");
        assert!(stage_seconds(fanout, "receive") < 0.3, "{fanout:?}");
        let channel = channels(&relay, KIND_CREATE_GROUP);
        assert_eq!(channels(&relay, KIND_DELETE_GROUP), channel);
        let create = relay
            .events()
            .into_iter()
            .find(|e| e.kind.as_u16() == KIND_CREATE_GROUP);
        let ttl = create
            .unwrap()
            .tags
            .iter()
            .find(|t| t.kind().to_string() == "ttl")
            .cloned();
        assert_eq!(
            ttl.and_then(|t| t.content().map(str::to_owned)).as_deref(),
            Some(CHANNEL_TTL_SECS)
        );
    }

    /// The relay serves the probe's messages with altered content under
    /// their original id. `fanout` and `history` must fail, not pass on the
    /// id alone.
    #[tokio::test]
    async fn a_corrupted_event_under_its_original_id_fails_fanout_and_history() {
        let relay = fake_relay::start(fake_relay::Script {
            corrupt: [KIND_MESSAGE].into(),
            ..Default::default()
        })
        .await;
        let report = run(&config(relay.url.clone())).await;

        for (name, stage) in [("fanout", "receive"), ("history", "req_eose")] {
            let c = check(&report, name);
            assert_eq!((c.result, c.failed_stage), (Outcome::Failure, Some(stage)));
            assert!(
                c.error.as_deref().unwrap().contains("failed verification"),
                "{c:?}"
            );
        }
        for name in ["channel_create", "publish", "access_control", "cleanup"] {
            assert_eq!(check(&report, name).result, Outcome::Success, "{name}");
        }
    }

    /// A refusal for another reason, or a closed socket, is not proof the
    /// relay enforces NIP-42: `access_control` fails at that stage.
    #[tokio::test]
    async fn access_control_fails_unless_the_refusal_is_the_auth_denial() {
        for unauthenticated in [
            fake_relay::Unauthenticated::Reason("error: database unavailable".into()),
            fake_relay::Unauthenticated::Close,
        ] {
            let relay = fake_relay::start(fake_relay::Script {
                unauthenticated,
                ..Default::default()
            })
            .await;
            let report = run(&config(relay.url.clone())).await;
            let access = check(&report, "access_control");
            assert_eq!(access.result, Outcome::Failure, "{access:?}");
            assert_eq!(access.failed_stage, Some("unauthenticated_rejected"));
            assert_eq!(check(&report, "cleanup").result, Outcome::Success);
        }
    }

    /// A relay that accepts a write signed by another key is not enforcing
    /// the NIP-42 binding: `access_control` fails at `pubkey_mismatch_rejected`.
    #[tokio::test]
    async fn access_control_fails_when_the_relay_accepts_another_keys_write() {
        let relay = fake_relay::start(fake_relay::Script {
            accept_mismatched: true,
            ..Default::default()
        })
        .await;
        let report = run(&config(relay.url.clone())).await;
        let access = check(&report, "access_control");
        assert_eq!(
            (access.result, access.failed_stage),
            (Outcome::Failure, Some("pubkey_mismatch_rejected")),
            "{access:?}"
        );
        assert!(
            access
                .error
                .as_deref()
                .unwrap()
                .contains("relay accepted the event"),
            "{access:?}"
        );
        assert_eq!(check(&report, "cleanup").result, Outcome::Success);
    }

    /// The create's OK never comes, but the relay stored it: the run fails
    /// at `channel_create`, skips what needed the channel, and still deletes
    /// that exact channel.
    #[tokio::test]
    async fn an_unanswered_create_is_still_deleted() {
        let relay = fake_relay::start(fake_relay::Script {
            no_ack: [KIND_CREATE_GROUP].into(),
            ..Default::default()
        })
        .await;
        let mut cfg = config(relay.url.clone());
        cfg.stage_timeout = Duration::from_millis(500);
        let report = run(&cfg).await;

        let create = check(&report, "channel_create");
        assert_eq!(
            (create.result, create.failed_stage),
            (Outcome::Failure, Some("create"))
        );
        for name in ["publish", "fanout", "history", "access_control"] {
            assert_eq!(check(&report, name).result, Outcome::Skipped, "{name}");
        }
        assert_eq!(check(&report, "cleanup").result, Outcome::Success);
        let created = channels(&relay, KIND_CREATE_GROUP);
        assert_eq!(created.len(), 1);
        assert_eq!(channels(&relay, KIND_DELETE_GROUP), created);
    }

    /// The relay closes the socket right after accepting the create. The
    /// checks on that socket fail, and cleanup deletes the channel over a
    /// fresh connection.
    #[tokio::test]
    async fn a_socket_lost_after_create_still_deletes_the_channel() {
        let relay = fake_relay::start(fake_relay::Script {
            close_after: [KIND_CREATE_GROUP].into(),
            ..Default::default()
        })
        .await;
        let report = run(&config(relay.url.clone())).await;

        assert_eq!(check(&report, "channel_create").result, Outcome::Success);
        assert_eq!(check(&report, "publish").result, Outcome::Failure);
        assert_eq!(check(&report, "cleanup").result, Outcome::Success);
        let created = channels(&relay, KIND_CREATE_GROUP);
        assert_eq!(channels(&relay, KIND_DELETE_GROUP), created);
    }

    /// A cleanup that fails names the channel it could not delete.
    #[tokio::test]
    async fn a_failed_cleanup_names_the_channel() {
        let relay = fake_relay::start(fake_relay::Script {
            no_ack: [KIND_DELETE_GROUP].into(),
            ..Default::default()
        })
        .await;
        let mut cfg = config(relay.url.clone());
        cfg.stage_timeout = Duration::from_millis(500);
        let report = run(&cfg).await;

        let cleanup = check(&report, "cleanup");
        assert_eq!(
            (cleanup.result, cleanup.failed_stage),
            (Outcome::Failure, Some("delete_channel"))
        );
        let channel = &channels(&relay, KIND_CREATE_GROUP)[0];
        assert!(
            cleanup.error.as_deref().unwrap().contains(channel.as_str()),
            "{cleanup:?}"
        );
    }
}
