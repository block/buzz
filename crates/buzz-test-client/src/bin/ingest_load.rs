//! Multi-community paced ingest load generator.
//!
//! Drives several communities at independently settable rates against one relay
//! process. That is what separates the two ingest ceilings under investigation:
//! the audit worker is per-pod and aggregate across communities, while the audit
//! advisory lock is per-community and cluster-wide. One community alone cannot
//! tell them apart. See `perf/RELAY_INGEST_CEILING.md`.
//!
//! Reports two latencies per target, because they diverge exactly when the relay
//! saturates and the gap between them is the measurement:
//!
//! - `service_ms` — signed event on the wire until the relay's OK.
//! - `scheduled_ms` — the send's *intended* slot until the OK. Delay the
//!   generator itself added by falling behind stays visible here instead of
//!   silently redefining the offered rate downward.
//!
//! Signing happens before the service clock starts, so BIP340 cost lands in
//! `scheduled_ms` and never inflates the relay's number.
//!
//! Raw per-send samples are deliberately not written: the harness verdict is a
//! ratio of rates, and p50/p95/p99/max cover what a human reads.
//!
//! Usage:
//!   ingest-load <duration_secs> <target> [<target> ...]
//!     target: url=<ws-url>,channel=<uuid>,rate=<events/s>[,conns=<n>]
//!
//! Env:
//!   BENCH_PRIVATE_KEY  hex secret key; must be a channel member on every target

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use buzz_core::kind::KIND_STREAM_MESSAGE;
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio::time::Instant;

/// Connections per target when the spec omits `conns`.
const DEFAULT_CONNS: usize = 8;

/// Padding that puts each event in the size range of a real chat message.
const PADDING: &str = "the quick brown fox jumps over the lazy dog 0123456789";

/// The run's observation window: what was asked for, and what it took.
struct Window {
    requested_secs: f64,
    elapsed_secs: f64,
}

/// One community's offered load: where to send, and how fast.
#[derive(Debug, Clone)]
struct Target {
    url: String,
    channel: String,
    rate: f64,
    conns: usize,
}

/// What one connection, or one whole target, observed.
#[derive(Debug, Default)]
struct Outcome {
    attempted: u64,
    accepted: u64,
    rejected: u64,
    service_ms: Vec<f64>,
    scheduled_ms: Vec<f64>,
    first_rejection: Option<String>,
    first_transport_error: Option<String>,
}

impl Outcome {
    fn absorb(&mut self, other: Self) {
        self.attempted += other.attempted;
        self.accepted += other.accepted;
        self.rejected += other.rejected;
        self.service_ms.extend(other.service_ms);
        self.scheduled_ms.extend(other.scheduled_ms);
        self.first_rejection = self.first_rejection.take().or(other.first_rejection);
        self.first_transport_error = self
            .first_transport_error
            .take()
            .or(other.first_transport_error);
    }
}

fn parse_target(spec: &str) -> anyhow::Result<Target> {
    let mut url = None;
    let mut channel = None;
    let mut rate = None;
    let mut conns = DEFAULT_CONNS;

    for field in spec.split(',') {
        let (key, value) = field
            .split_once('=')
            .ok_or_else(|| anyhow!("target field {field:?} is not key=value"))?;
        match key {
            "url" => url = Some(value.to_string()),
            "channel" => channel = Some(value.to_string()),
            "rate" => rate = Some(value.parse::<f64>().context("rate")?),
            "conns" => conns = value.parse::<usize>().context("conns")?,
            other => bail!("unknown target field {other:?}"),
        }
    }

    let target = Target {
        url: url.ok_or_else(|| anyhow!("target {spec:?} is missing url="))?,
        channel: channel.ok_or_else(|| anyhow!("target {spec:?} is missing channel="))?,
        rate: rate.ok_or_else(|| anyhow!("target {spec:?} is missing rate="))?,
        conns,
    };
    if !target.rate.is_finite() || target.rate <= 0.0 {
        bail!("target rate must be a positive number, got {}", target.rate);
    }
    if target.conns == 0 {
        bail!("target conns must be at least 1");
    }
    Ok(target)
}

/// Sends on one connection against a fixed schedule.
///
/// The schedule advances by a constant interval and is never rebased on the
/// response, so a slow relay produces a rising `scheduled_ms` and a visible
/// shortfall in `attempted` rather than a quietly reduced offer.
async fn drive_connection(
    mut client: BuzzTestClient,
    keys: Keys,
    channel: String,
    label: String,
    first_slot: Instant,
    interval: Duration,
    deadline: Instant,
) -> anyhow::Result<Outcome> {
    let h_tag = Tag::parse(["h", channel.as_str()]).map_err(|e| anyhow!("h tag: {e}"))?;
    let kind = Kind::Custom(u16::try_from(KIND_STREAM_MESSAGE).context("stream message kind")?);

    let mut out = Outcome::default();
    let mut slot = first_slot;
    let mut seq: u64 = 0;

    while slot < deadline && Instant::now() < deadline {
        tokio::time::sleep_until(slot).await;
        seq += 1;
        let event = EventBuilder::new(kind, format!("{label} seq={seq} {PADDING}"))
            .tags([h_tag.clone()])
            .sign_with_keys(&keys)?;

        let sent_at = Instant::now();
        let response = client.send_event(event).await;
        let settled_at = Instant::now();

        let ok = match response {
            Ok(ok) => ok,
            Err(e) => {
                // A dead connection ends this sender but keeps its samples:
                // losing them would hide the saturation that killed it.
                out.first_transport_error = Some(e.to_string());
                break;
            }
        };

        out.attempted += 1;
        out.service_ms
            .push((settled_at - sent_at).as_secs_f64() * 1e3);
        out.scheduled_ms
            .push((settled_at - slot).as_secs_f64() * 1e3);
        if ok.accepted {
            out.accepted += 1;
        } else {
            out.rejected += 1;
            if out.first_rejection.is_none() {
                out.first_rejection = Some(ok.message);
            }
        }
        slot += interval;
    }

    if let Err(e) = client.disconnect().await {
        out.first_transport_error = out.first_transport_error.or(Some(e.to_string()));
    }
    Ok(out)
}

fn percentiles(samples: &mut [f64]) -> Value {
    samples.sort_by(|a, b| a.total_cmp(b));
    let at = |p: f64| -> Option<f64> {
        let last = samples.len().checked_sub(1)?;
        let idx = (last as f64 * p).round() as usize;
        samples.get(idx).copied()
    };
    json!({ "p50": at(0.50), "p95": at(0.95), "p99": at(0.99), "max": at(1.0) })
}

fn summarize(target: &Target, window: &Window, out: &mut Outcome) -> Value {
    let service = percentiles(&mut out.service_ms);
    let achieved = out.accepted as f64 / window.elapsed_secs;

    // The ceiling this generator imposes on itself: each connection is
    // closed-loop, so it cannot exceed one send per service latency. A reader
    // comparing `achieved_per_s` against this can tell a relay ceiling from a
    // generator ceiling; raise `conns` when they are close.
    let conn_capacity = service["p50"]
        .as_f64()
        .filter(|p50| *p50 > 0.0)
        .map(|p50| target.conns as f64 / (p50 / 1e3));

    json!({
        "url": target.url,
        "channel": target.channel,
        "conns": target.conns,
        "offered_per_s": target.rate,
        "attempted": out.attempted,
        "accepted": out.accepted,
        "rejected": out.rejected,
        "achieved_per_s": achieved,
        // Fraction of the whole offer that was accepted. The denominator is the
        // offer the run asked for, not the rate re-derived from elapsed time, so
        // a run that finishes a hair early cannot report better than 1.0.
        "achieved_over_offered": out.accepted as f64 / (target.rate * window.requested_secs),
        "conn_capacity_per_s": conn_capacity,
        "service_ms": service,
        "scheduled_ms": percentiles(&mut out.scheduled_ms),
        "first_rejection": out.first_rejection,
        "first_transport_error": out.first_transport_error,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Errors only when another initializer already installed a provider, which
    // is the same outcome we want.
    let _ = rustls::crypto::CryptoProvider::install_default(
        rustls::crypto::aws_lc_rs::default_provider(),
    );

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (duration_arg, target_args) = args
        .split_first()
        .filter(|(_, targets)| !targets.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "usage: ingest-load <duration_secs> <target> [<target> ...]\n  \
                 target: url=<ws-url>,channel=<uuid>,rate=<events/s>[,conns=<n>]"
            )
        })?;

    let duration_secs: u64 = duration_arg.parse().context("duration_secs")?;
    let targets: Vec<Target> = target_args
        .iter()
        .map(|spec| parse_target(spec))
        .collect::<anyhow::Result<_>>()?;

    let keys = Keys::parse(
        &std::env::var("BENCH_PRIVATE_KEY")
            .context("BENCH_PRIVATE_KEY is required (a channel member's secret key)")?,
    )?;

    // Connect everything before the clock starts. Otherwise one target is
    // already publishing while another is still in its NIP-42 handshake, and
    // the per-community rates were never concurrent.
    let mut connected = Vec::new();
    for (t_idx, target) in targets.iter().enumerate() {
        for conn_idx in 0..target.conns {
            let client = BuzzTestClient::connect(&target.url, &keys)
                .await
                .with_context(|| format!("connecting to {}", target.url))?;
            connected.push((t_idx, conn_idx, client));
        }
    }

    let start = Instant::now();
    let deadline = start + Duration::from_secs(duration_secs);
    let mut tasks = Vec::new();
    for (t_idx, conn_idx, client) in connected {
        let target = targets
            .get(t_idx)
            .ok_or_else(|| anyhow!("target index {t_idx} vanished"))?
            .clone();
        // Stagger connections within a target so the aggregate offer is evenly
        // spaced at `rate` rather than arriving in bursts of `conns`.
        let first_slot = start + Duration::from_secs_f64(conn_idx as f64 / target.rate);
        let interval = Duration::from_secs_f64(target.conns as f64 / target.rate);
        let label = format!("ingest-load t{t_idx} c{conn_idx}");
        let keys = keys.clone();
        tasks.push(tokio::spawn(async move {
            let channel = target.channel.clone();
            let out =
                drive_connection(client, keys, channel, label, first_slot, interval, deadline)
                    .await?;
            Ok::<_, anyhow::Error>((t_idx, out))
        }));
    }

    let mut per_target: Vec<Outcome> = targets.iter().map(|_| Outcome::default()).collect();
    for task in tasks {
        let (t_idx, out) = task.await??;
        per_target
            .get_mut(t_idx)
            .ok_or_else(|| anyhow!("target index {t_idx} vanished"))?
            .absorb(out);
    }

    let window = Window {
        requested_secs: duration_secs as f64,
        elapsed_secs: start.elapsed().as_secs_f64(),
    };
    let mut aggregate = Outcome::default();
    let mut offered_total = 0.0;
    let mut summaries = Vec::new();
    for (target, mut out) in targets.iter().zip(per_target) {
        offered_total += target.rate;
        summaries.push(summarize(target, &window, &mut out));
        aggregate.absorb(out);
    }

    let achieved_total = aggregate.accepted as f64 / window.elapsed_secs;
    println!(
        "{}",
        json!({
            "duration_secs": duration_secs,
            "elapsed_secs": window.elapsed_secs,
            "targets": summaries,
            "aggregate": {
                "offered_per_s": offered_total,
                "attempted": aggregate.attempted,
                "accepted": aggregate.accepted,
                "rejected": aggregate.rejected,
                "achieved_per_s": achieved_total,
                "achieved_over_offered":
                    aggregate.accepted as f64 / (offered_total * window.requested_secs),
                "service_ms": percentiles(&mut aggregate.service_ms),
                "scheduled_ms": percentiles(&mut aggregate.scheduled_ms),
            },
        })
    );
    Ok(())
}
