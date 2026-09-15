//! WebSocket wrapper with an immutable admission deadline and broker liveness.
use futures_util::{Future, Sink, Stream};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio_tungstenite::{
    tungstenite::{Error, Message},
    WebSocketStream,
};

/// A socket's admitted lifetime never extends when a later assertion is issued.
/// The renewal probe also notices desktop logout while a local agent is idle.
pub struct IdentitySocket<S> {
    socket: WebSocketStream<S>,
    ended: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    closed: bool,
}
impl<S> std::fmt::Debug for IdentitySocket<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IdentitySocket([REDACTED])")
    }
}
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> IdentitySocket<S> {
    /// Close using tungstenite's familiar close-frame API.
    pub async fn close(
        &mut self,
        frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>,
    ) -> Result<(), Error> {
        self.socket.close(frame).await
    }
}
impl<S> From<WebSocketStream<S>> for IdentitySocket<S> {
    fn from(socket: WebSocketStream<S>) -> Self {
        Self {
            socket,
            ended: None,
            closed: false,
        }
    }
}
impl<S> IdentitySocket<S> {
    /// Install a fixed deadline plus periodic key-scoped authority check.
    pub fn admitted(
        socket: WebSocketStream<S>,
        expiry: Option<u64>,
        url: String,
        keys: nostr::Keys,
    ) -> Result<Self, crate::federated_identity::IdentityError> {
        let Some(expiry) = expiry else {
            return Ok(socket.into());
        };
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(
                expiry.saturating_sub(crate::federated_identity::unix_now()?),
            );
        let ended = Box::pin(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => return,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(10)) => {}
                }
                tokio::select! {
                    _ = tokio::time::sleep_until(deadline) => return,
                    result = crate::identity_adapter::environment_header(&url, &keys) => {
                        if result.is_err() { return; }
                    }
                }
            }
        });
        Ok(Self {
            socket,
            ended: Some(ended),
            closed: false,
        })
    }
    fn expired(&mut self, cx: &mut Context<'_>) -> bool {
        if self.closed {
            return true;
        }
        if self
            .ended
            .as_mut()
            .is_some_and(|future| future.as_mut().poll(cx).is_ready())
        {
            self.closed = true;
        }
        self.closed
    }
}
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> Stream for IdentitySocket<S> {
    type Item = Result<Message, Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.expired(cx) {
            return Poll::Ready(None);
        }
        Pin::new(&mut self.socket).poll_next(cx)
    }
}
impl<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin> Sink<Message> for IdentitySocket<S> {
    type Error = Error;
    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if self.expired(cx) {
            return Poll::Ready(Err(Error::ConnectionClosed));
        }
        Pin::new(&mut self.socket).poll_ready(cx)
    }
    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Error> {
        if self.closed {
            return Err(Error::ConnectionClosed);
        }
        Pin::new(&mut self.socket).start_send(item)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        if self.expired(cx) {
            return Poll::Ready(Err(Error::ConnectionClosed));
        }
        Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(&mut self.socket).poll_close(cx)
    }
}
