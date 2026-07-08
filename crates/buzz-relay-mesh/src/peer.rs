use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::{
    encode_datagram_checked, wire, MeshDatagram, MeshError, MeshStream, MeshStreamFrame, RuntimeId,
    StreamRecvHalf, StreamSendHalf, ALPN,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PeerCounters {
    pub streams_opened: u64,
    pub streams_accepted: u64,
    pub datagrams_sent: u64,
    pub datagrams_received: u64,
}

#[derive(Debug, Default)]
struct PeerCountersInner {
    streams_opened: AtomicU64,
    streams_accepted: AtomicU64,
    datagrams_sent: AtomicU64,
    datagrams_received: AtomicU64,
}

impl PeerCountersInner {
    fn snapshot(&self) -> PeerCounters {
        PeerCounters {
            streams_opened: self.streams_opened.load(Ordering::Relaxed),
            streams_accepted: self.streams_accepted.load(Ordering::Relaxed),
            datagrams_sent: self.datagrams_sent.load(Ordering::Relaxed),
            datagrams_received: self.datagrams_received.load(Ordering::Relaxed),
        }
    }
}

/// Authenticated iroh connection to one peer runtime.
#[derive(Debug, Clone)]
pub struct MeshPeer {
    _endpoint: iroh::Endpoint,
    conn: iroh::endpoint::Connection,
    runtime_id: RuntimeId,
    counters: Arc<PeerCountersInner>,
}

impl MeshPeer {
    pub(crate) fn from_connection(
        endpoint: iroh::Endpoint,
        conn: iroh::endpoint::Connection,
    ) -> Result<Self, MeshError> {
        if conn.alpn() != ALPN {
            return Err(MeshError::Transport(format!(
                "unexpected mesh ALPN {}",
                String::from_utf8_lossy(conn.alpn())
            )));
        }

        Ok(Self {
            _endpoint: endpoint,
            runtime_id: crate::endpoint::runtime_id_from_public_key(conn.remote_id()),
            conn,
            counters: Arc::default(),
        })
    }

    pub fn runtime_id(&self) -> RuntimeId {
        self.runtime_id
    }

    pub fn max_datagram_size(&self) -> Option<usize> {
        self.conn.max_datagram_size()
    }

    pub fn counters(&self) -> PeerCounters {
        self.counters.snapshot()
    }

    pub async fn open_bi(&self) -> Result<MeshStream, MeshError> {
        let (send, recv) = self
            .conn
            .open_bi()
            .await
            .map_err(|err| MeshError::Transport(err.to_string()))?;
        self.counters.streams_opened.fetch_add(1, Ordering::Relaxed);
        Ok(MeshStream::new(
            Box::new(IrohSendHalf(send)),
            Box::new(IrohRecvHalf(recv)),
        ))
    }

    pub async fn accept_bi(&self) -> Result<MeshStream, MeshError> {
        let (send, recv) = self
            .conn
            .accept_bi()
            .await
            .map_err(|err| MeshError::Transport(err.to_string()))?;
        self.counters
            .streams_accepted
            .fetch_add(1, Ordering::Relaxed);
        Ok(MeshStream::new(
            Box::new(IrohSendHalf(send)),
            Box::new(IrohRecvHalf(recv)),
        ))
    }

    pub fn send_datagram(&self, dgram: &MeshDatagram) -> Result<(), MeshError> {
        let max = self
            .conn
            .max_datagram_size()
            .ok_or_else(|| MeshError::Transport("peer does not support QUIC datagrams".into()))?;
        let bytes = encode_datagram_checked(dgram, max)?;
        self.conn
            .send_datagram(bytes)
            .map_err(|err| MeshError::Transport(err.to_string()))?;
        self.counters.datagrams_sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub async fn recv_datagram(&self) -> Result<MeshDatagram, MeshError> {
        let bytes = self
            .conn
            .read_datagram()
            .await
            .map_err(|err| MeshError::Transport(err.to_string()))?;
        let dgram = wire::decode::<MeshDatagram>(&bytes)?;
        self.counters
            .datagrams_received
            .fetch_add(1, Ordering::Relaxed);
        Ok(dgram)
    }
}

struct IrohSendHalf(iroh::endpoint::SendStream);
struct IrohRecvHalf(iroh::endpoint::RecvStream);

impl StreamSendHalf for IrohSendHalf {
    fn send_frame(
        &mut self,
        frame: MeshStreamFrame,
    ) -> crate::BoxFuture<'_, Result<(), MeshError>> {
        Box::pin(async move {
            let bytes = wire::encode(&frame)?;
            if bytes.len() > wire::MAX_STREAM_FRAME as usize {
                return Err(MeshError::FrameTooLarge {
                    size: bytes.len(),
                    max: wire::MAX_STREAM_FRAME as usize,
                });
            }
            self.0
                .write_all(&(bytes.len() as u32).to_le_bytes())
                .await
                .map_err(|err| MeshError::Transport(err.to_string()))?;
            self.0
                .write_all(&bytes)
                .await
                .map_err(|err| MeshError::Transport(err.to_string()))?;
            Ok(())
        })
    }

    fn finish(&mut self) -> Result<(), MeshError> {
        self.0
            .finish()
            .map_err(|err| MeshError::Transport(err.to_string()))
    }
}

impl StreamRecvHalf for IrohRecvHalf {
    fn recv_frame(&mut self) -> crate::BoxFuture<'_, Result<Option<MeshStreamFrame>, MeshError>> {
        Box::pin(async move {
            let mut len = [0u8; 4];
            match self.0.read_exact(&mut len).await {
                Ok(_) => {}
                Err(iroh::endpoint::ReadExactError::FinishedEarly(0)) => return Ok(None),
                Err(err) => return Err(MeshError::Transport(err.to_string())),
            }

            let len = u32::from_le_bytes(len);
            if len > wire::MAX_STREAM_FRAME {
                return Err(MeshError::FrameTooLarge {
                    size: len as usize,
                    max: wire::MAX_STREAM_FRAME as usize,
                });
            }

            let mut bytes = vec![0u8; len as usize];
            self.0
                .read_exact(&mut bytes)
                .await
                .map_err(|err| MeshError::Transport(err.to_string()))?;
            wire::decode::<MeshStreamFrame>(&bytes).map(Some)
        })
    }
}

impl MeshStream {
    pub(crate) fn new(send: Box<dyn StreamSendHalf>, recv: Box<dyn StreamRecvHalf>) -> Self {
        Self { send, recv }
    }
}
