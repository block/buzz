use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const SHARED_APP_SERVER_URL_ENV: &str = "CODEX_SHARED_APP_SERVER_URL";

/// Bridge the Codex app-server's WebSocket transport to the stdio transport
/// expected by codex-acp. This keeps the adapter as the ACP/app-server protocol
/// translator while allowing several clients to share one long-lived server.
pub async fn run() -> Result<()> {
    let url = std::env::var(SHARED_APP_SERVER_URL_ENV)
        .with_context(|| format!("{SHARED_APP_SERVER_URL_ENV} is required"))?;
    let (socket, _) = connect_async(&url)
        .await
        .with_context(|| format!("failed to connect to shared Codex app-server at {url}"))?;
    let (mut socket_writer, mut socket_reader) = socket.split();

    let mut stdin_lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = BufWriter::new(tokio::io::stdout());

    loop {
        tokio::select! {
            line = stdin_lines.next_line() => {
                match line.context("failed to read app-server request from stdin")? {
                    Some(line) => socket_writer
                        .send(Message::Text(line.into()))
                        .await
                        .context("failed to write app-server request to WebSocket")?,
                    None => {
                        let _ = socket_writer.close().await;
                        break;
                    }
                }
            }
            message = socket_reader.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        stdout
                            .write_all(text.as_bytes())
                            .await
                            .context("failed to write app-server response to stdout")?;
                        stdout.write_all(b"\n").await?;
                        stdout.flush().await?;
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        stdout.write_all(&bytes).await?;
                        stdout.write_all(b"\n").await?;
                        stdout.flush().await?;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        socket_writer.send(Message::Pong(payload)).await?;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Close(frame))) => {
                        return Err(anyhow!("shared app-server WebSocket closed unexpectedly: {frame:?}"));
                    }
                    None => {
                        return Err(anyhow!("shared app-server WebSocket ended unexpectedly"));
                    }
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(error)) => return Err(anyhow!(error).context("shared app-server WebSocket failed")),
                }
            }
        }
    }

    Ok(())
}
