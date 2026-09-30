use serde::{Deserialize, Serialize};

/// Host-side "who is using the compute I'm sharing" snapshot.
///
/// Read-only projection of the serving node's own runtime metrics (the same
/// `routing_metrics` / `inflight_requests` the SDK already exposes on the local
/// console). No new trust surface: it reads the node's own status payload.
///
/// The local/remote/endpoint attempt split is what distinguishes *my own*
/// agent (local) from *another member consuming my compute* (remote/endpoint).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MeshServingUsage {
    /// Requests being served right now.
    pub inflight: u64,
    /// Highest concurrent in-flight seen this session.
    pub peak_inflight: u64,
    /// Total requests routed through this node.
    pub requests_served: u64,
    /// Completion tokens produced.
    pub tokens_served: u64,
    /// Recent decode throughput.
    pub tokens_per_second: f64,
    /// Requests served for this machine's own agents.
    pub local_attempts: u64,
    /// Requests served for a remote peer (someone else consuming my compute).
    pub remote_attempts: u64,
    /// Requests served via an advertised endpoint (also a remote consumer).
    pub endpoint_attempts: u64,
    /// Other nodes currently visible as peers.
    pub peers: u64,
}

/// Pure extractor: project a raw SDK status payload into [`MeshServingUsage`].
///
/// Every field is read defensively (missing → 0) so an SDK shape change
/// degrades to "no usage shown" rather than an error. Kept pure so it can be
/// unit-tested against a captured payload without a live runtime.
pub fn serving_usage_from_payload(payload: &serde_json::Value) -> MeshServingUsage {
    let u64_at = |v: &serde_json::Value| v.as_u64().unwrap_or(0);
    let rm = payload.get("routing_metrics");
    let local = rm.and_then(|m| m.get("local_node"));
    let get_u64 = |obj: Option<&serde_json::Value>, key: &str| {
        obj.and_then(|o| o.get(key)).map(u64_at).unwrap_or(0)
    };
    MeshServingUsage {
        inflight: local
            .and_then(|l| l.get("current_inflight_requests"))
            .map(u64_at)
            .or_else(|| payload.get("inflight_requests").map(u64_at))
            .unwrap_or(0),
        peak_inflight: get_u64(local, "peak_inflight_requests"),
        requests_served: get_u64(rm, "request_count"),
        tokens_served: get_u64(rm, "completion_tokens_observed"),
        tokens_per_second: rm
            .and_then(|m| m.get("avg_tokens_per_second"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        local_attempts: get_u64(local, "local_attempt_count"),
        remote_attempts: get_u64(local, "remote_attempt_count"),
        endpoint_attempts: get_u64(local, "endpoint_attempt_count"),
        peers: payload
            .get("peers")
            .and_then(serde_json::Value::as_array)
            .map(|a| a.len() as u64)
            .unwrap_or(0),
    }
}
