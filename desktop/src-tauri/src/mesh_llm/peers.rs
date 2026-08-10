use serde::{Deserialize, Serialize};

/// A node this machine's mesh runtime is **actually talking to right now**.
///
/// ## Why this exists instead of reusing the relay snapshot
///
/// Buzz has two views of the mesh and they answer different questions:
///
/// - **Relay status notes** (`snapshot.rs`) — every member's last published
///   note, valid for 120s. Readable with no local node, so it is the only way
///   to see the pool *before* joining. But a note outlives the node that wrote
///   it: a machine that vanished 30s ago still looks present, and still counts
///   toward capacity.
/// - **Live peers** (this file) — the runtime's own gossip view. A node appears
///   here because our node is connected to it. No freshness window to guess at,
///   no reconciliation rule to invent.
///
/// A topology view has to be trustworthy, so it draws *this*. Mixing the two
/// sources meant refereeing disagreements between them, which produced a graph
/// that showed devices gossip already knew were gone.
///
/// The tradeoff, stated plainly: with no local node there are no peers, and the
/// relay snapshot remains the only available view.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeshPeer {
    /// Short node id from the runtime. Stable within a session.
    pub id: String,
    /// Human label — hostname as the peer reports it.
    pub label: String,
    /// Coarse role/state, normalized. See [`MeshPeerState`].
    pub state: MeshPeerState,
    /// Shared AI memory in GB, honoring the peer's own `--max-vram` cap.
    /// `None` when the peer reports nothing (a client-mode node reports 0, which
    /// is normalized to `None` rather than displayed as "0 GB").
    pub capacity_gb: Option<f64>,
    /// Models this peer is serving right now. Empty for a consuming peer.
    pub models: Vec<String>,
    /// Round-trip latency in ms, when measured.
    pub rtt_ms: Option<u64>,
}

/// What a peer is doing, as *it* reports — never inferred from traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeshPeerState {
    /// Advertising at least one model.
    Serving,
    /// Warming up — a host that has not advertised a model yet.
    Loading,
    /// Present and able to host, but advertising nothing.
    Standby,
    /// Client-mode: on the mesh to consume, not to contribute.
    Consuming,
}

/// This machine's live view of the mesh.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MeshLiveView {
    /// True when a local runtime is up. Distinguishes "no peers" (alone on the
    /// mesh) from "unknown" (not participating, so nothing to report).
    pub connected: bool,
    /// This machine's own shared capacity, when serving.
    pub self_capacity_gb: Option<f64>,
    /// Size of the model currently hosted on this machine, when reported.
    pub self_model_size_gb: Option<f64>,
    /// Peers currently connected, sharing first then by label.
    pub peers: Vec<MeshPeer>,
}

fn f64_at(value: &serde_json::Value, key: &str) -> Option<f64> {
    value.get(key).and_then(serde_json::Value::as_f64)
}

/// Normalize a peer's reported role/state into [`MeshPeerState`].
///
/// Trusts the peer's own `state`/`role` first and only falls back to inferring
/// from advertised models. A client-mode node that happens to list a model is
/// still consuming: role wins over model presence.
fn peer_state(value: &serde_json::Value, has_models: bool) -> MeshPeerState {
    let raw = value
        .get("state")
        .or_else(|| value.get("role"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match raw.as_str() {
        "client" => MeshPeerState::Consuming,
        "serving" => MeshPeerState::Serving,
        "loading" => MeshPeerState::Loading,
        "standby" => MeshPeerState::Standby,
        _ if has_models => MeshPeerState::Serving,
        _ => MeshPeerState::Standby,
    }
}

fn string_list(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Pure extractor: project a raw SDK status payload into [`MeshLiveView`].
///
/// Defensive throughout (missing → absent) so an SDK shape change degrades to
/// "no peers shown" rather than an error.
pub fn live_view_from_payload(payload: &serde_json::Value) -> MeshLiveView {
    let mut peers = payload
        .get("peers")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|peer| {
                    let id = peer
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    if id.is_empty() {
                        return None;
                    }
                    let mut models = string_list(peer, "serving_models");
                    if models.is_empty() {
                        models = string_list(peer, "hosted_models");
                    }
                    models.sort();
                    models.dedup();
                    let label = peer
                        .get("hostname")
                        .and_then(serde_json::Value::as_str)
                        .filter(|hostname| !hostname.is_empty())
                        .map(str::to_string)
                        // Fall back to a short id rather than "Unknown": the id
                        // is at least actionable in logs.
                        .unwrap_or_else(|| id.chars().take(10).collect());
                    Some(MeshPeer {
                        state: peer_state(peer, !models.is_empty()),
                        // 0 GB means "did not report" (client mode), not "has no
                        // memory" — keep it absent so the UI omits the figure.
                        capacity_gb: f64_at(peer, "vram_gb").filter(|gb| *gb > 0.0),
                        models,
                        rtt_ms: peer
                            .get("rtt_ms")
                            .and_then(serde_json::Value::as_u64)
                            .or_else(|| peer.get("latency_ms").and_then(serde_json::Value::as_u64)),
                        id,
                        label,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    peers.sort_by(|a, b| {
        let serving = |peer: &MeshPeer| peer.state == MeshPeerState::Serving;
        serving(b)
            .cmp(&serving(a))
            .then_with(|| a.label.cmp(&b.label))
    });

    MeshLiveView {
        connected: true,
        self_capacity_gb: f64_at(payload, "my_vram_gb").filter(|gb| *gb > 0.0),
        self_model_size_gb: f64_at(payload, "model_size_gb").filter(|gb| *gb > 0.0),
        peers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(peers: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "my_vram_gb": 115.0,
            "model_size_gb": 17.0,
            "peers": peers,
        })
    }

    #[test]
    fn projects_a_serving_peer_with_its_reported_figures() {
        let view = live_view_from_payload(&payload(serde_json::json!([{
            "id": "abc123def456",
            "hostname": "studio54.lan",
            "state": "serving",
            "vram_gb": 64.0,
            "rtt_ms": 30,
            "serving_models": ["gemma-4"],
        }])));
        assert!(view.connected);
        assert_eq!(view.self_capacity_gb, Some(115.0));
        assert_eq!(view.self_model_size_gb, Some(17.0));
        assert_eq!(view.peers.len(), 1);
        let peer = &view.peers[0];
        assert_eq!(peer.label, "studio54.lan");
        assert_eq!(peer.state, MeshPeerState::Serving);
        assert_eq!(peer.capacity_gb, Some(64.0));
        assert_eq!(peer.rtt_ms, Some(30));
        assert_eq!(peer.models, vec!["gemma-4".to_string()]);
    }

    #[test]
    fn a_client_peer_reports_no_capacity_rather_than_zero_gb() {
        // Client-mode nodes report vram_gb: 0. Showing "0 GB" would imply a
        // machine with no memory instead of one that shares none.
        let view = live_view_from_payload(&payload(serde_json::json!([{
            "id": "cli1",
            "hostname": "mac.lan",
            "state": "client",
            "vram_gb": 0.0,
        }])));
        assert_eq!(view.peers[0].state, MeshPeerState::Consuming);
        assert_eq!(view.peers[0].capacity_gb, None);
    }

    #[test]
    fn a_declared_role_wins_over_advertised_models() {
        // A client that lists a model is still consuming.
        let view = live_view_from_payload(&payload(serde_json::json!([{
            "id": "cli2",
            "hostname": "host.lan",
            "state": "client",
            "serving_models": ["gemma-4"],
        }])));
        assert_eq!(view.peers[0].state, MeshPeerState::Consuming);
    }

    #[test]
    fn serving_peers_sort_before_consumers() {
        let view = live_view_from_payload(&payload(serde_json::json!([
            { "id": "b", "hostname": "zulu.lan", "state": "client" },
            { "id": "a", "hostname": "alpha.lan", "state": "serving", "serving_models": ["m"] },
        ])));
        assert_eq!(view.peers[0].label, "alpha.lan");
        assert_eq!(view.peers[1].label, "zulu.lan");
    }

    #[test]
    fn peers_without_an_id_are_dropped() {
        let view = live_view_from_payload(&payload(serde_json::json!([
            { "hostname": "nameless.lan", "state": "serving" },
        ])));
        assert!(view.peers.is_empty());
    }

    #[test]
    fn a_missing_peers_array_is_alone_not_broken() {
        let view = live_view_from_payload(&serde_json::json!({ "my_vram_gb": 8.0 }));
        assert!(view.connected);
        assert!(view.peers.is_empty());
    }

    #[test]
    fn an_unlabelled_peer_falls_back_to_a_short_id() {
        let view = live_view_from_payload(&payload(serde_json::json!([
            { "id": "0123456789abcdef", "state": "standby" },
        ])));
        assert_eq!(view.peers[0].label, "0123456789");
    }
}
