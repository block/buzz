use std::io;

use anyhow::Context;
use buzz_ifc_broker::{Broker, EnforcementMode};
use clap::{Parser, ValueEnum};
use futures_util::StreamExt;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::codec::{FramedRead, LinesCodec};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Parser)]
#[command(name = "buzz-ifc-broker", version, about)]
struct Args {
    /// Whether decisions are observational or must be enforced by the adapter.
    #[arg(long, env = "BUZZ_IFC_BROKER_MODE", default_value = "audit")]
    mode: Mode,
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Audit,
    Enforce,
}

impl From<Mode> for EnforcementMode {
    fn from(value: Mode) -> Self {
        match value {
            Mode::Audit => Self::Audit,
            Mode::Enforce => Self::Enforce,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(io::stderr)
        .try_init();

    let mode = EnforcementMode::from(args.mode);
    tracing::info!(%mode, "Buzz IFC broker started");
    let mut broker = Broker::new(mode);
    let stdin = tokio::io::stdin();
    let mut frames = FramedRead::new(stdin, LinesCodec::new_with_max_length(MAX_REQUEST_BYTES));
    let mut stdout = BufWriter::new(tokio::io::stdout());

    while let Some(frame) = frames.next().await {
        let response = match frame {
            Ok(line) => broker.handle_line(&line),
            Err(error) => {
                tracing::warn!(%error, "Buzz IFC broker rejected an input frame");
                Broker::frame_error("request frame exceeds the broker limit")
            }
        };
        stdout
            .write_all(response.as_bytes())
            .await
            .context("write broker response")?;
        stdout.write_all(b"\n").await.context("write line ending")?;
        stdout.flush().await.context("flush broker response")?;
    }
    Ok(())
}
