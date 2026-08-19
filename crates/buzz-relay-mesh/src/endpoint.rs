use std::net::SocketAddr;

use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMode, SecretKey, TransportAddr};

use crate::{MeshError, RuntimeId, ALPN};

/// Local iroh endpoint for the relay mesh.
///
/// Identity is the iroh/ed25519 public key of a boot-unique keypair generated
/// at process start.
#[derive(Debug, Clone)]
pub struct MeshEndpoint {
    endpoint: Endpoint,
    runtime_id: RuntimeId,
}

impl MeshEndpoint {
    /// Generate a boot-unique mesh keypair and bind a mesh endpoint on `bind_addr`.
    pub async fn bind(bind_addr: SocketAddr) -> Result<Self, MeshError> {
        Self::bind_with_secret_key(SecretKey::generate(), bind_addr).await
    }

    /// Bind with an explicit keypair. Production should use [`Self::bind`] so
    /// every process boot gets a fresh RuntimeId; tests use this for stable
    /// identities.
    pub async fn bind_with_secret_key(
        secret_key: SecretKey,
        bind_addr: SocketAddr,
    ) -> Result<Self, MeshError> {
        let runtime_id = runtime_id_from_public_key(secret_key.public());
        let endpoint = Endpoint::builder(iroh::endpoint::presets::Minimal)
            .secret_key(secret_key)
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind_addr(bind_addr)
            .map_err(|err| MeshError::Transport(err.to_string()))?
            .bind()
            .await
            .map_err(|err| MeshError::Transport(err.to_string()))?;

        Ok(Self {
            endpoint,
            runtime_id,
        })
    }

    pub fn runtime_id(&self) -> RuntimeId {
        self.runtime_id
    }

    pub fn endpoint(&self) -> Endpoint {
        self.endpoint.clone()
    }

    pub fn addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// The endpoint's directly-dialable IP socket addrs (no relay paths).
    /// Lets consumers build advertise records without depending on iroh types.
    pub fn ip_addrs(&self) -> Vec<SocketAddr> {
        self.endpoint
            .addr()
            .addrs
            .iter()
            .filter_map(|ta| match ta {
                TransportAddr::Ip(sock) => Some(*sock),
                _ => None,
            })
            .collect()
    }

    pub async fn accept(&self) -> Result<Option<crate::peer::MeshPeer>, MeshError> {
        let Some(incoming) = self.endpoint.accept().await else {
            return Ok(None);
        };
        let conn = incoming
            .await
            .map_err(|err| MeshError::Transport(err.to_string()))?;
        crate::peer::MeshPeer::from_connection(self.endpoint.clone(), conn).map(Some)
    }

    pub async fn connect(
        &self,
        peer_addr: EndpointAddr,
    ) -> Result<crate::peer::MeshPeer, MeshError> {
        let conn = self
            .endpoint
            .connect(peer_addr, ALPN)
            .await
            .map_err(|err| MeshError::Transport(err.to_string()))?;
        crate::peer::MeshPeer::from_connection(self.endpoint.clone(), conn)
    }
}

pub fn runtime_id_from_public_key(public_key: PublicKey) -> RuntimeId {
    RuntimeId(*public_key.as_bytes())
}

pub fn public_key_from_runtime_id(runtime_id: RuntimeId) -> Result<PublicKey, MeshError> {
    PublicKey::from_bytes(&runtime_id.0).map_err(|err| MeshError::Transport(err.to_string()))
}

pub fn direct_addr(runtime_id: RuntimeId, addr: SocketAddr) -> Result<EndpointAddr, MeshError> {
    Ok(EndpointAddr::from_parts(
        public_key_from_runtime_id(runtime_id)?,
        [TransportAddr::Ip(addr)],
    ))
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::Duration;

    use iroh::SecretKey;
    use tokio::time::timeout;
    use uuid::Uuid;

    use super::MeshEndpoint;

    use crate::{
        wire, FencedHeader, GoodbyeReason, MeshDatagram, MeshError, MeshStreamFrame, Profile,
        RuntimeId, StreamHello, StreamRole,
    };

    fn loopback_any() -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
    }

    fn fenced(owner_runtime_id: RuntimeId) -> FencedHeader {
        FencedHeader {
            session_id: Uuid::from_u128(0xABCD),
            generation: 7,
            owner_runtime_id,
        }
    }

    async fn endpoint_pair() -> (MeshEndpoint, MeshEndpoint) {
        let a =
            MeshEndpoint::bind_with_secret_key(SecretKey::from_bytes(&[1u8; 32]), loopback_any())
                .await
                .unwrap();
        let b =
            MeshEndpoint::bind_with_secret_key(SecretKey::from_bytes(&[2u8; 32]), loopback_any())
                .await
                .unwrap();
        (a, b)
    }

    async fn connected_pair() -> (
        crate::peer::MeshPeer,
        crate::peer::MeshPeer,
        RuntimeId,
        RuntimeId,
    ) {
        let (a, b) = endpoint_pair().await;
        let a_runtime_id = a.runtime_id();
        let b_runtime_id = b.runtime_id();

        let b_addr = b.addr();
        let accept = tokio::spawn(async move { b.accept().await.unwrap().unwrap() });
        let a_peer = a.connect(b_addr).await.unwrap();
        let b_peer = accept.await.unwrap();

        (a_peer, b_peer, a_runtime_id, b_runtime_id)
    }

    #[tokio::test]
    async fn two_endpoints_connect_with_alpn_and_authenticated_identity() {
        let (a_peer, b_peer, a_runtime_id, b_runtime_id) = connected_pair().await;

        assert_eq!(a_peer.runtime_id(), b_runtime_id);
        assert_eq!(b_peer.runtime_id(), a_runtime_id);
        assert!(a_peer.max_datagram_size().expect("datagrams enabled") > 0);
    }

    #[tokio::test]
    async fn reliable_stream_roundtrip_carries_mesh_stream_frame() {
        let (a_peer, b_peer, _a_runtime_id, b_runtime_id) = connected_pair().await;
        let fenced = fenced(b_runtime_id);
        let hello = MeshStreamFrame::Hello(StreamHello {
            sender: RuntimeId([9u8; 32]),
            role: StreamRole::Session {
                fenced,
                profile: Profile::ReliableStream,
            },
        });
        let data = MeshStreamFrame::Data {
            fenced,
            payload: b"goose bytes".to_vec(),
        };
        let goodbye = MeshStreamFrame::Goodbye {
            fenced,
            reason: GoodbyeReason::SessionEnded,
        };

        let recv = tokio::spawn(async move {
            let mut stream = b_peer.accept_bi().await.unwrap();
            let first = stream.recv_frame().await.unwrap().unwrap();
            let second = stream.recv_frame().await.unwrap().unwrap();
            let third = stream.recv_frame().await.unwrap().unwrap();
            (first, second, third)
        });

        let mut stream = a_peer.open_bi().await.unwrap();
        stream.send_frame(hello.clone()).await.unwrap();
        stream.send_frame(data.clone()).await.unwrap();
        stream.send_frame(goodbye.clone()).await.unwrap();
        stream.finish().unwrap();

        let (got_hello, got_data, got_goodbye) = timeout(Duration::from_secs(5), recv)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got_hello, hello);
        assert_eq!(got_data, data);
        assert_eq!(got_goodbye, goodbye);
    }

    #[tokio::test]
    async fn datagram_roundtrip_carries_mesh_datagram() {
        let (a_peer, b_peer, _a_runtime_id, b_runtime_id) = connected_pair().await;
        let dgram = MeshDatagram {
            fenced: fenced(b_runtime_id),
            seq: 1,
            payload: vec![13, 37, 42],
        };

        a_peer.send_datagram(&dgram).unwrap();
        let got = timeout(Duration::from_secs(5), b_peer.recv_datagram())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, dgram);
    }

    #[tokio::test]
    async fn oversized_datagram_is_rejected_before_send() {
        let (a_peer, _b_peer, _a_runtime_id, b_runtime_id) = connected_pair().await;
        let max = a_peer.max_datagram_size().expect("datagrams enabled");
        let dgram = MeshDatagram {
            fenced: fenced(b_runtime_id),
            seq: 1,
            payload: vec![0u8; max + 1],
        };

        let err = a_peer.send_datagram(&dgram).unwrap_err();
        assert!(matches!(
            err,
            MeshError::DatagramTooLarge { size, max: limit } if size > limit
        ));
    }

    #[tokio::test]
    async fn opus_sized_datagrams_clear_empirical_local_loss_gate() {
        let (a_peer, b_peer, _a_runtime_id, b_runtime_id) = connected_pair().await;
        let payload_len = 1 /* Dawn huddle peer_index */ + 8 /* v2 audio header */ + 160;
        let encoded_len = wire::encode(&MeshDatagram {
            fenced: fenced(b_runtime_id),
            seq: 0,
            payload: vec![0u8; payload_len],
        })
        .unwrap()
        .len();
        assert!(encoded_len <= a_peer.max_datagram_size().expect("datagrams enabled"));

        let count = 64u64;
        for seq in 0..count {
            a_peer
                .send_datagram(&MeshDatagram {
                    fenced: fenced(b_runtime_id),
                    seq,
                    payload: vec![seq as u8; payload_len],
                })
                .unwrap();
            tokio::task::yield_now().await;
        }

        let mut got = Vec::new();
        for _ in 0..count {
            got.push(
                timeout(Duration::from_secs(5), b_peer.recv_datagram())
                    .await
                    .unwrap()
                    .unwrap()
                    .seq,
            );
        }
        got.sort_unstable();
        assert_eq!(got, (0..count).collect::<Vec<_>>());
    }
}
