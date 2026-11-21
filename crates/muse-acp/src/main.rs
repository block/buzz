//! `muse-acp` binary: ACP agent over stdio, plus the `auth status` probe.
//!
//! With no arguments, the binary speaks ACP on stdin/stdout (see
//! [`muse_acp::acp`]) and logs to stderr — stdout is the protocol channel
//! and must carry nothing else.
//!
//! `muse-acp auth status` is the desktop catalog's auth probe: it exits 0
//! when a previous Muse sign-in was recorded in the bridge profile, and 1
//! otherwise, without launching a browser.

use std::sync::Arc;

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use muse_acp::acp::run_agent;
use muse_acp::auth;
use muse_acp::muse_web::MuseWebDriver;

const USAGE: &str = "usage: muse-acp [auth status]";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(std::env::var("MUSE_ACP_LOG").unwrap_or_else(|_| "info".to_string()))
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(|arg| arg.as_str())
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => run_stdio().await,
        ["auth", "status"] => {
            if auth::has_saved_session() {
                println!("signed in to Muse (saved bridge session present)");
                std::process::exit(0);
            } else {
                println!("no saved Muse session");
                std::process::exit(1);
            }
        }
        ["--help" | "-h"] => {
            println!("{USAGE}");
            println!();
            println!("Speaks ACP over stdio when run with no arguments.");
            println!("auth status: exit 0 when a saved Muse session exists, 1 otherwise.");
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

/// Wire stdio to the agent loop: stdin lines in, outbound frames to
/// stdout, one frame per line.
async fn run_stdio() {
    let (frames_tx, mut frames_rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(frame) = frames_rx.recv().await {
            if stdout.write_all(frame.as_bytes()).await.is_err() {
                break;
            }
            if stdout.write_all(b"\n").await.is_err() {
                break;
            }
            if stdout.flush().await.is_err() {
                break;
            }
        }
    });
    let driver = Arc::new(MuseWebDriver::new());
    run_agent(BufReader::new(tokio::io::stdin()), frames_tx, driver).await;
    // Input ended: the harness is gone. Stop forwarding and exit rather
    // than waiting on turn tasks that were just cancelled.
    writer.abort();
}
