//! Paced kind:9 load generator for relay write-amplification benchmarking.
//!
//! Opens `conns` authenticated WebSocket connections (one shared identity)
//! and sends kind:9 text events to `channel_uuid` at a total rate of `qps`
//! for `duration_secs`. Each connection is synchronous (send -> await OK),
//! paced by a per-connection tokio interval, so OK latency is measured
//! end-to-end. Emits a JSON summary on stdout and one raw latency sample
//! (milliseconds, f64) per line to `latency_out`.
//!
//! Usage: wamp-bench <channel_uuid> <qps> <duration_secs> <conns> <latency_out>
//! Env:   BUZZ_RELAY_URL (default ws://localhost:3000), BENCH_PRIVATE_KEY (hex)

use std::time::{Duration, Instant};

use buzz_test_client::BuzzTestClient;
use nostr::Keys;
use tokio::time::MissedTickBehavior;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = rustls::crypto::CryptoProvider::install_default(
        rustls::crypto::aws_lc_rs::default_provider(),
    );
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        eprintln!("Usage: wamp-bench <channel_uuid> <qps> <duration_secs> <conns> <latency_out>");
        std::process::exit(1);
    }
    let channel_id = args[1].clone();
    let qps: f64 = args[2].parse()?;
    let duration_secs: u64 = args[3].parse()?;
    let conns: usize = args[4].parse()?;
    let latency_out = args[5].clone();

    let url = std::env::var("BUZZ_RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".into());
    let keys = match std::env::var("BENCH_PRIVATE_KEY") {
        Ok(hex) => Keys::parse(&hex)?,
        Err(_) => anyhow::bail!("BENCH_PRIVATE_KEY is required (channel member secret key)"),
    };

    let per_conn_interval = Duration::from_secs_f64(conns as f64 / qps);
    let deadline = Instant::now() + Duration::from_secs(duration_secs);

    let mut tasks = Vec::new();
    for conn_idx in 0..conns {
        let url = url.clone();
        let keys = keys.clone();
        let channel_id = channel_id.clone();
        tasks.push(tokio::spawn(async move {
            let mut client = BuzzTestClient::connect(&url, &keys).await?;
            let mut interval = tokio::time::interval(per_conn_interval);
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
            let mut latencies: Vec<f64> = Vec::new();
            let mut sent: u64 = 0;
            let mut rejected: u64 = 0;
            let mut seq: u64 = 0;
            while Instant::now() < deadline {
                interval.tick().await;
                seq += 1;
                let content = format!(
                    "wamp-bench c{conn_idx} m{seq} payload: the quick brown fox jumps over the lazy dog 0123456789"
                );
                let start = Instant::now();
                let ok = client
                    .send_text_message(&keys, &channel_id, &content, 9)
                    .await?;
                let elapsed_ms = start.elapsed().as_secs_f64() * 1e3;
                sent += 1;
                if ok.accepted {
                    latencies.push(elapsed_ms);
                } else {
                    rejected += 1;
                    eprintln!("REJECTED conn={conn_idx} seq={seq}: {}", ok.message);
                }
            }
            client.disconnect().await?;
            Ok::<_, anyhow::Error>((sent, rejected, latencies))
        }));
    }

    let mut sent = 0u64;
    let mut rejected = 0u64;
    let mut latencies: Vec<f64> = Vec::new();
    for task in tasks {
        let (s, r, l) = task.await??;
        sent += s;
        rejected += r;
        latencies.extend(l);
    }
    latencies.sort_by(|a, b| a.partial_cmp(b).expect("finite latencies"));
    let pct = |p: f64| -> f64 {
        if latencies.is_empty() {
            return f64::NAN;
        }
        let idx = ((latencies.len() as f64 - 1.0) * p).round() as usize;
        latencies[idx]
    };
    let raw: String = latencies.iter().map(|l| format!("{l:.3}\n")).collect();
    std::fs::write(&latency_out, raw)?;
    println!(
        "{}",
        serde_json::json!({
            "sent": sent,
            "accepted": sent - rejected,
            "rejected": rejected,
            "qps_target": qps,
            "duration_secs": duration_secs,
            "conns": conns,
            "ok_latency_ms": {
                "p50": pct(0.50),
                "p95": pct(0.95),
                "p99": pct(0.99),
                "max": latencies.last().copied().unwrap_or(f64::NAN),
            },
        })
    );
    Ok(())
}
