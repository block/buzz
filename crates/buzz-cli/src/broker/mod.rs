//! A deliberately small local broker: one signer, one channel, one IFC session.
//!
//! The operator chooses the scope before the socket exists. A socket caller
//! cannot choose a relay, channel, audience, event kind, or signing payload.
//! This is not a same-user security boundary: the executable, environment,
//! socket, and agent's other inputs are not isolated by the OS. See README.md.

mod relay;
#[cfg(test)]
mod tests;

use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};

use buzz_ifc::{ExecutionDomain, IfcSession, ResourceLabel};
use nostr::{Keys, PublicKey, Tag};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    time::timeout,
};
use uuid::Uuid;

use crate::error::CliError;
use relay::Relay;

const READ: &str = "channel.read";
const REPLY: &str = "message.post";
const REQUEST_BYTES: usize = 32 * 1024;
const RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(30);

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Hold the key and serve one fixed channel; prints the socket path as JSON
    Serve(Scope),
    /// Read recent messages without a private key
    Read {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Post plain text to the broker's channel (no thread or mention expansion)
    Reply {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long)]
        content: String,
    },
}

/// Trusted startup configuration, never deserialized from a socket request.
#[derive(Clone, clap::Args)]
pub(crate) struct Scope {
    /// Community UUID belonging to the configured relay, supplied by its operator
    #[arg(long)]
    community: Uuid,
    /// The only channel this broker may read or write
    #[arg(long)]
    channel: Uuid,
    /// Pinned relay signing key for channel metadata and membership
    #[arg(long)]
    relay_key: PublicKey,
    /// Person authorizing this work; must be a current channel member
    #[arg(long)]
    requester: PublicKey,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Read { limit: u32 },
    Reply { content: String },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Response {
    Ok { result: Value },
    Error { message: String },
}

struct Broker {
    relay: Relay,
    scope: Scope,
    session: IfcSession,
}

impl Broker {
    async fn open(relay: Relay, scope: Scope) -> Result<Self, CliError> {
        let domain = relay.domain(&scope).await?;
        Ok(Self {
            relay,
            scope,
            session: IfcSession::enter(domain),
        })
    }

    async fn check_scope(&mut self) -> Result<ExecutionDomain, CliError> {
        let current = match self.relay.domain(&self.scope).await {
            Ok(domain) => domain,
            Err(error) => {
                self.session.mark_unknown_input();
                return Err(error);
            }
        };
        if current.key() != self.session.domain_key() {
            // Never replace the session: the agent may still hold data from
            // the old audience. Even a subsequent policy rollback cannot
            // make this instance safe to publish again.
            self.session.mark_unknown_input();
            return Err(denied(
                "channel policy changed; start a fresh agent and broker",
            ));
        }
        self.session
            .read(&ResourceLabel::from_domain(&current))
            .map_err(|_| denied("resource is outside the session"))?;
        Ok(current)
    }

    async fn handle(&mut self, request: Request) -> Result<Value, CliError> {
        match &request {
            Request::Read { limit } if !(1..=100).contains(limit) => {
                return Err(denied("read limit must be between 1 and 100"))
            }
            Request::Reply { content }
                if content.trim().is_empty() || content.len() > 16 * 1024 =>
            {
                return Err(denied("reply must contain 1 to 16384 bytes of text"))
            }
            _ => {}
        }
        let domain = self.check_scope().await?;
        match request {
            Request::Read { limit } => {
                self.session
                    .call(READ)
                    .map_err(|_| denied("read is not allowed"))?;
                let events = self.relay.read(self.scope.channel, limit).await?;
                // Do not deliver a fetch that overlapped an observed policy
                // change. A fresh query is not an atomic relay transaction;
                // the remaining race is documented alongside this prototype.
                self.check_scope().await?;
                Ok(json!(events))
            }
            Request::Reply { content } => {
                let event = self.relay.message(self.scope.channel, &content)?;
                let bytes =
                    serde_json::to_vec(&event).map_err(|_| denied("cannot encode reply"))?;
                let authorization = self
                    .session
                    .publish(REPLY, domain.audience(), bytes)
                    .map_err(|_| denied("session cannot publish"))?;
                self.relay.publish(authorization, event.id).await
            }
        }
    }
}

pub(crate) async fn serve(
    url: String,
    keys: Keys,
    auth: Option<Tag>,
    scope: Scope,
) -> Result<(), CliError> {
    eprintln!("WARNING: local broker prototype; no OS isolation from same-user processes.");
    let relay = Relay::new(url, keys, auth)?;
    let mut broker = Broker::open(relay, scope).await?;
    let (_directory, listener, socket) = bind_socket()?;
    println!(
        "{}",
        json!({"socket": socket, "channel": broker.scope.channel})
    );
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.map_err(io_error)?;
                return Ok(());
            }
            incoming = listener.accept() => {
                let (stream, _) = incoming.map_err(io_error)?;
                // One request at a time keeps IFC state changes serialized.
                // A silent client cannot hold the broker forever.
                if let Err(error) = connection(&mut broker, stream).await {
                    eprintln!("local broker connection failed: {error}");
                }
            }
        }
    }
}

fn bind_socket() -> Result<(tempfile::TempDir, UnixListener, PathBuf), CliError> {
    // tempfile creates a new 0700 directory. Never unlink a caller-selected
    // socket or reuse its name for a new IFC session after a restart.
    let directory = tempfile::Builder::new()
        .prefix("buzz-broker-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .map_err(io_error)?;
    let socket = directory.path().join("broker.sock");
    let listener = UnixListener::bind(&socket).map_err(io_error)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).map_err(io_error)?;
    Ok((directory, listener, socket))
}

async fn connection(broker: &mut Broker, mut stream: UnixStream) -> Result<(), CliError> {
    timeout(DEADLINE, async {
        let result = match receive::<Request>(&mut stream, REQUEST_BYTES).await {
            Ok(request) => broker.handle(request).await,
            Err(error) => Err(error),
        };
        let response = match result {
            Ok(result) => Response::Ok { result },
            Err(error) => Response::Error {
                message: error.to_string(),
            },
        };
        send(&mut stream, &response, RESPONSE_BYTES).await
    })
    .await
    .map_err(|_| denied("broker request timed out; publication outcome may be unknown"))?
}

pub(crate) async fn call(command: &Command) -> Result<(), CliError> {
    let (socket, request) = match command {
        Command::Read { socket, limit } => (socket, Request::Read { limit: *limit }),
        Command::Reply { socket, content } => (
            socket,
            Request::Reply {
                content: content.clone(),
            },
        ),
        Command::Serve(_) => return Err(denied("serve requires the operator's credentials")),
    };
    let result = timeout(DEADLINE + Duration::from_secs(5), async {
        let mut stream = UnixStream::connect(socket).await.map_err(io_error)?;
        send(&mut stream, &request, REQUEST_BYTES).await?;
        match receive::<Response>(&mut stream, RESPONSE_BYTES).await? {
            Response::Ok { result } => Ok(result),
            Response::Error { message } => Err(denied(message)),
        }
    })
    .await
    .map_err(|_| denied("broker did not respond; publication outcome may be unknown"))??;
    println!("{result}");
    Ok(())
}

// Length-prefixed JSON bounds allocation before decoding. No agent-controlled
// string reaches a shell, a URL, or a relay filter.
async fn receive<T: DeserializeOwned>(stream: &mut UnixStream, max: usize) -> Result<T, CliError> {
    let len = stream.read_u32().await.map_err(io_error)? as usize;
    if len > max {
        return Err(denied("broker frame is too large"));
    }
    let mut bytes = vec![0; len];
    stream.read_exact(&mut bytes).await.map_err(io_error)?;
    serde_json::from_slice(&bytes).map_err(|_| denied("invalid broker request or response"))
}

async fn send<T: Serialize>(
    stream: &mut UnixStream,
    value: &T,
    max: usize,
) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(value).map_err(|_| denied("cannot encode broker frame"))?;
    if bytes.len() > max {
        return Err(denied("broker frame is too large"));
    }
    stream
        .write_u32(bytes.len() as u32)
        .await
        .map_err(io_error)?;
    stream.write_all(&bytes).await.map_err(io_error)
}

fn denied(message: impl Into<String>) -> CliError {
    CliError::Other(message.into())
}
fn io_error(error: std::io::Error) -> CliError {
    denied(format!("local broker I/O failed: {error}"))
}
