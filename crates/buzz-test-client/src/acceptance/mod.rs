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

pub mod metrics;

use std::future::Future;
use std::path::PathBuf;
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
            run_channel_checks(cfg, &mut conn, &mut checks).await;
            // Closing is best effort: every check has already been recorded.
            let _ = conn.disconnect().await;
        }
        None => {
            for &check in &CHECKS[2..] {
                checks.push(skipped(check, "connect_auth"));
            }
        }
    }

    Report {
        relay_url: cfg.relay_url.clone(),
        relay_version,
        passed: checks.iter().all(|c| c.result != Outcome::Failure),
        seconds: started.elapsed().as_secs_f64(),
        checks,
    }
}

/// The checks that need the authenticated connection `conn`.
async fn run_channel_checks(
    cfg: &Config,
    conn: &mut NostrWsConnection,
    checks: &mut Vec<CheckReport>,
) {
    let (report, channel) = channel_create(cfg, conn).await;
    checks.push(report);
    let Some(channel) = channel else {
        for &check in &CHECKS[3..] {
            checks.push(skipped(check, "channel_create"));
        }
        return;
    };

    let (report, published) = publish(cfg, conn, &channel).await;
    checks.push(report);
    checks.push(fanout(cfg, conn, &channel).await);
    checks.push(match published {
        Some(id) => history(cfg, conn, &channel, id).await,
        None => skipped("history", "publish"),
    });
    checks.push(access_control(cfg, conn, &channel).await);
    checks.push(cleanup(cfg, conn, &channel).await);
}

async fn relay_info(cfg: &Config) -> (CheckReport, Option<String>) {
    let mut check = Check::new("relay_info", cfg.stage_timeout);
    let result = check
        .stage("nip11", async {
            let url = http_url(&cfg.relay_url)?;
            let response = reqwest::Client::new()
                .get(url)
                .header("Accept", "application/nostr+json")
                .send()
                .await?
                .error_for_status()?;
            let info: serde_json::Value = response.json().await?;
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
        let mut conn = check
            .stage("ws_connect", async {
                Ok(NostrWsConnection::connect_with_headers(&cfg.relay_url, upgrade_headers(cfg)?)
                    .await?)
            })
            .await?;
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

async fn channel_create(cfg: &Config, conn: &mut NostrWsConnection) -> (CheckReport, Option<String>) {
    let mut check = Check::new("channel_create", cfg.stage_timeout);
    let channel = uuid::Uuid::new_v4().to_string();
    let result = check
        .stage("create", async {
            let event = EventBuilder::new(Kind::Custom(KIND_CREATE_GROUP), "")
                .tags([
                    tag(["h", &channel])?,
                    tag(["name", &format!("acceptance-{channel}")])?,
                    tag(["channel_type", "stream"])?,
                    tag(["visibility", "private"])?,
                ])
                .sign_with_keys(&cfg.keys)?;
            accepted(conn, event).await
        })
        .await;
    match result {
        Ok(()) => (check.finish(Ok(())), Some(channel)),
        Err(e) => (check.finish(Err(e)), None),
    }
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
/// second must receive it. `receive` is timed from the publish to the moment
/// the event arrived on the second socket.
async fn fanout(cfg: &Config, conn: &mut NostrWsConnection, channel: &str) -> CheckReport {
    let mut check = Check::new("fanout", cfg.stage_timeout);
    let result = async {
        let mut listener = check
            .stage("connect_second", async {
                Ok(NostrWsConnection::connect_authenticated_with_headers(
                    &cfg.relay_url,
                    &cfg.keys,
                    None,
                    upgrade_headers(cfg)?,
                )
                .await?)
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
    let event = message(&cfg.keys, channel, "acceptance fanout")
        .map_err(|e| StageError { stage: "publish", error: format!("{e:#}") })?;
    let id = event.id;
    let sent_at = tokio::time::Instant::now();
    check.stage("publish", accepted(conn, event)).await?;
    let arrived = async {
        loop {
            match listener.next_event_received(timeout).await? {
                (at, RelayMessage::Event { event, .. }) if event.id == id => return Ok(at),
                (_, RelayMessage::Closed { message, .. }) => bail!("subscription closed: {message}"),
                _ => {}
            }
        }
    };
    match tokio::time::timeout(timeout, arrived).await {
        Ok(Ok(at)) => {
            check.stages.push(StageTiming {
                stage: "receive",
                seconds: at.saturating_duration_since(sent_at).as_secs_f64(),
            });
            Ok(())
        }
        Ok(Err(e)) => Err(StageError { stage: "receive", error: format!("{e:#}") }),
        Err(_) => Err(StageError {
            stage: "receive",
            error: format!("timed out after {timeout:?}"),
        }),
    }
}

/// A fresh REQ for the channel must return the event `publish` stored.
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
                    RelayMessage::Event { subscription_id, event } if subscription_id == "history" => {
                        found |= event.id == id;
                    }
                    RelayMessage::Eose { subscription_id } if subscription_id == "history" => break,
                    RelayMessage::Closed { subscription_id, message } if subscription_id == "history" => {
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

/// The relay must refuse a write from a connection that has not
/// authenticated, and a write signed by a key other than the one the
/// connection authenticated as.
async fn access_control(cfg: &Config, conn: &mut NostrWsConnection, channel: &str) -> CheckReport {
    let mut check = Check::new("access_control", cfg.stage_timeout);
    let result = async {
        check
            .stage("unauthenticated_rejected", async {
                let stranger = Keys::generate();
                let event = message(&stranger, channel, "acceptance unauthenticated")?;
                let mut anon = match NostrWsConnection::connect(&cfg.relay_url).await {
                    Ok(anon) => anon,
                    // A relay that requires federated identity refuses the upgrade.
                    Err(e) if matches!(e.upgrade_rejection(), Some((401 | 403, _))) => return Ok(()),
                    Err(e) => return Err(e.into()),
                };
                let outcome = anon.send_event(event).await;
                let _ = anon.disconnect().await;
                refused(outcome)
            })
            .await?;
        check
            .stage("pubkey_mismatch_rejected", async {
                let event = message(&Keys::generate(), channel, "acceptance mismatch")?;
                refused(conn.send_event(event).await)
            })
            .await
    }
    .await;
    check.finish(result)
}

async fn cleanup(cfg: &Config, conn: &mut NostrWsConnection, channel: &str) -> CheckReport {
    let mut check = Check::new("cleanup", cfg.stage_timeout);
    let result = check
        .stage("delete_channel", async {
            let event = EventBuilder::new(Kind::Custom(KIND_DELETE_GROUP), "")
                .tags([tag(["h", channel])?])
                .sign_with_keys(&cfg.keys)?;
            accepted(conn, event).await
        })
        .await;
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

/// Passes when the relay refused a write: `OK false`, or closed the socket.
fn refused(outcome: Result<buzz_ws_client::OkResponse, WsClientError>) -> anyhow::Result<()> {
    match outcome {
        Ok(ok) if ok.accepted => bail!("relay accepted the event"),
        Ok(_) | Err(WsClientError::ConnectionClosed | WsClientError::PolicyClose(_)) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

async fn wait_for_eose(
    conn: &mut NostrWsConnection,
    sub_id: &str,
    timeout: Duration,
) -> anyhow::Result<()> {
    loop {
        match conn.next_event(timeout).await? {
            RelayMessage::Eose { subscription_id } if subscription_id == sub_id => return Ok(()),
            RelayMessage::Closed { subscription_id, message } if subscription_id == sub_id => {
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
        let assertion = std::fs::read_to_string(path)
            .with_context(|| format!("read assertion file {}", path.display()))?;
        let assertion = assertion.trim();
        if assertion.is_empty() {
            bail!("assertion file {} is empty", path.display());
        }
        let mut value = HeaderValue::from_str(&format!("Bearer {assertion}"))
            .map_err(|_| anyhow!("assertion file {} is not a header value", path.display()))?;
        value.set_sensitive(true);
        headers.insert(FEDERATED_IDENTITY_HEADER, value);
    }
    Ok(headers)
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
        assert_eq!(results[1], ("connect_auth", Outcome::Failure, Some("ws_connect")));
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
        assert_eq!(upgrade_headers(&cfg).unwrap()[FEDERATED_IDENTITY_HEADER], "Bearer first");
        std::fs::write(&path, "second").unwrap();
        assert_eq!(upgrade_headers(&cfg).unwrap()[FEDERATED_IDENTITY_HEADER], "Bearer second");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn http_url_maps_websocket_schemes() {
        assert_eq!(http_url("wss://r.example/").unwrap().as_str(), "https://r.example/");
        assert_eq!(http_url("ws://127.0.0.1:3000").unwrap().as_str(), "http://127.0.0.1:3000/");
        assert!(http_url("https://r.example").is_err());
    }

    #[test]
    fn refused_passes_only_when_the_relay_did_not_accept() {
        let ok = |accepted| buzz_ws_client::OkResponse {
            event_id: "e".into(),
            accepted,
            message: String::new(),
        };
        assert!(refused(Ok(ok(false))).is_ok());
        assert!(refused(Err(WsClientError::ConnectionClosed)).is_ok());
        assert!(refused(Ok(ok(true))).is_err());
        assert!(refused(Err(WsClientError::Timeout)).is_err());
    }
}
